//! Human terminal control surface. No chat, LLM, or capsule-owned UI dependency.
mod model;
mod native;
mod transport;
mod updates_view;
mod view;

use std::io::{self, IsTerminal};
use std::net::Shutdown;
use std::os::unix::net::UnixStream;
use std::process::ExitCode;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
    mpsc,
};
use std::time::{Duration, Instant};

use clap::Args;
use crossterm::{
    event::{
        self, DisableBracketedPaste, DisableMouseCapture, EnableBracketedPaste, EnableMouseCapture,
        Event as TerminalEvent, KeyCode, KeyEventKind, KeyModifiers, MouseButton, MouseEventKind,
    },
    execute,
};
use ratatui::layout::Rect;
use zeroize::{Zeroize, Zeroizing};

use model::{CAPACITY, Destination, Kind, MAX_VALUE, Request};

#[derive(Args)]
pub(crate) struct ConsoleArgs {
    /// Preview the interface with clearly marked, in-memory sample requests.
    #[arg(long)]
    preview: bool,
}

enum Event {
    Updates(Result<crate::updates::Inventory, String>),
    Request(Request),
    Disconnected(u64),
    Delivered { id: String, delivered: bool },
}

struct App {
    updates_open: bool,
    updates_busy: bool,
    updates_confirm: bool,
    updates: Option<crate::updates::Inventory>,
    updates_notice: String,
    updates_scroll: u16,
    updates_sender: Option<mpsc::SyncSender<Event>>,
    requests: Vec<Request>,
    selected: usize,
    action: usize,
    input: Zeroizing<String>,
    notice: String,
    private_status: String,
    preview: bool,
    rows: Vec<Rect>,
    buttons: Vec<Rect>,
    scroll: u16,
    show_help: bool,
    detail_area: Rect,
    awaiting: std::collections::HashSet<String>,
}

impl App {
    fn new(preview: bool) -> Self {
        Self {
            updates_open: false,
            updates_busy: false,
            updates_confirm: false,
            updates: None,
            updates_notice: String::new(),
            updates_scroll: 0,
            updates_sender: None,
            requests: if preview {
                model::preview()
            } else {
                Vec::new()
            },
            selected: 0,
            action: 0,
            input: Zeroizing::new(String::new()),
            notice: String::new(),
            private_status: if preview {
                "Preview · sample requests, no runtime connection"
            } else {
                "Not configured · run aos native-setup to pair private input"
            }
            .into(),
            preview,
            rows: Vec::new(),
            buttons: Vec::new(),
            scroll: 0,
            show_help: false,
            detail_area: Rect::default(),
            awaiting: Default::default(),
        }
    }
    fn select(&mut self, index: usize) {
        self.notice.clear();
        self.selected = index.min(self.requests.len().saturating_sub(1));
        self.action = 0;
        self.scroll = 0;
        self.input.zeroize();
    }
    fn finish(&mut self, choice: Option<usize>) {
        let Some(request) = self.requests.get_mut(self.selected) else {
            return;
        };
        if Instant::now() >= request.deadline {
            self.expire();
            return;
        }
        match request.send(choice, &self.input) {
            Ok(()) => {
                if matches!(request.destination, Destination::Private { .. }) {
                    self.awaiting.insert(request.id.clone());
                    self.notice = "Sent · waiting for the runtime to acknowledge delivery".into();
                } else {
                    self.notice = if self.preview {
                        "Preview only · no runtime was changed"
                    } else {
                        "Response sent to the requesting connection"
                    }
                    .into();
                }
                self.requests.remove(self.selected);
                let notice = std::mem::take(&mut self.notice);
                self.select(self.selected);
                if self.requests.is_empty() {
                    self.notice = notice;
                }
            }
            Err(_) => {
                self.notice =
                    "Response failed. Check the connection or input; no success was recorded."
                        .into();
                self.input.zeroize();
            }
        }
    }
    fn expire(&mut self) {
        let now = Instant::now();
        if self.requests.iter().any(|r| r.deadline <= now) {
            self.requests.retain(|r| r.deadline > now);
            self.select(self.selected);
            self.notice = "A request expired. No approval was sent.".into();
        }
    }
    fn key(&mut self, code: KeyCode, modifiers: KeyModifiers) -> bool {
        if code == KeyCode::Char('c') && modifiers.contains(KeyModifiers::CONTROL) {
            if self.updates_busy {
                self.updates_notice =
                    "An update operation is running. Wait for its result before closing.".into();
                return true;
            }
            return false;
        }
        if code == KeyCode::F(2) {
            self.updates_open = !self.updates_open;
            self.updates_confirm = false;
            if self.updates_open && self.updates.is_none() {
                self.update_operation(0);
            }
            return true;
        }
        if self.updates_open {
            match code {
                KeyCode::Down => self.updates_scroll = self.updates_scroll.saturating_add(1),
                KeyCode::Up => self.updates_scroll = self.updates_scroll.saturating_sub(1),
                KeyCode::PageDown => self.updates_scroll = self.updates_scroll.saturating_add(8),
                KeyCode::PageUp => self.updates_scroll = self.updates_scroll.saturating_sub(8),
                KeyCode::Char('r') => self.update_operation(1),
                KeyCode::Char('u') if !self.updates_busy => {
                    self.updates_confirm = self
                        .updates
                        .as_ref()
                        .is_some_and(|v| v.items.iter().any(|i| i.action == "apply"));
                }
                KeyCode::Enter if self.updates_confirm => {
                    self.updates_confirm = false;
                    self.update_operation(2);
                }
                KeyCode::Esc => self.updates_confirm = false,
                _ => {}
            }
            return true;
        }
        match code {
            KeyCode::F(1) => self.show_help = !self.show_help,
            KeyCode::Esc => self.finish(None),
            KeyCode::Tab => {
                if let Some(r) = self.requests.get(self.selected) {
                    self.action = (self.action + 1) % (r.options().len() + 1);
                }
            }
            KeyCode::BackTab => {
                if let Some(r) = self.requests.get(self.selected) {
                    let n = r.options().len() + 1;
                    self.action = (self.action + n - 1) % n;
                }
            }
            KeyCode::Up => self.select(self.selected.saturating_sub(1)),
            KeyCode::Down => self.select(self.selected + 1),
            KeyCode::PageDown => self.scroll = self.scroll.saturating_add(3),
            KeyCode::PageUp => self.scroll = self.scroll.saturating_sub(3),
            KeyCode::Enter => {
                if modifiers.contains(KeyModifiers::ALT)
                    && self
                        .requests
                        .get(self.selected)
                        .is_some_and(|r| matches!(r.kind, Kind::Array))
                {
                    if self.input.len() < MAX_VALUE {
                        self.input.push('\n');
                    }
                } else {
                    self.finish(self.action.checked_sub(1));
                }
            }
            KeyCode::Backspace => {
                self.input.pop();
            }
            KeyCode::Char(c)
                if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.append(&c.to_string())
            }
            _ => {}
        }
        true
    }

    fn update_operation(&mut self, operation: u8) {
        if self.updates_busy {
            return;
        }
        if self.preview {
            self.updates_notice = "Preview only · no update operation was executed".into();
            return;
        }
        let Some(sender) = self.updates_sender.clone() else {
            return;
        };
        self.updates_busy = true;
        self.updates_notice = match operation {
            2 => "Installing reviewed updates…",
            1 => "Checking authenticated metadata…",
            _ => "Reading cached updates…",
        }
        .into();
        std::thread::spawn(move || {
            let result = match operation {
                2 => crate::updates::apply("all"),
                1 => crate::updates::check(None),
                _ => crate::updates::list(),
            };
            let _ = sender.send(Event::Updates(result));
        });
    }
    fn wheel(&mut self, column: u16, row: u16, down: bool) {
        if self.detail_area.contains((column, row).into()) {
            self.scroll = if down {
                self.scroll.saturating_add(3)
            } else {
                self.scroll.saturating_sub(3)
            };
        } else if self
            .rows
            .iter()
            .any(|rect| rect.contains((column, row).into()))
        {
            self.select(if down {
                self.selected + 1
            } else {
                self.selected.saturating_sub(1)
            });
        }
    }
    fn append(&mut self, text: &str) {
        if self
            .requests
            .get(self.selected)
            .is_some_and(|r| matches!(r.kind, Kind::Text | Kind::Secret | Kind::Array))
        {
            for c in text.chars().filter(|c| !c.is_control()) {
                if self.input.len() + c.len_utf8() > MAX_VALUE {
                    break;
                }
                self.input.push(c);
            }
        }
    }
}

struct Backend {
    stop: Arc<AtomicBool>,
    listener: Option<std::thread::JoinHandle<()>>,
    private: Option<UnixStream>,
}
impl Drop for Backend {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(stream) = &self.private {
            let _ = stream.shutdown(Shutdown::Both);
        }
        if let Some(thread) = self.listener.take() {
            let _ = thread.join();
        }
    }
}

pub(crate) fn run(args: ConsoleArgs) -> ExitCode {
    match run_inner(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("aos console: {error}");
            ExitCode::FAILURE
        }
    }
}

fn console_home(
    home: unicity_aos_bootstrap::AosHome,
) -> io::Result<unicity_aos_bootstrap::AosHome> {
    // Resolve the operator-selected home once, as pairing does. Credential
    // components beneath it still use the strict no-follow reader.
    Ok(unicity_aos_bootstrap::AosHome::from_root(
        crate::native_setup::canonical_setup_home(home.root())
            .map_err(|_| io::Error::other("AOS home cannot be resolved"))?,
    ))
}

fn run_inner(args: ConsoleArgs) -> io::Result<()> {
    if !io::stdin().is_terminal() || !io::stdout().is_terminal() {
        return Err(io::Error::other(
            "open this command in an interactive terminal",
        ));
    }
    let mut app = App::new(args.preview);
    let (tx, rx) = mpsc::sync_channel(CAPACITY);
    app.updates_sender = Some(tx.clone());
    let mut backend = Backend {
        stop: Arc::new(AtomicBool::new(false)),
        listener: None,
        private: None,
    };
    if !args.preview {
        let home = crate::resolve_home().map_err(|_| io::Error::other("AOS home unavailable"))?;
        let home = console_home(home)?;
        let listener = transport::Listener::bind(home.root())?;
        let stop = backend.stop.clone();
        let sender = tx.clone();
        backend.listener = Some(std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                match listener.socket.accept() {
                    Ok((stream, _)) => {
                        if let Ok(request) = transport::approval(stream) {
                            let _ = sender.try_send(Event::Request(request));
                        }
                    }
                    Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(50))
                    }
                    Err(_) => break,
                }
            }
        }));
        let path = crate::native_setup::connection_path(&home);
        if path.exists() {
            match native::connect(&path, tx.clone(), 1) {
                Ok(stream) => {
                    backend.private = Some(stream);
                    app.private_status = "Connected · authenticated private input".into();
                }
                Err(_) => {
                    app.private_status =
                        "Disconnected · check native-input setup and runtime status".into()
                }
            }
        }
    }
    let mut terminal = ratatui::init();
    struct Restore;
    impl Drop for Restore {
        fn drop(&mut self) {
            let _ = execute!(io::stdout(), DisableMouseCapture, DisableBracketedPaste);
            ratatui::restore();
        }
    }
    let _restore = Restore;
    execute!(io::stdout(), EnableMouseCapture, EnableBracketedPaste)?;
    loop {
        while let Ok(event) = rx.try_recv() {
            match event {
                Event::Updates(result) => {
                    app.updates_busy = false;
                    match result {
                        Ok(inventory) => {
                            app.updates = Some(inventory);
                            app.updates_notice.clear();
                        }
                        Err(message) => app.updates_notice = message,
                    }
                }
                Event::Request(request) => {
                    if app.requests.len() < CAPACITY
                        && !app.requests.iter().any(|r| r.id == request.id)
                    {
                        if app.requests.is_empty() {
                            app.notice.clear();
                        }
                        app.requests.push(request);
                    }
                }
                Event::Disconnected(generation) => {
                    app.requests.retain(|r|!matches!(r.destination,Destination::Private{generation:g,..} if g==generation));
                    app.awaiting.clear();
                    app.select(app.selected);
                    app.private_status = "Disconnected · pending private input cancelled".into();
                }
                Event::Delivered { id, delivered } => {
                    if app.awaiting.remove(&id) {
                        app.notice=if delivered {"Previous private response: runtime acknowledged delivery."}else{"Previous private response: runtime refused delivery. No answer was replayed."}.into();
                    }
                }
            }
        }
        app.expire();
        terminal.draw(|frame| view::draw(frame, &mut app))?;
        if event::poll(Duration::from_millis(100))? {
            match event::read()? {
                TerminalEvent::Key(key)
                    if key.kind == KeyEventKind::Press && !app.key(key.code, key.modifiers) =>
                {
                    break;
                }
                TerminalEvent::Paste(text) if !app.updates_open => app.append(&text),
                TerminalEvent::Mouse(mouse)
                    if matches!(
                        mouse.kind,
                        MouseEventKind::ScrollDown | MouseEventKind::ScrollUp
                    ) =>
                {
                    app.wheel(
                        mouse.column,
                        mouse.row,
                        mouse.kind == MouseEventKind::ScrollDown,
                    );
                }
                TerminalEvent::Mouse(mouse)
                    if mouse.kind == MouseEventKind::Down(MouseButton::Left) =>
                {
                    let pos = (mouse.column, mouse.row).into();
                    if app.updates_open {
                        if let Some(i) = app.buttons.iter().position(|r| r.contains(pos)) {
                            if i == 0 {
                                app.update_operation(1);
                            } else if app.updates_confirm {
                                app.updates_confirm = false;
                                app.update_operation(2);
                            } else {
                                app.updates_confirm = app
                                    .updates
                                    .as_ref()
                                    .is_some_and(|v| v.items.iter().any(|i| i.action == "apply"));
                            }
                        }
                        continue;
                    }
                    if let Some(i) = app.rows.iter().position(|r| r.contains(pos)) {
                        app.select(i);
                    } else if let Some(i) = app.buttons.iter().position(|r| r.contains(pos)) {
                        app.action = i;
                        app.finish(i.checked_sub(1));
                    }
                }
                _ => {}
            }
        }
    }
    for mut request in app.requests.drain(..) {
        let _ = request.send(None, "");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn private_connection_resolves_home_alias_but_rejects_credential_symlinks() {
        use std::os::unix::fs::{PermissionsExt, symlink};
        use unicity_aos_bootstrap::AosHome;

        let root = std::env::temp_dir().join(format!("aos-console-alias-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir(&root).unwrap();
        let root = root.canonicalize().unwrap();
        let real = root.join("real");
        std::fs::create_dir_all(real.join("native-input")).unwrap();
        let alias = root.join("alias");
        symlink(&real, &alias).unwrap();
        let credential = real.join("native-input/connection.json");
        std::fs::write(&credential, b"{}").unwrap();
        std::fs::set_permissions(&credential, std::fs::Permissions::from_mode(0o600)).unwrap();
        let home = console_home(AosHome::from_root(alias)).unwrap();
        let path = crate::native_setup::connection_path(&home);
        assert_eq!(transport::private_read(&path, 1024).unwrap(), b"{}");

        std::fs::remove_file(&credential).unwrap();
        let target = root.join("other.json");
        std::fs::write(&target, b"{}").unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o600)).unwrap();
        symlink(&target, &credential).unwrap();
        assert!(transport::private_read(&path, 1024).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
    #[test]
    fn wheel_scrolls_without_submitting_a_request() {
        let mut app = App::new(true);
        app.detail_area = Rect::new(30, 5, 60, 15);
        app.wheel(40, 10, true);
        assert_eq!(app.scroll, 3);
        app.wheel(40, 10, false);
        assert_eq!(app.scroll, 0);
        assert_eq!(app.requests.len(), 2);
        assert_eq!(app.action, 0);
    }
    #[test]
    fn enter_defaults_to_cancel_not_approval() {
        let mut app = App::new(true);
        assert_eq!(app.action, 0);
        app.key(KeyCode::Enter, KeyModifiers::NONE);
        assert_eq!(app.requests.len(), 1);
        assert!(
            app.notice.is_empty(),
            "a new request must not inherit success"
        );
    }
    #[test]
    fn switching_requests_clears_request_specific_feedback() {
        let mut app = App::new(true);
        app.notice = "Response failed".into();
        app.select(1);
        assert!(app.notice.is_empty());
    }
    #[test]
    fn changing_request_clears_private_input() {
        let mut app = App::new(true);
        app.select(1);
        app.append("private");
        app.select(0);
        assert!(app.input.is_empty());
    }
    #[test]
    fn pasted_control_sequences_cannot_submit_or_overflow() {
        let mut app = App::new(true);
        app.select(1);
        app.append(&"x".repeat(MAX_VALUE + 10));
        app.append("\r\n");
        assert_eq!(app.input.len(), MAX_VALUE);
        assert_eq!(app.requests.len(), 2);
    }
}
