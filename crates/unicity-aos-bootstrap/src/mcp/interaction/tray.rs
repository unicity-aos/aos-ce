//! Same-user Unix socket client for the AOS tray presenter.
//!
//! An explicit `--interaction-socket` never falls back to AppKit, pinentry, or
//! the MCP host. Invalid frames, mismatched ids, out-of-range selections, EOF,
//! and an unavailable endpoint all surface as `Unavailable` so the caller can
//! cancel without synthesizing consent.

use std::fs::{self, Metadata};
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::os::fd::AsRawFd as _;
use std::os::unix::fs::{FileTypeExt as _, MetadataExt as _};
use std::os::unix::net::UnixStream;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::Deserialize;
use serde::Serialize;
use serde_json::Value;

use super::{
    InteractionError, InteractionRequest, MAX_INTERACTION_TIMEOUT_SECONDS, MAX_MESSAGE_BYTES,
    MIN_INTERACTION_TIMEOUT_SECONDS, Presenter,
};

const PROTOCOL_VERSION: u32 = 1;
const MAX_FRAME_BYTES: usize = 16384;
const MAX_ID_BYTES: usize = 128;
const MIN_ID_BYTES: usize = 1;
const MAX_OPTIONS: usize = 4;
const MIN_OPTIONS: usize = 1;
const _: () = assert!(
    super::DEFAULT_INTERACTION_TIMEOUT_SECONDS >= MIN_INTERACTION_TIMEOUT_SECONDS
        && super::DEFAULT_INTERACTION_TIMEOUT_SECONDS <= MAX_INTERACTION_TIMEOUT_SECONDS
);
const CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const WRITE_TIMEOUT: Duration = Duration::from_secs(5);
const SOCKET_MODE: u32 = 0o600;
const PARENT_MODE: u32 = 0o700;

pub(in crate::mcp) struct TrayPresenter {
    socket: PathBuf,
    timeout_seconds: u32,
}

impl TrayPresenter {
    pub(in crate::mcp) fn new(socket: PathBuf, timeout_seconds: u32) -> Self {
        Self {
            socket,
            timeout_seconds,
        }
    }
}

impl Presenter for TrayPresenter {
    fn present(&mut self, request: &InteractionRequest) -> Result<Option<usize>, InteractionError> {
        present_on_socket(&self.socket, request, self.timeout_seconds)
    }
}

fn prompt_read_timeout(timeout_seconds: u32) -> Result<Duration, InteractionError> {
    if !(MIN_INTERACTION_TIMEOUT_SECONDS..=MAX_INTERACTION_TIMEOUT_SECONDS)
        .contains(&timeout_seconds)
    {
        return Err(unavailable("tray prompt timeout is out of range"));
    }
    Ok(Duration::from_secs(u64::from(timeout_seconds)))
}

fn present_on_socket(
    socket: &Path,
    request: &InteractionRequest,
    timeout_seconds: u32,
) -> Result<Option<usize>, InteractionError> {
    let read_timeout = prompt_read_timeout(timeout_seconds)?;
    if !(MIN_OPTIONS..=MAX_OPTIONS).contains(&request.options.len()) {
        return Err(unavailable("tray request has an invalid number of options"));
    }
    if request.message.is_empty() || request.message.len() > MAX_MESSAGE_BYTES {
        return Err(unavailable("tray request message is empty or too large"));
    }
    validate_socket_path(socket)?;
    let stream = connect_unix(socket, read_timeout)?;
    let peer = peer_uid(&stream)?;
    if peer != current_uid() {
        return Err(unavailable("tray socket peer is not the current user"));
    }

    let id = correlation_id()?;
    let payload = SocketRequest {
        version: PROTOCOL_VERSION,
        id: id.clone(),
        message: &request.message,
        options: request
            .options
            .iter()
            .map(|option| SocketOption {
                label: option.label.as_str(),
            })
            .collect(),
        timeout_seconds,
        consent: request.consent.as_ref(),
    };
    let mut stream = stream;
    write_json_frame(&mut stream, &payload)?;
    let response = read_json_frame(&stream)?;
    if response.version != PROTOCOL_VERSION {
        return Err(unavailable("tray response version is unsupported"));
    }
    if response.id != id {
        return Err(unavailable("tray response id does not match the request"));
    }
    selected_index(&response.selected, request.options.len())
}

#[derive(Serialize)]
struct SocketRequest<'a> {
    version: u32,
    id: String,
    message: &'a str,
    options: Vec<SocketOption<'a>>,
    #[serde(rename = "timeoutSeconds")]
    timeout_seconds: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    consent: Option<&'a super::consent::ConsentPresentation>,
}

#[derive(Serialize)]
struct SocketOption<'a> {
    label: &'a str,
}

#[derive(Deserialize)]
struct SocketResponse {
    version: u32,
    id: String,
    selected: Value,
}

fn correlation_id() -> Result<String, InteractionError> {
    let id = uuid::Uuid::new_v4().to_string();
    if (MIN_ID_BYTES..=MAX_ID_BYTES).contains(&id.len()) {
        Ok(id)
    } else {
        Err(unavailable("failed to allocate a tray correlation id"))
    }
}

fn selected_index(value: &Value, option_count: usize) -> Result<Option<usize>, InteractionError> {
    match value {
        Value::Null => Ok(None),
        Value::Number(number) => {
            let Some(index) = number.as_u64() else {
                return Err(unavailable("tray selected index is out of range"));
            };
            usize::try_from(index)
                .ok()
                .filter(|index| *index < option_count)
                .map(Some)
                .ok_or_else(|| unavailable("tray selected index is out of range"))
        }
        _ => Err(unavailable("tray selected value is malformed")),
    }
}

fn validate_socket_path(path: &Path) -> Result<(), InteractionError> {
    if path.as_os_str().is_empty() {
        return Err(unavailable("interaction socket path is empty"));
    }
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| unavailable("interaction socket has no parent directory"))?;

    reject_symlink_components(path)?;

    let uid = current_uid();
    let parent_meta = fs::symlink_metadata(parent).map_err(|error| {
        unavailable(format!(
            "failed to inspect interaction socket directory: {error}"
        ))
    })?;
    if !parent_meta.is_dir() {
        return Err(unavailable("interaction socket parent is not a directory"));
    }
    if parent_meta.uid() != uid || permission_bits(&parent_meta) != PARENT_MODE {
        return Err(unavailable(
            "interaction socket directory must be owned by the current user with mode 0700",
        ));
    }

    let socket_meta = fs::symlink_metadata(path)
        .map_err(|error| unavailable(format!("failed to inspect interaction socket: {error}")))?;
    if !socket_meta.file_type().is_socket() {
        return Err(unavailable("interaction path is not a unix socket"));
    }
    if socket_meta.uid() != uid || permission_bits(&socket_meta) != SOCKET_MODE {
        return Err(unavailable(
            "interaction socket must be owned by the current user with mode 0600",
        ));
    }
    Ok(())
}

fn reject_symlink_components(path: &Path) -> Result<(), InteractionError> {
    let mut current = PathBuf::new();
    for component in path.components() {
        match component {
            Component::Prefix(prefix) => current.push(prefix.as_os_str()),
            Component::RootDir => current.push(Component::RootDir.as_os_str()),
            Component::CurDir => continue,
            Component::ParentDir => current.push(".."),
            Component::Normal(name) => current.push(name),
        }
        let metadata = fs::symlink_metadata(&current).map_err(|error| {
            unavailable(format!("failed to inspect {}: {error}", current.display()))
        })?;
        if metadata.file_type().is_symlink() {
            return Err(unavailable(format!(
                "interaction socket path component is a symlink: {}",
                current.display()
            )));
        }
    }
    Ok(())
}

fn permission_bits(metadata: &Metadata) -> u32 {
    metadata.mode() & 0o777
}

fn connect_unix(path: &Path, read_timeout: Duration) -> Result<UnixStream, InteractionError> {
    let path = path.to_owned();
    let (sender, receiver) = std::sync::mpsc::channel();
    std::thread::Builder::new()
        .name("aos-tray-connect".into())
        .spawn(move || {
            let _ = sender.send(UnixStream::connect(path));
        })
        .map_err(|error| unavailable(format!("failed to start tray connect: {error}")))?;
    match receiver.recv_timeout(CONNECT_TIMEOUT) {
        Ok(Ok(stream)) => {
            stream
                .set_read_timeout(Some(read_timeout))
                .map_err(|error| unavailable(format!("failed to bound tray read: {error}")))?;
            stream
                .set_write_timeout(Some(WRITE_TIMEOUT))
                .map_err(|error| unavailable(format!("failed to bound tray write: {error}")))?;
            Ok(stream)
        }
        Ok(Err(error)) => Err(unavailable(format!(
            "failed to connect to tray socket: {error}"
        ))),
        Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {
            Err(unavailable("timed out connecting to tray socket"))
        }
        Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
            Err(unavailable("tray connect worker ended without a result"))
        }
    }
}

fn write_json_frame(
    stream: &mut UnixStream,
    request: &SocketRequest<'_>,
) -> Result<(), InteractionError> {
    let mut frame =
        serde_json::to_vec(request).map_err(|_| unavailable("failed to encode tray request"))?;
    if frame.len() + 1 > MAX_FRAME_BYTES {
        return Err(unavailable("tray request frame is too large"));
    }
    frame.push(b'\n');
    stream
        .write_all(&frame)
        .and_then(|()| stream.flush())
        .map_err(|error| unavailable(format!("failed to write tray request: {error}")))
}

fn read_json_frame(stream: &UnixStream) -> Result<SocketResponse, InteractionError> {
    let mut reader = BufReader::new(stream);
    let mut frame = Vec::new();
    let mut limited = (&mut reader).take(MAX_FRAME_BYTES as u64 + 1);
    limited.read_until(b'\n', &mut frame).map_err(|error| {
        if error.kind() == std::io::ErrorKind::TimedOut {
            unavailable("timed out waiting for tray response")
        } else {
            unavailable(format!("failed to read tray response: {error}"))
        }
    })?;
    if frame.is_empty() {
        return Err(unavailable("tray closed without a response"));
    }
    if frame.len() > MAX_FRAME_BYTES || !frame.ends_with(b"\n") {
        return Err(unavailable("tray response frame is invalid or too large"));
    }
    frame.pop();
    if frame.last() == Some(&b'\r') {
        frame.pop();
    }
    serde_json::from_slice(&frame)
        .map_err(|_| unavailable("tray response is not a valid protocol object"))
}

pub(crate) fn local_peer_uid(stream: &UnixStream) -> std::io::Result<u32> {
    peer_uid(stream).map_err(|_| std::io::Error::other("could not verify local peer"))
}

fn peer_uid(stream: &UnixStream) -> Result<u32, InteractionError> {
    peer_uid_from_fd(stream.as_raw_fd())
}

#[cfg(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "openbsd",
    target_os = "netbsd"
))]
fn peer_uid_from_fd(fd: i32) -> Result<u32, InteractionError> {
    let mut euid = 0_u32;
    let mut egid = 0_u32;
    let result = unsafe {
        unsafe extern "C" {
            fn getpeereid(socket: i32, euid: *mut u32, egid: *mut u32) -> i32;
        }
        getpeereid(fd, &mut euid, &mut egid)
    };
    if result != 0 {
        return Err(unavailable(format!(
            "failed to read tray socket peer: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(euid)
}

#[cfg(target_os = "linux")]
fn peer_uid_from_fd(fd: i32) -> Result<u32, InteractionError> {
    #[repr(C)]
    struct Ucred {
        pid: i32,
        uid: u32,
        gid: u32,
    }
    const SOL_SOCKET: i32 = 1;
    const SO_PEERCRED: i32 = 17;
    let mut cred = Ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut length = std::mem::size_of::<Ucred>() as u32;
    let result = unsafe {
        unsafe extern "C" {
            fn getsockopt(
                socket: i32,
                level: i32,
                name: i32,
                value: *mut u8,
                length: *mut u32,
            ) -> i32;
        }
        getsockopt(
            fd,
            SOL_SOCKET,
            SO_PEERCRED,
            (&raw mut cred).cast(),
            &mut length,
        )
    };
    if result != 0 {
        return Err(unavailable(format!(
            "failed to read tray socket peer: {}",
            std::io::Error::last_os_error()
        )));
    }
    Ok(cred.uid)
}

#[cfg(not(any(
    target_os = "macos",
    target_os = "ios",
    target_os = "freebsd",
    target_os = "dragonfly",
    target_os = "openbsd",
    target_os = "netbsd",
    target_os = "linux"
)))]
fn peer_uid_from_fd(_fd: i32) -> Result<u32, InteractionError> {
    Err(unavailable(
        "no same-user peer credential API on this platform",
    ))
}

fn current_uid() -> u32 {
    unsafe {
        unsafe extern "C" {
            fn getuid() -> u32;
        }
        getuid()
    }
}

fn unavailable(message: impl Into<String>) -> InteractionError {
    InteractionError::Unavailable(message.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::os::unix::fs::PermissionsExt as _;
    use std::os::unix::net::UnixListener;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::mpsc;
    use std::thread;
    use std::time::Duration;

    static FIXTURE_SEQ: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
        socket: PathBuf,
    }

    impl Fixture {
        fn new(name: &str) -> Self {
            let base = if cfg!(target_os = "macos") {
                PathBuf::from("/private/tmp")
            } else {
                std::env::temp_dir()
            };
            let root = base.join(format!(
                "aos-tray-{name}-{}-{}",
                std::process::id(),
                FIXTURE_SEQ.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir_all(&root).expect("create tray fixture");
            set_mode(&root, PARENT_MODE);
            let socket = root.join("tray.sock");
            Self { root, socket }
        }
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.root);
        }
    }

    fn set_mode(path: &Path, mode: u32) {
        let mut permissions = fs::metadata(path).expect("metadata").permissions();
        permissions.set_mode(mode);
        fs::set_permissions(path, permissions).expect("chmod");
    }

    fn grant_request() -> InteractionRequest {
        super::super::parse_request(&json!({
            "jsonrpc": "2.0",
            "id": 17,
            "method": "elicitation/create",
            "params": {
                "mode": "form",
                "message": "Allow this capsule to continue?",
                "requestedSchema": {
                    "type": "object",
                    "properties": { "grant": { "type": "boolean" } },
                    "required": ["grant"]
                }
            }
        }))
        .expect("grant request")
    }

    fn present_with<F>(respond: F) -> Result<Option<usize>, InteractionError>
    where
        F: FnOnce(Value) -> Option<String> + Send + 'static,
    {
        present_with_timeout(super::super::DEFAULT_INTERACTION_TIMEOUT_SECONDS, respond)
    }

    fn present_with_timeout<F>(
        timeout_seconds: u32,
        respond: F,
    ) -> Result<Option<usize>, InteractionError>
    where
        F: FnOnce(Value) -> Option<String> + Send + 'static,
    {
        let fixture = Fixture::new("server");
        let listener = UnixListener::bind(&fixture.socket).expect("bind tray");
        set_mode(&fixture.socket, SOCKET_MODE);
        let (started, ready) = mpsc::sync_channel(1);
        let handle = thread::spawn(move || {
            let _ = started.send(());
            let (mut stream, _) = listener.accept().expect("accept");
            let mut line = String::new();
            BufReader::new(&stream)
                .read_line(&mut line)
                .expect("read request");
            let request: Value = serde_json::from_str(line.trim_end()).unwrap_or(json!({}));
            if let Some(response) = respond(request) {
                let mut bytes = response.into_bytes();
                if !bytes.ends_with(b"\n") {
                    bytes.push(b'\n');
                }
                let _ = stream.write_all(&bytes);
                let _ = stream.flush();
            }
        });
        ready.recv().expect("server listening");
        let mut presenter = TrayPresenter::new(fixture.socket.clone(), timeout_seconds);
        let result = presenter.present(&grant_request());
        let _ = UnixStream::connect(&fixture.socket);
        let _ = handle.join();
        result
    }

    fn echo_selected(request: &Value, selected: Value) -> String {
        json!({
            "version": 1,
            "id": request["id"],
            "selected": selected
        })
        .to_string()
    }

    #[test]
    fn tray_accept_returns_selected_index() {
        let selected = present_with(|request| {
            assert_eq!(request["version"], 1);
            assert_eq!(
                request["timeoutSeconds"],
                json!(super::super::DEFAULT_INTERACTION_TIMEOUT_SECONDS)
            );
            assert_eq!(request["message"], "Allow this capsule to continue?");
            assert_eq!(
                request["options"],
                json!([{ "label": "Grant" }, { "label": "Deny" }])
            );
            let id = request["id"].as_str().expect("id");
            assert!((MIN_ID_BYTES..=MAX_ID_BYTES).contains(&id.len()));
            Some(echo_selected(&request, json!(0)))
        })
        .expect("accept");
        assert_eq!(selected, Some(0));
    }

    #[test]
    fn tray_deny_returns_selected_index() {
        let selected =
            present_with(|request| Some(echo_selected(&request, json!(1)))).expect("deny");
        assert_eq!(selected, Some(1));
    }

    #[test]
    fn tray_cancel_returns_no_selection() {
        let selected =
            present_with(|request| Some(echo_selected(&request, Value::Null))).expect("cancel");
        assert_eq!(selected, None);
    }

    #[test]
    fn tray_id_mismatch_is_unavailable() {
        let error = present_with(|request| {
            Some(
                json!({
                    "version": 1,
                    "id": format!("{}-other", request["id"].as_str().unwrap_or("missing")),
                    "selected": 0
                })
                .to_string(),
            )
        })
        .expect_err("id mismatch");
        assert!(matches!(error, InteractionError::Unavailable(_)));
    }

    #[test]
    fn tray_malformed_response_is_unavailable() {
        let error = present_with(|_| Some("not-json".to_owned())).expect_err("malformed");
        assert!(matches!(error, InteractionError::Unavailable(_)));

        let error = present_with(|request| {
            Some(
                json!({
                    "version": 2,
                    "id": request["id"],
                    "selected": 0
                })
                .to_string(),
            )
        })
        .expect_err("version");
        assert!(matches!(error, InteractionError::Unavailable(_)));

        let error = present_with(|request| {
            Some(
                json!({
                    "version": 1,
                    "id": request["id"],
                    "selected": true
                })
                .to_string(),
            )
        })
        .expect_err("selected type");
        assert!(matches!(error, InteractionError::Unavailable(_)));
    }

    #[test]
    fn tray_out_of_range_selected_never_consents() {
        let error = present_with(|request| Some(echo_selected(&request, json!(99))))
            .expect_err("out of range");
        assert!(matches!(error, InteractionError::Unavailable(_)));
    }

    #[test]
    fn tray_unavailable_socket_never_consents() {
        let fixture = Fixture::new("missing");
        let mut presenter = TrayPresenter::new(
            fixture.socket.clone(),
            super::super::DEFAULT_INTERACTION_TIMEOUT_SECONDS,
        );
        let error = presenter
            .present(&grant_request())
            .expect_err("missing socket");
        assert!(matches!(error, InteractionError::Unavailable(_)));
    }

    #[test]
    fn tray_eof_without_response_is_unavailable() {
        let error = present_with(|_| None).expect_err("eof");
        assert!(matches!(error, InteractionError::Unavailable(_)));
    }

    #[test]
    fn configured_prompt_deadlines_are_serialized_without_waiting() {
        for timeout in [1_u32, 180] {
            let selected = present_with_timeout(timeout, move |request| {
                assert_eq!(request["timeoutSeconds"], json!(timeout));
                Some(echo_selected(&request, json!(0)))
            })
            .expect("configured timeout");
            assert_eq!(selected, Some(0));
        }
    }

    #[test]
    fn prompt_read_timeout_matches_configured_seconds() {
        assert_eq!(
            prompt_read_timeout(1).expect("short"),
            Duration::from_secs(1)
        );
        assert_eq!(
            prompt_read_timeout(180).expect("long"),
            Duration::from_secs(180)
        );
        assert!(prompt_read_timeout(0).is_err());
        assert!(prompt_read_timeout(301).is_err());
    }

    #[test]
    fn out_of_range_prompt_timeout_never_consents() {
        let fixture = Fixture::new("bad-timeout");
        let mut presenter = TrayPresenter::new(fixture.socket.clone(), 0);
        let error = presenter
            .present(&grant_request())
            .expect_err("zero timeout");
        assert!(matches!(error, InteractionError::Unavailable(_)));
    }
}
