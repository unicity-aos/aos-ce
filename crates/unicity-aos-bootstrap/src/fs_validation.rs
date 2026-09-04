//! Private-directory and regular-file validation helpers.

use std::fs;
use std::io;
use std::path::Path;

pub(crate) fn create_private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    validate_private_dir(path)
}

pub(crate) fn validate_private_dir(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "AOS managed path must be a real directory: {}",
                path.display()
            ),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o777 != 0o700 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("AOS managed directory must be private: {}", path.display()),
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_regular_file(path: &Path, executable: bool) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "release inventory entry must be a regular file: {}",
                path.display()
            ),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let expected_mode = if executable { 0o700 } else { 0o600 };
        if metadata.permissions().mode() & 0o777 != expected_mode {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "release inventory entry has unexpected permissions: {}",
                    path.display()
                ),
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_activation_file(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "active AOS launcher must be a regular file: {}",
                path.display()
            ),
        ));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o100 == 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("active AOS launcher is not executable: {}", path.display()),
            ));
        }
    }
    Ok(())
}
