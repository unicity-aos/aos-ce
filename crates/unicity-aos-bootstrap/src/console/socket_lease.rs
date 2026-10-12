//! A crash releases the lock, but leaves enough identity to reclaim our socket.
use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::Path;

use fs2::FileExt;
use rustix::fs::{Mode, OFlags, open};

pub(super) struct SocketLease(File);

impl SocketLease {
    pub fn acquire(directory: &Path) -> io::Result<Self> {
        let path = directory.join("approval.lock");
        let file = File::from(open(
            &path,
            OFlags::RDWR | OFlags::CREATE | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::NONBLOCK,
            Mode::from_bits_truncate(0o600),
        )?);
        let meta = file.metadata()?;
        let named = fs::symlink_metadata(&path)?;
        if !meta.is_file()
            || meta.uid() != rustix::process::getuid().as_raw()
            || meta.mode() & 0o777 != 0o600
            || meta.nlink() != 1
            || (meta.dev(), meta.ino()) != (named.dev(), named.ino())
        {
            return Err(io::Error::other(
                "console lock must be a private user-owned regular file",
            ));
        }
        file.try_lock_exclusive().map_err(|error| {
            if error.kind() == io::ErrorKind::WouldBlock {
                io::Error::new(error.kind(), "another AOS console is already open")
            } else {
                error
            }
        })?;
        Ok(Self(file))
    }

    pub fn recover(&mut self, path: &Path) -> io::Result<()> {
        let meta = match fs::symlink_metadata(path) {
            Ok(meta) => meta,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(error),
        };
        self.0.seek(SeekFrom::Start(0))?;
        let mut bytes = Vec::new();
        // Two decimal u64 identifiers fit in 64 bytes. Larger records are corrupt.
        (&self.0).take(65).read_to_end(&mut bytes)?;
        let identity: (u64, u64) = serde_json::from_slice(&bytes).map_err(|_| {
            io::Error::other("existing console socket has no recoverable ownership record")
        })?;
        if !meta.file_type().is_socket()
            || meta.uid() != rustix::process::getuid().as_raw()
            || meta.mode() & 0o777 != 0o600
            || identity != (meta.dev(), meta.ino())
        {
            return Err(io::Error::other(
                "existing console socket does not match its ownership record",
            ));
        }
        match UnixStream::connect(path) {
            Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {}
            _ => {
                return Err(io::Error::other(
                    "existing console socket is not confirmed stopped",
                ));
            }
        }
        let named = fs::symlink_metadata(path)?;
        if (named.dev(), named.ino()) != identity {
            return Err(io::Error::other("console socket changed during recovery"));
        }
        // The held lease serializes recovery and bind. Unattributed legacy sockets
        // are deliberately preserved rather than guessing they belong to us.
        fs::remove_file(path)
    }

    pub fn record(&self, device: u64, inode: u64) -> io::Result<()> {
        let mut file = &self.0;
        file.seek(SeekFrom::Start(0))?;
        file.set_len(0)?;
        file.write_all(&serde_json::to_vec(&(device, inode))?)?;
        file.sync_all()
    }
}

#[cfg(test)]
mod tests;
