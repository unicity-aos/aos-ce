//! Mounted CLI trust projection and active-apply receipt for Unicity CE.

use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::json;

use crate::AosHome;
use crate::fs_validation::{create_private_dir, validate_private_dir};

pub(crate) const SELECTED_DISTRO_ID: &str = "unicity-ce";

#[derive(Debug, Deserialize)]
struct SelectedDistro {
    distro: SelectedDistroMetadata,
}

#[derive(Debug, Deserialize)]
struct SelectedDistroMetadata {
    id: String,
    version: String,
    signing: Option<SelectedDistroSigning>,
}

#[derive(Debug, Deserialize)]
struct SelectedDistroSigning {
    pubkey: String,
}

#[derive(Debug, Deserialize)]
struct ActiveApplyReceipt {
    schema: u32,
    kind: String,
    active: bool,
    cutover_complete: bool,
    distro_id: String,
    distro_version: String,
    pin_blake3: String,
}

/// Read the operator-facing signing key declared by the selected release.
pub fn selected_signing_key(manifest: &Path) -> io::Result<String> {
    let manifest = selected_distro(manifest)?;
    let key = manifest
        .distro
        .signing
        .map(|signing| signing.pubkey)
        .filter(|key| !key.is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                "selected distribution has no signing key",
            )
        })?;
    Ok(key)
}

/// Seed the exact selected key onto Astrid's mounted trust projection.
///
/// A matching pin is left untouched. A foreign or redirected pin is retained
/// and fails closed before the caller dispatches the signed apply.
pub fn seed_selected_pin(home: &AosHome, manifest: &Path) -> io::Result<String> {
    let key = selected_signing_key(manifest)?;
    let trust_dir = home.runtime_home().join("trust");
    let pin = trust_dir.join(format!("{SELECTED_DISTRO_ID}.pub"));
    match fs::symlink_metadata(&trust_dir) {
        Ok(_) => validate_private_dir(&trust_dir)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => create_private_dir(&trust_dir)?,
        Err(error) => return Err(error),
    }

    match fs::symlink_metadata(&pin) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                format!(
                    "distribution trust pin must be a regular file: {}",
                    pin.display()
                ),
            ));
        }
        Ok(_) => {
            let existing = fs::read_to_string(&pin)?;
            if existing.trim() == key {
                return Ok(key);
            }
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!(
                    "distribution trust pin differs from the selected release: {}",
                    pin.display()
                ),
            ));
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    let temporary = pin.with_extension("pub.aos-next");
    write_private_atomic(&temporary, format!("{key}\n").as_bytes())?;
    fs::rename(&temporary, &pin)?;
    Ok(key)
}

/// The AOS-owned durable receipt proving a successful stop-safe cutover.
pub(crate) fn active_receipt_path(home: &AosHome) -> PathBuf {
    home.root()
        .join("receipts")
        .join(format!("{SELECTED_DISTRO_ID}.active.json"))
}

pub fn write_active_receipt(
    home: &AosHome,
    manifest: &Path,
    principal: &str,
    key: &str,
) -> io::Result<()> {
    let receipt = active_receipt_path(home);
    create_private_dir(receipt.parent().expect("active receipt has a parent"))?;
    let temporary = receipt.with_extension("json.aos-next");
    let value = json!({
        "schema": 1,
        "kind": "aos-distro-apply-active-v1",
        "active": true,
        "cutover_complete": true,
        "distro_id": SELECTED_DISTRO_ID,
        "distro_version": selected_distro(manifest)?.distro.version,
        "principal": principal,
        "pin_blake3": blake3::hash(key.as_bytes()).to_string(),
    });
    write_private_atomic(&temporary, &serde_json::to_vec_pretty(&value)?)?;
    fs::rename(&temporary, &receipt)?;
    Ok(())
}

pub(crate) fn validate_active_receipt(home: &AosHome, manifest: &Path) -> io::Result<()> {
    let path = active_receipt_path(home);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("active distribution receipt is missing: {}", path.display()),
            ));
        }
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "active distribution receipt is redirected or not a regular file: {}",
                path.display()
            ),
        ));
    }
    let receipt: ActiveApplyReceipt =
        serde_json::from_slice(&fs::read(&path)?).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidData,
                format!("active distribution receipt is invalid: {error}"),
            )
        })?;
    let expected_key = selected_signing_key(manifest)?;
    if receipt.schema != 1
        || receipt.kind != "aos-distro-apply-active-v1"
        || !receipt.active
        || !receipt.cutover_complete
        || receipt.distro_id != SELECTED_DISTRO_ID
        || receipt.distro_version != selected_distro(manifest)?.distro.version
        || receipt.pin_blake3 != blake3::hash(expected_key.as_bytes()).to_string()
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "active distribution receipt does not match this release: {}",
                path.display()
            ),
        ));
    }
    Ok(())
}

fn selected_distro(manifest: &Path) -> io::Result<SelectedDistro> {
    let text = fs::read_to_string(manifest).map_err(|error| invalid_manifest(manifest, error))?;
    let manifest: SelectedDistro =
        toml::from_str(&text).map_err(|error| invalid_manifest(manifest, error))?;
    if manifest.distro.id != SELECTED_DISTRO_ID {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "selected distribution is not Unicity CE",
        ));
    }
    Ok(manifest)
}

fn write_private_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            format!(
                "AOS staging path is redirected or not a regular file: {}",
                path.display()
            ),
        ));
    }
    let result = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .and_then(|mut file| file.write_all(bytes).and_then(|()| file.sync_all()));
    if let Err(error) = result {
        let _ = fs::remove_file(path);
        return Err(error);
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }
    Ok(())
}

fn invalid_manifest(path: &Path, error: impl std::fmt::Display) -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        format!("invalid selected distribution {}: {error}", path.display()),
    )
}

#[cfg(all(test, unix))]
mod tests {
    use std::os::unix::fs::PermissionsExt;

    use super::*;

    const TEST_MANIFEST: &str = r#"
[distro]
id = "unicity-ce"
version = "test"

[distro.signing]
pubkey = "test-key"
"#;

    struct TempRoot(PathBuf);

    impl TempRoot {
        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempRoot {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixture(name: &str) -> (TempRoot, AosHome, PathBuf) {
        let root_path = std::env::temp_dir().join(format!(
            "unicity-aos-distro-trust-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root_path).expect("create trust fixture root");
        let root = TempRoot(root_path);
        let home = AosHome::from_root(root.0.join("home"));
        let manifest = root.path().join("Distro.toml");
        fs::write(&manifest, TEST_MANIFEST).expect("write manifest");
        std::fs::create_dir_all(home.runtime_home()).expect("create runtime projection");
        (root, home, manifest)
    }

    fn pin_path(home: &AosHome) -> PathBuf {
        home.runtime_home()
            .join("trust")
            .join(format!("{SELECTED_DISTRO_ID}.pub"))
    }

    #[test]
    fn missing_pin_is_created_as_a_private_regular_file() {
        let (_root, home, manifest) = fixture("missing");
        let key = seed_selected_pin(&home, &manifest).expect("seed missing pin");
        let path = pin_path(&home);
        let metadata = fs::symlink_metadata(&path).expect("pin metadata");
        assert!(metadata.is_file());
        assert_eq!(metadata.permissions().mode() & 0o777, 0o600);
        assert_eq!(fs::read_to_string(&path).unwrap(), format!("{key}\n"));
    }

    #[test]
    fn matching_pin_is_not_rewritten() {
        let (_root, home, manifest) = fixture("matching");
        let path = pin_path(&home);
        fs::create_dir_all(path.parent().unwrap()).expect("create trust");
        fs::set_permissions(
            path.parent().unwrap(),
            std::fs::Permissions::from_mode(0o700),
        )
        .expect("make trust private");
        fs::write(&path, "test-key\n").expect("write matching pin");
        let before = fs::metadata(&path).unwrap().modified().unwrap();
        std::thread::sleep(std::time::Duration::from_millis(10));
        seed_selected_pin(&home, &manifest).expect("matching pin");
        let after = fs::metadata(&path).unwrap().modified().unwrap();
        assert_eq!(before, after);
    }

    #[test]
    fn foreign_pin_fails_without_replacement() {
        let (_root, home, manifest) = fixture("foreign");
        let path = pin_path(&home);
        fs::create_dir_all(path.parent().unwrap()).expect("create trust");
        fs::set_permissions(
            path.parent().unwrap(),
            std::fs::Permissions::from_mode(0o700),
        )
        .expect("make trust private");
        fs::write(&path, "foreign\n").expect("write foreign pin");
        let error = seed_selected_pin(&home, &manifest).expect_err("foreign pin");
        assert!(error.to_string().contains("differs"));
        assert_eq!(fs::read_to_string(&path).unwrap(), "foreign\n");
    }

    #[test]
    fn redirected_pin_fails_closed() {
        let (root, home, manifest) = fixture("redirect");
        let target = root.path().join("outside.pub");
        fs::write(&target, "foreign\n").expect("write target");
        let path = pin_path(&home);
        fs::create_dir_all(path.parent().unwrap()).expect("create trust");
        fs::set_permissions(
            path.parent().unwrap(),
            std::fs::Permissions::from_mode(0o700),
        )
        .expect("make trust private");
        std::os::unix::fs::symlink(&target, &path).expect("link pin");
        let error = seed_selected_pin(&home, &manifest).expect_err("redirected pin");
        assert!(error.to_string().contains("regular file"));
        assert!(path.is_symlink());
    }
}
