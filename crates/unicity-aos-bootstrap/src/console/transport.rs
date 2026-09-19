//! Existing tray presentation protocol, served by the terminal instead of AppKit.
use std::fs::{self, File};
use std::io::{self, BufRead, BufReader, Read};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use rustix::fs::{Mode, OFlags, open, openat};
use serde::Deserialize;
use serde_json::Value;

use super::model::{Destination, Kind, Request, display};

pub(super) const FRAME_LIMIT: usize = 16_384;

pub(super) fn private_read(path: &Path, maximum: usize) -> io::Result<Vec<u8>> {
    let text = path
        .to_str()
        .ok_or_else(|| io::Error::other("invalid credential path"))?;
    if !text.starts_with('/')
        || text[1..]
            .split('/')
            .any(|s| s.is_empty() || s == "." || s == "..")
    {
        return Err(io::Error::other(
            "credential path must be absolute without aliases",
        ));
    }
    let parts: Vec<_> = text[1..].split('/').collect();
    let mut fd = open(
        "/",
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    for (i, part) in parts.iter().enumerate() {
        let mut flags = OFlags::RDONLY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK;
        if i + 1 != parts.len() {
            flags |= OFlags::DIRECTORY;
        }
        fd = openat(&fd, *part, flags, Mode::empty())?;
    }
    let file = File::from(fd);
    let meta = file.metadata()?;
    if !meta.is_file()
        || meta.uid() != rustix::process::getuid().as_raw()
        || meta.mode() & 0o077 != 0
    {
        return Err(io::Error::other(
            "credential must be a private file owned by this user",
        ));
    }
    let mut bytes = Vec::new();
    file.take(maximum as u64 + 1).read_to_end(&mut bytes)?;
    if bytes.is_empty() || bytes.len() > maximum {
        return Err(io::Error::other("invalid credential size"));
    }
    Ok(bytes)
}

pub(super) fn same_user(stream: &UnixStream) -> io::Result<()> {
    let uid = crate::mcp::interaction::tray::local_peer_uid(stream)
        .map_err(|_| io::Error::other("could not verify local peer"))?;
    if uid != rustix::process::getuid().as_raw() {
        return Err(io::Error::other("peer is not this user"));
    }
    Ok(())
}

pub(super) struct Listener {
    pub socket: UnixListener,
    path: PathBuf,
    inode: u64,
    device: u64,
}
impl Listener {
    pub fn bind(home: &Path) -> io::Result<Self> {
        // Canonicalize the existing home once, including macOS /var aliases.
        let home = home.canonicalize()?;
        let directory = home.join("console");
        match fs::create_dir(&directory) {
            Ok(()) => fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))?,
            Err(e) if e.kind() == io::ErrorKind::AlreadyExists => {}
            Err(e) => return Err(e),
        }
        let meta = fs::symlink_metadata(&directory)?;
        if !meta.is_dir()
            || meta.uid() != rustix::process::getuid().as_raw()
            || meta.mode() & 0o777 != 0o700
        {
            return Err(io::Error::other(
                "console directory must be user-owned and mode 0700",
            ));
        }
        let path = directory.join("approval.sock");
        // Never unlink a socket belonging to another console, even if it looks stale.
        let socket = UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))?;
        let meta = fs::symlink_metadata(&path)?;
        socket.set_nonblocking(true)?;
        Ok(Self {
            socket,
            path,
            inode: meta.ino(),
            device: meta.dev(),
        })
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        if let Ok(meta) = fs::symlink_metadata(&self.path)
            && meta.ino() == self.inode
            && meta.dev() == self.device
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

#[derive(Deserialize)]
struct Prompt {
    version: u32,
    id: String,
    message: String,
    options: Vec<Choice>,
    #[serde(rename = "timeoutSeconds")]
    timeout: u64,
    consent: Option<Value>,
}
#[derive(Deserialize)]
struct Choice {
    label: String,
}

pub(super) fn approval(stream: UnixStream) -> io::Result<Request> {
    same_user(&stream)?;
    stream.set_read_timeout(Some(Duration::from_secs(1)))?;
    stream.set_write_timeout(Some(Duration::from_secs(1)))?;
    let mut frame = Vec::new();
    BufReader::new((&stream).take(FRAME_LIMIT as u64 + 1)).read_until(b'\n', &mut frame)?;
    if frame.len() > FRAME_LIMIT || !frame.ends_with(b"\n") {
        return Err(io::Error::other("invalid approval frame"));
    }
    let prompt: Prompt =
        serde_json::from_slice(&frame).map_err(|_| io::Error::other("invalid approval request"))?;
    if prompt.version != 1
        || prompt.id.is_empty()
        || prompt.id.len() > 128
        || prompt.message.is_empty()
        || prompt.options.is_empty()
        || prompt.options.len() > 4
        || !(1..=3600).contains(&prompt.timeout)
        || prompt
            .options
            .iter()
            .any(|o| o.label.is_empty() || o.label.len() > 256)
    {
        return Err(io::Error::other("invalid approval request"));
    }
    let consent = prompt.consent.unwrap_or(Value::Null);
    let text = |key: &str| consent[key].as_str().map(display).unwrap_or_default();
    // Keep the full explanation in the scrollable body. A one-line heading
    // must never silently discard the requested scope or its qualifications.
    let mut detail = prompt
        .message
        .lines()
        .map(display)
        .collect::<Vec<_>>()
        .join("\n");
    detail.push_str("\n\n");
    for (key, label) in [
        ("action", "Action"),
        ("resource", "Resource"),
        ("reason", "Reason"),
    ] {
        let value = text(key);
        if !value.is_empty() {
            detail.push_str(&format!("{label}: {value}\n"));
        }
    }
    let title = text("capsule");
    let action = text("action");
    Ok(Request {
        id: prompt.id,
        title: if title.is_empty() {
            "Requested by the connected agent".into()
        } else {
            title
        },
        principal: text("principal"),
        message: if action.is_empty() {
            "Allow this request?".into()
        } else {
            format!("Allow {action}?")
        },
        detail,
        kind: Kind::Approval(
            prompt
                .options
                .into_iter()
                .map(|o| display(&o.label))
                .collect(),
        ),
        deadline: Instant::now() + Duration::from_secs(prompt.timeout),
        destination: Destination::Approval(stream),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::io::Write;

    #[test]
    fn approval_preserves_full_multiline_explanation_in_scrollable_body() {
        let (reader, mut writer) = UnixStream::pair().unwrap();
        let explanation = format!(
            "{}\nOnly for this session.\nNo access to other principals.",
            "Long explanation. ".repeat(20)
        );
        writeln!(
            writer,
            "{}",
            json!({"version":1,"id":"long-description","message":explanation,
            "options":[{"label":"Allow"},{"label":"Deny"}],"timeoutSeconds":120})
        )
        .unwrap();
        let request = approval(reader).unwrap();
        assert_eq!(request.message, "Allow this request?");
        assert!(request.detail.starts_with(&explanation));
        assert!(request.detail.contains("\nNo access to other principals."));
        assert!(!request.detail.contains('\u{001b}'));
    }
}
