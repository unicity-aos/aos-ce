//! Compatibility repair for private roots created by pre-2026.9 AOS.
//!
//! AOS owns these roots, while Astrid Runtime owns the permission contract it
//! enforces before boot. Older AOS releases used the process umask and left
//! existing directories/files more permissive than Astrid accepts. This module
//! narrows the compatibility repair to the legacy roots that have been
//! observed in published layouts. It does not create missing state or touch
//! any other runtime path.

use std::fs;
use std::io;
use std::path::Path;

/// Tighten the known legacy AOS-owned roots before handing the home to Astrid.
///
/// `runtime/secrets`, `runtime/home`, and `runtime/cow` were all observed in
/// pre-2026.9 homes with permissive descendants. Current Astrid deliberately
/// rejects those paths before boot because mutable private state must be
/// owner-only. Existing symlinks and special entries fail closed; no link is
/// followed and no missing content is created.
pub(super) fn repair_legacy_private_roots(runtime_home: &Path) -> io::Result<()> {
    for name in ["secrets", "home", "cow"] {
        tighten_legacy_private_tree(&runtime_home.join(name))?;
    }
    Ok(())
}

fn tighten_legacy_private_tree(root: &Path) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(root) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid_root(root));
    }
    tighten_existing(root)
}

fn tighten_existing(path: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            format!(
                "AOS runtime private tree contains a symlink: {}",
                path.display()
            ),
        ));
    }

    if metadata.is_dir() {
        // The Astrid helpers reopen each path component with no-follow
        // semantics and apply the platform's private policy. The metadata
        // precheck above preserves the compatibility contract that absent
        // entries are not created; the helper closes symlink/type races.
        astrid_core::platform_fs::ensure_private_directory(path)?;
        for entry in fs::read_dir(path)? {
            tighten_existing(&entry?.path())?;
        }
        return Ok(());
    }

    if metadata.is_file() {
        // This is an fd-based no-follow operation on Unix and the equivalent
        // protected-handle operation on Windows. It also rejects a raced
        // symlink or special entry instead of chmod-ing an unexpected target.
        return astrid_core::platform_fs::restrict_private_file(path);
    }

    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "AOS runtime private tree contains a non-regular entry: {}",
            path.display()
        ),
    ))
}

fn invalid_root(path: &Path) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidInput,
        format!(
            "AOS runtime private root is not a real directory: {}",
            path.display()
        ),
    )
}

#[cfg(test)]
mod tests {
    use super::repair_legacy_private_roots;
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temporary_root(label: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "unicity-aos-{label}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ))
    }

    fn short_temporary_root() -> PathBuf {
        PathBuf::from(format!(
            "/tmp/aos-lp-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ))
    }

    #[cfg(unix)]
    fn mode(path: &Path) -> u32 {
        use std::os::unix::fs::PermissionsExt;

        fs::metadata(path).expect("metadata").permissions().mode() & 0o777
    }

    #[cfg(unix)]
    #[test]
    fn legacy_private_roots_are_tightened_before_runtime_boot() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = temporary_root("legacy-private");
        let mut files = Vec::new();
        let mut directories = Vec::new();
        for root_name in ["secrets", "home", "cow"] {
            let root = fixture.join(root_name);
            let default = root.join("default");
            let nested = default.join("provider");
            fs::create_dir_all(&nested).expect("create legacy tree");
            let file = nested.join(format!("{root_name}.log"));
            fs::write(&file, root_name.as_bytes()).expect("write legacy file");
            files.push((file, root_name.as_bytes().to_vec()));
            directories.extend([root, default, nested]);
        }
        fs::set_permissions(fixture.join("home"), fs::Permissions::from_mode(0o755))
            .expect("model legacy home mode");
        fs::set_permissions(fixture.join("secrets"), fs::Permissions::from_mode(0o755))
            .expect("model legacy secrets mode");
        fs::set_permissions(fixture.join("cow"), fs::Permissions::from_mode(0o755))
            .expect("model legacy cow mode");
        for directory in &directories {
            fs::set_permissions(directory, fs::Permissions::from_mode(0o755))
                .expect("model legacy nested directory mode");
        }
        for (file, _) in &files {
            fs::set_permissions(file, fs::Permissions::from_mode(0o644))
                .expect("model legacy file mode");
        }

        repair_legacy_private_roots(&fixture).expect("tighten legacy private roots");

        for directory in &directories {
            assert_eq!(mode(directory), 0o700, "{}", directory.display());
        }
        for (file, bytes) in &files {
            assert_eq!(mode(file), 0o600, "{}", file.display());
            assert_eq!(fs::read(file).expect("read repaired file"), *bytes);
        }
        fs::remove_dir_all(fixture).expect("remove legacy private fixture");
    }

    #[cfg(unix)]
    #[test]
    fn legacy_private_roots_reject_symlinks_and_special_entries() {
        use std::os::unix::fs::symlink;
        use std::os::unix::net::UnixListener;

        let fixture = temporary_root("legacy-symlink");
        let secrets = fixture.join("secrets");
        fs::create_dir_all(&secrets).expect("create secrets root");
        let outside = fixture.join("outside");
        fs::create_dir_all(&outside).expect("create outside directory");
        symlink(&outside, secrets.join("default")).expect("create secret symlink");

        let error =
            repair_legacy_private_roots(&fixture).expect_err("secret symlink must fail closed");
        assert!(error.to_string().contains("contains a symlink"));
        fs::remove_dir_all(fixture).expect("remove symlink fixture");

        // macOS caps Unix-domain socket paths at SUN_LEN; keep only this
        // disposable fixture's random directory under the short `/tmp` root.
        let fixture = short_temporary_root();
        let home = fixture.join("home");
        fs::create_dir_all(&home).expect("create home fixture");
        let socket_path = home.join("unexpected.sock");
        let listener = UnixListener::bind(&socket_path).expect("create special entry fixture");
        let error =
            repair_legacy_private_roots(&fixture).expect_err("special entries must fail closed");
        assert!(error.to_string().contains("non-regular entry"));
        drop(listener);
        fs::remove_dir_all(fixture).expect("remove special entry fixture");
    }
}
