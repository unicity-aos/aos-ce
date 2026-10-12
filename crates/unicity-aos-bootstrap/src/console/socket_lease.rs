//! The held lease serializes bind/recovery; a retained socket link proves identity.
use std::fs::{self, File};
use std::io;
use std::os::unix::fs::{FileTypeExt, MetadataExt};
use std::os::unix::net::UnixStream;
use std::path::Path;

use fs2::FileExt;
use rustix::fs::{Mode, OFlags, open};

pub(super) struct SocketLease {
    _file: File,
}

impl Drop for SocketLease {
    fn drop(&mut self) {
        // Explicit release also covers a descriptor temporarily inherited by a
        // concurrent fork before CLOEXEC closes it in the child.
        let _ = FileExt::unlock(&self._file);
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum BindStage {
    Bound,
    Configured,
    Published,
}

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
        Ok(Self { _file: file })
    }

    pub fn recover(&self, path: &Path) -> io::Result<()> {
        let owner = path.with_file_name("approval.owner.sock");
        let published = match fs::symlink_metadata(path) {
            Ok(meta) => Some(meta),
            Err(error) if error.kind() == io::ErrorKind::NotFound => None,
            Err(error) => return Err(error),
        };
        let meta = match fs::symlink_metadata(&owner) {
            Ok(meta) => meta,
            Err(error) if error.kind() == io::ErrorKind::NotFound && published.is_none() => {
                return Ok(());
            }
            Err(error) => return Err(error),
        };
        let identity = (meta.dev(), meta.ino());
        // A crash before chmod can leave an unpublished socket with its initial
        // mode. Its parent is already validated as private and user-owned.
        if !meta.file_type().is_socket()
            || meta.uid() != rustix::process::getuid().as_raw()
            || published.as_ref().is_some_and(|named| {
                !named.file_type().is_socket()
                    || named.mode() & 0o777 != 0o600
                    || identity != (named.dev(), named.ino())
            })
        {
            return Err(io::Error::other(
                "existing console socket does not match its retained ownership link",
            ));
        }
        match UnixStream::connect(&owner) {
            Err(error) if error.kind() == io::ErrorKind::ConnectionRefused => {}
            _ => {
                return Err(io::Error::other(
                    "existing console socket is not confirmed stopped",
                ));
            }
        }
        let named = fs::symlink_metadata(&owner)?;
        if (named.dev(), named.ino()) != identity {
            return Err(io::Error::other("console socket changed during recovery"));
        }
        if published.is_some() {
            let named = fs::symlink_metadata(path)?;
            if (named.dev(), named.ino()) != identity {
                return Err(io::Error::other("console socket changed during recovery"));
            }
            // Remove the public name first: interruption leaves an attributable
            // unpublished socket, never an ownerless public socket.
            fs::remove_file(path)?;
        }
        fs::remove_file(owner)
    }
}

#[cfg(test)]
mod tests;
