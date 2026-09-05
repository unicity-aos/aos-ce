//! Verification and receipt helpers for the bundled Unicity CE distro.
//!
//! The release directory is the package boundary for Distro Apply.  The
//! manifest, lock, and signature are consumed only from that selected path;
//! callers cannot substitute another manifest or opt into Astrid's unsigned
//! trust bypasses.

use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use astrid_crypto::{PublicKey, Signature};
use serde::{Deserialize, Serialize};
use unicity_aos_bootstrap::AosHome;

pub(crate) const SELECTED_DISTRO_ID: &str = "unicity-ce";
pub(crate) const ASTRID_RUNTIME_VERSION: &str = "0.10.4";
const SIG_DOMAIN_TAG: &[u8] = b"astrid-distro-lock-sig-v1\x00";
const RECEIPT_SCHEMA_VERSION: u32 = 1;
const RECEIPT_KIND: &str = "aos-distro-apply-active-v1";

#[derive(Debug, Clone)]
pub(crate) struct VerifiedDistro {
    pub(crate) manifest_path: PathBuf,
    pub(crate) distro_id: String,
    pub(crate) distro_version: String,
    pub(crate) manifest_blake3: String,
    pub(crate) signing_pubkey: String,
    pub(crate) lock_blake3: String,
    pub(crate) signature_blake3: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct DistroLock {
    schema_version: u32,
    distro: DistroLockMeta,
    #[serde(default, rename = "capsule")]
    capsules: Vec<LockedCapsule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    manifest_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct DistroLockMeta {
    id: String,
    version: String,
    resolved_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LockedCapsule {
    name: String,
    version: String,
    source: String,
    hash: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    resolved_ref: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ActiveReceipt {
    schema: u32,
    kind: String,
    active: bool,
    cutover_complete: bool,
    distro_id: String,
    distro_version: String,
    manifest_blake3: String,
    signing_pubkey: String,
    pin_blake3: String,
    lock_blake3: String,
    signature_blake3: String,
    principal: String,
    astrid_runtime_version: String,
}

/// Verify the exact release members used by the selected Distro Apply path.
pub(crate) fn verify_selected_release(home: &AosHome) -> io::Result<VerifiedDistro> {
    let release_dir = home.release_dir();
    require_directory(&release_dir, "release directory")?;

    let manifest_path = release_dir.join("Distro.toml");
    let lock_path = release_dir.join("Distro.lock");
    let signature_path = release_dir.join("Distro.sig");
    require_private_release_file(&manifest_path, "bundled Distro.toml")?;
    require_private_release_file(&lock_path, "bundled Distro.lock")?;
    require_private_release_file(&signature_path, "bundled Distro.sig")?;

    let manifest_bytes = fs::read(&manifest_path)?;
    let manifest_text = std::str::from_utf8(&manifest_bytes).map_err(|error| {
        invalid_data(format!("bundled Distro.toml is not valid UTF-8: {error}"))
    })?;
    let manifest: toml::Value = toml::from_str(manifest_text)
        .map_err(|error| invalid_data(format!("failed to parse bundled Distro.toml: {error}")))?;
    let schema_version = manifest
        .get("schema-version")
        .and_then(toml::Value::as_integer)
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| invalid_data("Distro.toml is missing integer schema-version"))?;
    let distro = table(&manifest, "distro", "bundled Distro.toml")?;
    if schema_version != 1 {
        return Err(invalid_data(format!(
            "unsupported bundled Distro.toml schema-version {schema_version}"
        )));
    }
    let distro_id = required_string(distro, "id", "Distro.toml [distro]")?;
    if distro_id != SELECTED_DISTRO_ID {
        return Err(invalid_data(format!(
            "bundled distro id must be {SELECTED_DISTRO_ID:?}, got {distro_id:?}"
        )));
    }
    let distro_version = required_string(distro, "version", "Distro.toml [distro]")?;
    if distro_version != env!("CARGO_PKG_VERSION") {
        return Err(invalid_data(format!(
            "bundled distro version {distro_version:?} does not match AOS release {}",
            env!("CARGO_PKG_VERSION")
        )));
    }
    let astrid_version = required_string(distro, "astrid-version", "Distro.toml [distro]")?;
    if astrid_version != format!("={ASTRID_RUNTIME_VERSION}") {
        return Err(invalid_data(format!(
            "bundled distro requires Astrid {astrid_version:?}, expected ={ASTRID_RUNTIME_VERSION}"
        )));
    }
    let signing = distro
        .get("signing")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| invalid_data("bundled Distro.toml is missing [distro.signing]"))?;
    let signing_pubkey = required_string(signing, "pubkey", "Distro.toml [distro.signing]")?;
    let public_key = parse_public_key(&signing_pubkey)?;
    let canonical_pubkey = format!("ed25519:{}", public_key.to_base64());
    if signing_pubkey != canonical_pubkey {
        return Err(invalid_data(
            "bundled Distro.toml signing pubkey is not canonical ed25519 wire form",
        ));
    }

    let lock_bytes = fs::read(&lock_path)?;
    let lock_text = std::str::from_utf8(&lock_bytes).map_err(|error| {
        invalid_data(format!("bundled Distro.lock is not valid UTF-8: {error}"))
    })?;
    let lock: DistroLock = toml::from_str(lock_text)
        .map_err(|error| invalid_data(format!("failed to parse bundled Distro.lock: {error}")))?;
    if lock.schema_version != schema_version {
        return Err(invalid_data(format!(
            "Distro.lock schema-version {} does not match Distro.toml {schema_version}",
            lock.schema_version
        )));
    }
    if lock.distro.id != distro_id || lock.distro.version != distro_version {
        return Err(invalid_data(
            "Distro.lock identity does not match bundled Distro.toml",
        ));
    }
    let manifest_blake3 = format!("blake3:{}", blake3::hash(&manifest_bytes).to_hex());
    if lock.manifest_hash.as_deref() != Some(manifest_blake3.as_str()) {
        return Err(invalid_data(
            "Distro.lock manifest-hash does not bind the exact bundled Distro.toml bytes",
        ));
    }

    let canonical_lock = serde_json::to_vec(&lock).map_err(|error| {
        invalid_data(format!(
            "failed to canonicalize bundled Distro.lock: {error}"
        ))
    })?;
    let mut hasher = blake3::Hasher::new();
    hasher.update(SIG_DOMAIN_TAG);
    hasher.update(&canonical_lock);
    let signing_digest = hasher.finalize();
    let signature_text = fs::read_to_string(&signature_path)?;
    let signature = Signature::from_hex(signature_text.trim()).map_err(|error| {
        invalid_data(format!(
            "malformed bundled Distro.sig (expected 64-byte hex): {error}"
        ))
    })?;
    public_key
        .verify(signing_digest.as_bytes(), &signature)
        .map_err(|_| {
            invalid_data("bundled Distro.sig does not verify against Distro.lock and pubkey")
        })?;

    verify_release_inventory(
        &release_dir,
        &manifest_bytes,
        &lock_bytes,
        signature_text.as_bytes(),
    )?;

    Ok(VerifiedDistro {
        manifest_path,
        distro_id,
        distro_version,
        manifest_blake3,
        signing_pubkey,
        lock_blake3: format!("blake3:{}", blake3::hash(&lock_bytes).to_hex()),
        signature_blake3: format!(
            "blake3:{}",
            blake3::hash(signature_text.as_bytes()).to_hex()
        ),
    })
}

/// Seed the runtime trust file with the already-verified package key.
pub(crate) fn seed_runtime_trust(home: &AosHome, signing_pubkey: &str) -> io::Result<()> {
    let trust_dir = home.runtime_home().join("trust");
    ensure_private_directory(&trust_dir)?;
    let trust_path = trust_dir.join(format!("{SELECTED_DISTRO_ID}.pub"));
    match fs::symlink_metadata(&trust_path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(
            invalid_data("runtime distro trust pin must be a regular file"),
        ),
        Ok(_) => {
            let metadata = fs::metadata(&trust_path)?;
            require_private_mode(&metadata, "runtime distro trust pin")?;
            let existing = fs::read_to_string(&trust_path)?;
            if existing.trim() == signing_pubkey {
                Ok(())
            } else {
                Err(invalid_data(
                    "runtime distro trust pin does not match bundled Distro.toml",
                ))
            }
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            write_private_atomic(&trust_path, format!("{signing_pubkey}\n").as_bytes())
        }
        Err(error) => Err(error),
    }
}

/// Refuse to dispatch when an existing receipt binds another apply.
pub(crate) fn check_existing_receipt(
    home: &AosHome,
    distro: &VerifiedDistro,
    principal: &str,
) -> io::Result<()> {
    let path = receipt_path(home);
    let metadata = match fs::symlink_metadata(&path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid_data(
            "AOS Distro Apply receipt must be a regular file",
        ));
    }
    require_private_mode(&metadata, "AOS Distro Apply receipt")?;
    let bytes = fs::read(&path)?;
    let receipt: ActiveReceipt = serde_json::from_slice(&bytes).map_err(|error| {
        invalid_data(format!("AOS Distro Apply receipt is invalid JSON: {error}"))
    })?;
    if receipt.schema != RECEIPT_SCHEMA_VERSION
        || receipt.kind != RECEIPT_KIND
        || !receipt.active
        || !receipt.cutover_complete
        || receipt.distro_id != distro.distro_id
        || receipt.distro_version != distro.distro_version
        || receipt.manifest_blake3 != distro.manifest_blake3
        || receipt.signing_pubkey != distro.signing_pubkey
        || receipt.pin_blake3 != blake3::hash(distro.signing_pubkey.as_bytes()).to_string()
        || receipt.lock_blake3 != distro.lock_blake3
        || receipt.signature_blake3 != distro.signature_blake3
        || receipt.principal != principal
        || receipt.astrid_runtime_version != ASTRID_RUNTIME_VERSION
    {
        return Err(invalid_data(
            "existing AOS Distro Apply receipt does not match the selected distro, key, or principal",
        ));
    }
    Ok(())
}

/// Write the success receipt after exact stop confirmation.
pub(crate) fn write_active_receipt(
    home: &AosHome,
    distro: &VerifiedDistro,
    principal: &str,
) -> io::Result<()> {
    let receipts_dir = home.root().join("receipts");
    ensure_private_directory(&receipts_dir)?;
    let receipt = ActiveReceipt {
        schema: RECEIPT_SCHEMA_VERSION,
        kind: RECEIPT_KIND.to_owned(),
        active: true,
        cutover_complete: true,
        distro_id: distro.distro_id.clone(),
        distro_version: distro.distro_version.clone(),
        manifest_blake3: distro.manifest_blake3.clone(),
        signing_pubkey: distro.signing_pubkey.clone(),
        pin_blake3: blake3::hash(distro.signing_pubkey.as_bytes()).to_string(),
        lock_blake3: distro.lock_blake3.clone(),
        signature_blake3: distro.signature_blake3.clone(),
        principal: principal.to_owned(),
        astrid_runtime_version: ASTRID_RUNTIME_VERSION.to_owned(),
    };
    let bytes = serde_json::to_vec_pretty(&receipt).map_err(|error| {
        invalid_data(format!(
            "failed to encode AOS Distro Apply receipt: {error}"
        ))
    })?;
    let path = receipts_dir.join(format!("{SELECTED_DISTRO_ID}.active.json"));
    write_private_atomic(&path, &bytes)
}

/// Require the materialized runtime state produced by a successful Distro Apply.
///
/// Generic stopped-state confirmation preserves an empty or absent runtime home
/// for a fresh installation.  Distro Apply is stricter: a successful apply must
/// leave exactly one non-empty private volume before its activation receipt is
/// written.
pub(crate) fn require_stopped_volume(home: &AosHome) -> io::Result<()> {
    let runtime = home.runtime_home();
    require_directory(&runtime, "stopped runtime state")?;
    for entry in fs::read_dir(&runtime)? {
        let entry = entry?;
        if entry.file_name() != OsStr::new("astrid.volume") {
            return Err(invalid_data(
                "stopped runtime state must contain only astrid.volume",
            ));
        }
    }

    let volume = runtime.join("astrid.volume");
    require_regular_file(&volume, "stopped runtime astrid.volume")?;
    let metadata = fs::metadata(&volume)?;
    if metadata.len() == 0 {
        return Err(invalid_data(
            "stopped runtime astrid.volume must not be empty",
        ));
    }
    require_private_mode(&metadata, "stopped runtime astrid.volume")
}

fn receipt_path(home: &AosHome) -> PathBuf {
    home.root()
        .join("receipts")
        .join(format!("{SELECTED_DISTRO_ID}.active.json"))
}

fn verify_release_inventory(
    release_dir: &Path,
    manifest_bytes: &[u8],
    lock_bytes: &[u8],
    signature_bytes: &[u8],
) -> io::Result<()> {
    let path = release_dir.join("release-manifest.json");
    require_regular_file(&path, "release-manifest.json")?;
    let metadata = fs::metadata(&path)?;
    require_private_mode(&metadata, "release-manifest.json")?;
    let bytes = fs::read(&path)?;
    let manifest: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|error| invalid_data(format!("release-manifest.json is invalid JSON: {error}")))?;
    let files = manifest
        .get("release_files")
        .and_then(serde_json::Value::as_object)
        .ok_or_else(|| invalid_data("release-manifest.json is missing release_files"))?;
    for (name, contents) in [
        ("Distro.toml", manifest_bytes),
        ("Distro.lock", lock_bytes),
        ("Distro.sig", signature_bytes),
    ] {
        let entry = files
            .get(name)
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| invalid_data(format!("release-manifest.json is missing {name}")))?;
        let expected = format!("blake3:{}", blake3::hash(contents).to_hex());
        let declared = entry
            .get("blake3")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| invalid_data(format!("release-manifest.json has no {name} digest")))?;
        if declared != expected.strip_prefix("blake3:").unwrap_or(&expected) && declared != expected
        {
            return Err(invalid_data(format!(
                "release-manifest.json digest does not match {name}"
            )));
        }
        if entry.get("mode").and_then(serde_json::Value::as_u64) != Some(0o600) {
            return Err(invalid_data(format!(
                "release-manifest.json requires private mode 0600 for {name}"
            )));
        }
    }
    Ok(())
}

fn parse_public_key(wire: &str) -> io::Result<PublicKey> {
    let encoded = wire.strip_prefix("ed25519:").ok_or_else(|| {
        invalid_data("bundled Distro.toml signing pubkey must use ed25519:<base64> wire form")
    })?;
    PublicKey::from_base64(encoded).map_err(|error| {
        invalid_data(format!(
            "invalid bundled Distro.toml signing pubkey: {error}"
        ))
    })
}

fn table<'a>(
    value: &'a toml::Value,
    key: &str,
    context: &str,
) -> io::Result<&'a toml::map::Map<String, toml::Value>> {
    value
        .get(key)
        .and_then(toml::Value::as_table)
        .ok_or_else(|| invalid_data(format!("{context} is missing [{key}]")))
}

fn required_string(
    table: &toml::map::Map<String, toml::Value>,
    key: &str,
    context: &str,
) -> io::Result<String> {
    table
        .get(key)
        .and_then(toml::Value::as_str)
        .filter(|value| !value.is_empty())
        .map(ToOwned::to_owned)
        .ok_or_else(|| invalid_data(format!("{context} is missing non-empty {key}")))
}

fn require_directory(path: &Path, label: &str) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            invalid_data(format!("{label} is missing at {}", path.display()))
        } else {
            error
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(invalid_data(format!(
            "{label} must be a real directory at {}",
            path.display()
        )));
    }
    Ok(())
}

fn require_regular_file(path: &Path, label: &str) -> io::Result<()> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        if error.kind() == io::ErrorKind::NotFound {
            invalid_data(format!("{label} is missing at {}", path.display()))
        } else {
            error
        }
    })?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(invalid_data(format!(
            "{label} must be a regular file at {}",
            path.display()
        )));
    }
    Ok(())
}

fn require_private_release_file(path: &Path, label: &str) -> io::Result<()> {
    require_regular_file(path, label)?;
    let metadata = fs::metadata(path)?;
    require_private_mode(&metadata, label)
}

fn require_private_mode(metadata: &fs::Metadata, label: &str) -> io::Result<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = metadata.permissions().mode() & 0o7777;
        if mode != 0o600 {
            return Err(invalid_data(format!(
                "{label} must use private mode 0600 (found {mode:o})"
            )));
        }
    }
    #[cfg(not(unix))]
    let _ = (metadata, label);
    Ok(())
}

fn ensure_private_directory(path: &Path) -> io::Result<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(invalid_data(format!(
                "private directory must be a real directory at {}",
                path.display()
            )));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => fs::create_dir_all(path)?,
        Err(error) => return Err(error),
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(path)?.permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions)?;
    }
    Ok(())
}

fn write_private_atomic(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let parent = path
        .parent()
        .ok_or_else(|| invalid_data("private file has no parent directory"))?;
    ensure_private_directory(parent)?;
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(invalid_data(format!(
            "private file must be a regular file at {}",
            path.display()
        )));
    }
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| invalid_data("private file name is not valid UTF-8"))?;
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    let temporary = parent.join(format!(".{name}.tmp-{}-{nonce}", std::process::id()));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = file.metadata()?.permissions();
        permissions.set_mode(0o600);
        file.set_permissions(permissions)?;
    }
    if let Err(error) = file.write_all(bytes).and_then(|()| file.sync_all()) {
        drop(file);
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    drop(file);
    if let Ok(metadata) = fs::symlink_metadata(path)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        let _ = fs::remove_file(&temporary);
        return Err(invalid_data(format!(
            "private file must be a regular file at {}",
            path.display()
        )));
    }
    if let Err(error) = fs::rename(&temporary, path) {
        #[cfg(windows)]
        if error.kind() == io::ErrorKind::AlreadyExists {
            if let Err(remove_error) = fs::remove_file(path) {
                let _ = fs::remove_file(&temporary);
                return Err(remove_error);
            }
            if let Err(rename_error) = fs::rename(&temporary, path) {
                let _ = fs::remove_file(&temporary);
                return Err(rename_error);
            }
            return Ok(());
        }
        let _ = fs::remove_file(&temporary);
        return Err(error);
    }
    Ok(())
}

fn invalid_data(message: impl Into<String>) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message.into())
}
