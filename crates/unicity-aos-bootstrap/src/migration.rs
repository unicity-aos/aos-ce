//! Explicit, staged import of a standalone Astrid Runtime home.

use std::collections::HashSet;
use std::ffi::OsStr;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::path::{Component, Path, PathBuf};

use fs2::FileExt;
use serde::de::Error as _;
use serde::{Deserialize, Serialize};

use crate::{AosHome, RUNTIME_EXECUTABLE_NAMES};

const MIGRATION_VERSION: u32 = 1;
const RECEIPT_SCHEMA_VERSION: u32 = 3;
const STAGING_DIR: &str = ".runtime-import";
const LOCK_FILE: &str = "astrid-home-v1.lock";
const PERSISTENT_TOP_LEVEL: &[&str] = &[
    "config.toml",
    "keys",
    "secrets",
    "var",
    "wit",
    "home",
    "lib",
    "trust",
    "capsules",
    "history",
    "log",
];
const EPHEMERAL_TOP_LEVEL: &[&str] = &["run", "cow"];
const ETC_ALLOWLIST: &[&str] = &[
    "config.toml",
    "servers.toml",
    "gateway.toml",
    "gateway-http.toml",
    "layout-version",
    "groups.toml",
    "invites.toml",
    "pair-tokens.toml",
    "gateway-revocations.json",
    "profiles",
    "hooks",
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MigrationOutcome {
    Migrated,
    AlreadyMigrated,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LegacyDistro {
    pub principal: String,
    pub id: String,
    pub version: String,
}

#[derive(Debug, Serialize, Deserialize)]
struct Receipt {
    #[serde(default)]
    migration_version: u32,
    #[serde(default)]
    schema_version: u32,
    source: PathBuf,
    entries: Vec<Entry>,
    #[serde(default)]
    legacy_distros: Vec<LegacyDistro>,
}

#[derive(Deserialize)]
struct ReceiptHeader {
    schema_version: u32,
}

#[derive(Debug, Serialize, Deserialize)]
struct Entry {
    path: PathBuf,
    bytes: u64,
    digest: Blake3Digest,
}

/// Canonical BLAKE3 content digest used by the private migration receipt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Blake3Digest([u8; 32]);

impl Blake3Digest {
    const PREFIX: &'static str = "blake3:";

    fn from_hash(hash: blake3::Hash) -> Self {
        Self(*hash.as_bytes())
    }

    fn parse(value: &str) -> Result<Self, &'static str> {
        let Some(hex) = value.strip_prefix(Self::PREFIX) else {
            return Err("migration digest must use blake3:<64 lowercase hex>");
        };
        if hex.len() != 64
            || !hex
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            return Err("migration digest must use blake3:<64 lowercase hex>");
        }
        let hash = blake3::Hash::from_hex(hex)
            .map_err(|_| "migration digest must use blake3:<64 lowercase hex>")?;
        Ok(Self::from_hash(hash))
    }
}

impl fmt::Display for Blake3Digest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}{}",
            Self::PREFIX,
            blake3::Hash::from_bytes(self.0).to_hex()
        )
    }
}

impl Serialize for Blake3Digest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Blake3Digest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).map_err(D::Error::custom)
    }
}

struct MigrationLock {
    _file: File,
}

impl MigrationLock {
    fn acquire(home: &AosHome) -> io::Result<Self> {
        let path = home.root().join("migrations").join(LOCK_FILE);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;
        if !file.metadata()?.is_file() {
            return invalid("runtime migration lock must be a regular file");
        }
        set_private_permissions(&path, false)?;
        file.try_lock_exclusive().map_err(|error| {
            if error.kind() == io::ErrorKind::WouldBlock {
                io::Error::new(
                    io::ErrorKind::WouldBlock,
                    "another runtime migration is already in progress",
                )
            } else {
                error
            }
        })?;
        Ok(Self { _file: file })
    }
}

pub(crate) fn migrate_runtime(home: &AosHome, source: &Path) -> io::Result<MigrationOutcome> {
    let source = checked_root(source, "legacy runtime home")?;
    let target = checked_target_path(&home.runtime_home())?;
    if source == target || source.starts_with(&target) || target.starts_with(&source) {
        return invalid("legacy runtime home and product runtime home must not overlap");
    }
    if source.join("run/system.sock").exists() {
        return invalid("stop the standalone runtime before migration");
    }

    create_private_dir(&home.root().join("migrations"))?;
    let _migration_lock = MigrationLock::acquire(home)?;
    let receipt_path = home.migration_receipt();
    recover_interrupted_transaction(home, &target, &source, &receipt_path)?;
    if receipt_path.is_file() {
        let receipt: Receipt = read_receipt(&receipt_path)?;
        if receipt.source == source && receipt_matches(&target, &receipt)? {
            remove_backup(&target_backup(&target))?;
            return Ok(MigrationOutcome::AlreadyMigrated);
        }
        return invalid(
            "an existing migration receipt does not match the requested source or target",
        );
    }

    validate_target(&target)?;
    validate_source_layout(&source)?;
    let staging = home.root().join(STAGING_DIR);
    if staging.exists() {
        return invalid(
            "a previous migration staging directory could not be recovered automatically",
        );
    }

    let result = (|| {
        create_private_dir(&staging)?;
        let mut entries = copy_product_binaries(&target, &staging)?;
        copy_etc_state(&source, &staging, &mut entries)?;
        for name in PERSISTENT_TOP_LEVEL {
            copy_if_present(
                &source.join(name),
                &staging.join(name),
                Path::new(name),
                &mut entries,
            )?;
        }
        copy_wasm_blobs(&source.join("bin"), &staging.join("bin"), &mut entries)?;
        ensure_no_ephemeral_data(&staging)?;
        entries.sort_by(|left, right| left.path.cmp(&right.path));
        let receipt = Receipt {
            migration_version: MIGRATION_VERSION,
            schema_version: RECEIPT_SCHEMA_VERSION,
            source: source.clone(),
            entries,
            legacy_distros: legacy_distros(&staging)?,
        };
        if !receipt_matches(&staging, &receipt)? {
            return invalid("staged runtime did not validate against its import manifest");
        }
        let staged_receipt = write_staged_receipt(&receipt_path, &receipt)?;
        let backup = replace_target(&target, &staging)?;
        if let Err(error) = finalize_receipt(&staged_receipt, &receipt_path) {
            remove_path(&receipt_path)?;
            remove_path(&staged_receipt)?;
            rollback_target(&target, &backup)?;
            return Err(error);
        }
        remove_backup(&backup)?;
        Ok(())
    })();
    if result.is_err() && staging.exists() {
        let _ = remove_path(&staging);
    }
    result.map(|()| MigrationOutcome::Migrated)
}

fn checked_target_path(path: &Path) -> io::Result<PathBuf> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink() || !metadata.is_dir() {
                return invalid("bundled product runtime must be a real directory");
            }
            path.canonicalize()
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let parent = path.parent().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "bundled product runtime path must have a parent",
                )
            })?;
            let name = path.file_name().ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::InvalidInput,
                    "bundled product runtime path must have a final component",
                )
            })?;
            Ok(parent.canonicalize()?.join(name))
        }
        Err(error) => Err(error),
    }
}

pub(crate) fn imported_legacy_distros(home: &AosHome) -> io::Result<Vec<LegacyDistro>> {
    let receipt = read_receipt(&home.migration_receipt())?;
    Ok(receipt.legacy_distros)
}

fn legacy_distros(runtime_home: &Path) -> io::Result<Vec<LegacyDistro>> {
    let homes = runtime_home.join("home");
    if !homes.is_dir() {
        return Ok(Vec::new());
    }
    let mut distros = Vec::new();
    for entry in fs::read_dir(homes)? {
        let entry = entry?;
        let principal = entry.file_name().to_string_lossy().into_owned();
        let lock = entry.path().join(".config/distro.lock");
        let Ok(contents) = fs::read_to_string(lock) else {
            continue;
        };
        let Ok(value) = contents.parse::<toml::Value>() else {
            continue;
        };
        let Some(distro) = value.get("distro") else {
            continue;
        };
        let (Some(id), Some(version)) = (
            distro.get("id").and_then(toml::Value::as_str),
            distro.get("version").and_then(toml::Value::as_str),
        ) else {
            continue;
        };
        if matches!(id, "astralis" | "aos-ce") {
            distros.push(LegacyDistro {
                principal,
                id: id.to_owned(),
                version: version.to_owned(),
            });
        }
    }
    distros.sort_by(|left, right| left.principal.cmp(&right.principal));
    Ok(distros)
}

fn checked_root(path: &Path, description: &str) -> io::Result<PathBuf> {
    if !path.is_absolute() {
        return invalid(&format!("{description} must be an absolute path"));
    }
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return invalid(&format!(
            "{description} must be a real directory, not a symlink"
        ));
    }
    path.canonicalize()
}

fn validate_target(target: &Path) -> io::Result<()> {
    let target_metadata = fs::symlink_metadata(target).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "bundled product runtime is not installed",
        )
    })?;
    if target_metadata.file_type().is_symlink() || !target_metadata.is_dir() {
        return invalid("bundled product runtime must be a real directory");
    }
    let bin = target.join("bin");
    let bin_metadata = fs::symlink_metadata(&bin).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "bundled product runtime executable set is not installed",
        )
    })?;
    if bin_metadata.file_type().is_symlink() || !bin_metadata.is_dir() {
        return invalid("bundled product runtime bin must be a real directory");
    }
    let expected: HashSet<_> = RUNTIME_EXECUTABLE_NAMES.iter().copied().collect();
    let mut actual = HashSet::new();
    for entry in fs::read_dir(&bin)? {
        let entry = entry?;
        let name = entry.file_name().into_string().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "product runtime bin contains a non-UTF-8 entry",
            )
        })?;
        let metadata = fs::symlink_metadata(entry.path())?;
        if !expected.contains(name.as_str())
            || metadata.file_type().is_symlink()
            || !metadata.is_file()
            || !actual.insert(name)
        {
            return invalid("product runtime home contains data; migration refuses to merge state");
        }
    }
    if actual.len() != expected.len() {
        return invalid("bundled product runtime executable set is incomplete");
    }
    for entry in fs::read_dir(target)? {
        let entry = entry?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if path != bin || metadata.file_type().is_symlink() || !metadata.is_dir() {
            return invalid("product runtime home contains data; migration refuses to merge state");
        }
    }
    Ok(())
}

fn validate_source_layout(source: &Path) -> io::Result<()> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name().into_string().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "legacy runtime contains a non-UTF-8 top-level path",
            )
        })?;
        let known = name == "etc"
            || name == "bin"
            || PERSISTENT_TOP_LEVEL.contains(&name.as_str())
            || EPHEMERAL_TOP_LEVEL.contains(&name.as_str());
        if !known {
            return invalid(&format!(
                "legacy runtime contains unsupported top-level state `{name}`; migration refuses to omit it"
            ));
        }
    }
    Ok(())
}

fn copy_etc_state(source_root: &Path, staging: &Path, entries: &mut Vec<Entry>) -> io::Result<()> {
    let source = source_root.join("etc");
    let metadata = match fs::symlink_metadata(&source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return invalid("legacy runtime etc must be a real directory");
    }

    for entry in fs::read_dir(&source)? {
        let entry = entry?;
        let name = entry.file_name().into_string().map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "legacy runtime etc contains a non-UTF-8 path",
            )
        })?;
        if !ETC_ALLOWLIST.contains(&name.as_str()) {
            return invalid(&format!(
                "legacy runtime contains unsupported configuration `etc/{name}`; migration refuses to omit it"
            ));
        }
        let relative = PathBuf::from("etc").join(&name);
        copy_tree(&entry.path(), &staging.join(&relative), &relative, entries)?;
    }
    Ok(())
}

fn copy_product_binaries(target: &Path, staging: &Path) -> io::Result<Vec<Entry>> {
    RUNTIME_EXECUTABLE_NAMES
        .iter()
        .map(|name| {
            let source = target.join("bin").join(name);
            let relative = PathBuf::from("bin").join(name);
            let destination = staging.join(&relative);
            copy_executable(&source, &destination, &relative)
        })
        .collect()
}

fn copy_if_present(
    source: &Path,
    destination: &Path,
    relative: &Path,
    entries: &mut Vec<Entry>,
) -> io::Result<()> {
    match fs::symlink_metadata(source) {
        Ok(_) => copy_tree(source, destination, relative, entries)?,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    Ok(())
}

fn copy_wasm_blobs(source: &Path, destination: &Path, entries: &mut Vec<Entry>) -> io::Result<()> {
    let metadata = match fs::symlink_metadata(source) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return invalid("legacy runtime bin must be a real directory");
    }
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path)?;
        if metadata.file_type().is_symlink() || !metadata.is_file() {
            return invalid("legacy runtime bin contains a non-regular entry");
        }
        if Path::new(&name)
            .extension()
            .is_some_and(|ext| ext == "wasm")
        {
            let relative = PathBuf::from("bin").join(&name);
            entries.push(copy_file(&path, &destination.join(&name), &relative)?);
        } else if !RUNTIME_EXECUTABLE_NAMES
            .iter()
            .any(|runtime| name == OsStr::new(runtime))
        {
            return invalid(&format!(
                "legacy runtime bin contains unsupported state `{}`; migration refuses to omit it",
                name.to_string_lossy()
            ));
        }
    }
    Ok(())
}

fn copy_tree(
    source: &Path,
    destination: &Path,
    relative: &Path,
    entries: &mut Vec<Entry>,
) -> io::Result<()> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() {
        return invalid("legacy runtime contains a symlink; migration refuses to follow links");
    }
    if metadata.is_file() {
        entries.push(copy_file(source, destination, relative)?);
        return Ok(());
    }
    if !metadata.is_dir() {
        return invalid("legacy runtime contains a non-regular file");
    }
    create_private_dir(destination)?;
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let name = entry.file_name();
        if Path::new(&name)
            .components()
            .any(|part| matches!(part, Component::ParentDir))
        {
            return invalid("unsafe legacy runtime path");
        }
        copy_tree(
            &entry.path(),
            &destination.join(&name),
            &relative.join(name),
            entries,
        )?;
    }
    Ok(())
}

fn copy_file(source: &Path, destination: &Path, relative: &Path) -> io::Result<Entry> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return invalid("migration only copies regular files");
    }
    if let Some(parent) = destination.parent() {
        create_private_dir(parent)?;
    }
    let mut input = File::open(source)?;
    let mut output = File::create(destination)?;
    let mut hasher = blake3::Hasher::new();
    let mut bytes = 0_u64;
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = input.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        output.write_all(&buffer[..read])?;
        hasher.update(&buffer[..read]);
        bytes = bytes
            .checked_add(read as u64)
            .ok_or_else(|| io::Error::other("migration byte count overflow"))?;
    }
    set_private_copied_file_permissions(destination, &metadata)?;
    output.sync_all()?;
    sync_parent(destination)?;
    Ok(Entry {
        path: relative.to_path_buf(),
        bytes,
        digest: Blake3Digest::from_hash(hasher.finalize()),
    })
}

#[cfg(unix)]
fn set_private_copied_file_permissions(path: &Path, source: &fs::Metadata) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;

    let executable = source.permissions().mode() & 0o111 != 0;
    fs::set_permissions(
        path,
        fs::Permissions::from_mode(if executable { 0o700 } else { 0o600 }),
    )
}

#[cfg(not(unix))]
fn set_private_copied_file_permissions(_path: &Path, _source: &fs::Metadata) -> io::Result<()> {
    Ok(())
}

fn copy_executable(source: &Path, destination: &Path, relative: &Path) -> io::Result<Entry> {
    let metadata = fs::symlink_metadata(source)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return invalid("bundled runtime executable must be a regular file");
    }
    if let Some(parent) = destination.parent() {
        create_private_dir(parent)?;
    }
    let bytes = fs::copy(source, destination)?;
    fs::set_permissions(destination, metadata.permissions())?;
    File::open(destination)?.sync_all()?;
    sync_parent(destination)?;
    Ok(Entry {
        path: relative.to_path_buf(),
        bytes,
        digest: blake3_file(destination)?,
    })
}

fn replace_target(target: &Path, staging: &Path) -> io::Result<PathBuf> {
    let backup = target_backup(target);
    if backup.exists() {
        return invalid("previous product runtime backup exists; inspect it before migration");
    }
    if target.exists() {
        fs::rename(target, &backup)?;
        sync_parent(target)?;
    }
    if let Err(error) = fs::rename(staging, target) {
        let _ = fs::rename(&backup, target);
        let _ = sync_parent(target);
        return Err(error);
    }
    sync_parent(target)?;
    Ok(backup)
}

fn target_backup(target: &Path) -> PathBuf {
    target.with_extension("pre-migration")
}

fn rollback_target(target: &Path, backup: &Path) -> io::Result<()> {
    let failed_target = target.with_extension("failed-migration");
    if failed_target.exists() {
        return invalid("failed migration target already exists; manual recovery is required");
    }
    fs::rename(target, &failed_target)?;
    sync_parent(target)?;
    if let Err(error) = fs::rename(backup, target) {
        let _ = fs::rename(&failed_target, target);
        let _ = sync_parent(target);
        return Err(error);
    }
    sync_parent(target)?;
    fs::remove_dir_all(failed_target)?;
    sync_parent(target)
}

fn remove_backup(backup: &Path) -> io::Result<()> {
    if backup.exists() {
        fs::remove_dir_all(backup)?;
        sync_parent(backup)?;
    }
    Ok(())
}

fn recover_interrupted_transaction(
    home: &AosHome,
    target: &Path,
    source: &Path,
    receipt_path: &Path,
) -> io::Result<()> {
    let backup = target_backup(target);
    let staging = home.root().join(STAGING_DIR);
    let staged_receipt = staged_receipt_path(receipt_path);

    if receipt_path.is_file() {
        return Ok(());
    }

    if !backup.exists() {
        if staging.exists() || staged_receipt.exists() {
            remove_path(&staging)?;
            remove_path(&staged_receipt)?;
        }
        return Ok(());
    }

    if !target.exists() {
        fs::rename(&backup, target)?;
        sync_parent(target)?;
        remove_path(&staging)?;
        remove_path(&staged_receipt)?;
        return Ok(());
    }

    let can_complete = read_receipt(&staged_receipt)
        .ok()
        .filter(|receipt| receipt.source == source)
        .is_some_and(|receipt| receipt_matches(target, &receipt).unwrap_or(false));
    if can_complete {
        finalize_receipt(&staged_receipt, receipt_path)?;
        remove_backup(&backup)?;
        return Ok(());
    }

    rollback_target(target, &backup)?;
    remove_path(&staging)?;
    remove_path(&staged_receipt)
}

fn remove_path(path: &Path) -> io::Result<()> {
    let Ok(metadata) = fs::symlink_metadata(path) else {
        return Ok(());
    };
    if metadata.is_dir() && !metadata.file_type().is_symlink() {
        fs::remove_dir_all(path)?;
    } else {
        fs::remove_file(path)?;
    }
    sync_parent(path)
}

fn ensure_no_ephemeral_data(staging: &Path) -> io::Result<()> {
    if EPHEMERAL_TOP_LEVEL
        .iter()
        .any(|name| staging.join(name).exists())
    {
        return invalid("ephemeral runtime data entered migration staging");
    }
    Ok(())
}

fn receipt_matches(root: &Path, receipt: &Receipt) -> io::Result<bool> {
    validate_receipt(receipt)?;
    for entry in &receipt.entries {
        let path = root.join(&entry.path);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        };
        if metadata.file_type().is_symlink()
            || !metadata.is_file()
            || metadata.len() != entry.bytes
            || blake3_file(&path)? != entry.digest
        {
            return Ok(false);
        }
    }
    Ok(true)
}

fn write_staged_receipt(path: &Path, receipt: &Receipt) -> io::Result<PathBuf> {
    validate_receipt(receipt)?;
    let temporary = staged_receipt_path(path);
    let bytes = serde_json::to_vec_pretty(receipt).map_err(io::Error::other)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)?;
    file.write_all(&bytes)?;
    set_private_permissions(&temporary, false)?;
    file.sync_all()?;
    sync_parent(&temporary)?;
    Ok(temporary)
}

fn staged_receipt_path(path: &Path) -> PathBuf {
    path.with_extension("tmp")
}

fn finalize_receipt(temporary: &Path, path: &Path) -> io::Result<()> {
    fs::rename(temporary, path)?;
    sync_parent(path)
}

fn create_private_dir(path: &Path) -> io::Result<()> {
    fs::create_dir_all(path)?;
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return invalid("runtime migration managed paths must be real directories");
    }
    set_private_permissions(path, true)?;
    sync_directory(path)?;
    sync_parent(path)
}

fn read_receipt(path: &Path) -> io::Result<Receipt> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return invalid("migration receipt must be a regular file");
    }
    let bytes = fs::read(path)?;
    let header: ReceiptHeader = serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    if header.schema_version != RECEIPT_SCHEMA_VERSION {
        return invalid("unsupported runtime migration receipt schema");
    }
    let receipt = serde_json::from_slice(&bytes)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidInput, error))?;
    validate_receipt(&receipt)?;
    Ok(receipt)
}

fn validate_receipt(receipt: &Receipt) -> io::Result<()> {
    if receipt.migration_version != MIGRATION_VERSION {
        return invalid("unsupported runtime migration version in receipt");
    }
    if receipt.schema_version != RECEIPT_SCHEMA_VERSION {
        return invalid("unsupported runtime migration receipt schema");
    }
    let mut paths = HashSet::new();
    for entry in &receipt.entries {
        if !is_safe_relative(&entry.path) {
            return invalid("migration receipt contains an unsafe path");
        }
        if !paths.insert(entry.path.clone()) {
            return invalid("migration receipt contains a duplicate path");
        }
    }
    Ok(())
}

fn is_safe_relative(path: &Path) -> bool {
    !path.as_os_str().is_empty()
        && !path.is_absolute()
        && path
            .components()
            .all(|component| matches!(component, Component::Normal(_)))
}

fn blake3_file(path: &Path) -> io::Result<Blake3Digest> {
    let mut file = File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(Blake3Digest::from_hash(hasher.finalize()))
}

fn sync_parent(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        sync_directory(parent)?;
    }
    Ok(())
}

#[cfg(unix)]
fn sync_directory(path: &Path) -> io::Result<()> {
    File::open(path)?.sync_all()
}

#[cfg(not(unix))]
fn sync_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(unix)]
fn set_private_permissions(path: &Path, directory: bool) -> io::Result<()> {
    use std::os::unix::fs::PermissionsExt;
    fs::set_permissions(
        path,
        fs::Permissions::from_mode(if directory { 0o700 } else { 0o600 }),
    )
}

#[cfg(not(unix))]
fn set_private_permissions(_path: &Path, _directory: bool) -> io::Result<()> {
    Ok(())
}

fn invalid<T>(message: &str) -> io::Result<T> {
    Err(io::Error::new(
        io::ErrorKind::InvalidInput,
        message.to_owned(),
    ))
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::ffi::OsStr;
    use std::fs;
    use std::path::{Component, Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::{
        Blake3Digest, MIGRATION_VERSION, MigrationLock, MigrationOutcome, RECEIPT_SCHEMA_VERSION,
        create_private_dir, migrate_runtime, recover_interrupted_transaction,
    };
    use crate::AosHome;

    fn fixture_root(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "unicity-aos-{name}-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create fixture root");
        root
    }

    fn write(root: &Path, relative: &str, content: &[u8]) {
        let path = root.join(relative);
        fs::create_dir_all(path.parent().expect("fixture parent")).expect("create fixture parent");
        fs::write(path, content).expect("write fixture file");
    }

    fn install_product_runtime(product: &AosHome) {
        for name in crate::RUNTIME_EXECUTABLE_NAMES {
            let relative = format!("bin/{name}");
            write(
                &product.runtime_home(),
                &relative,
                format!("bundled-{name}").as_bytes(),
            );
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(
                    product.runtime_home().join(relative),
                    fs::Permissions::from_mode(0o755),
                )
                .expect("make bundled executable executable");
            }
        }
    }

    fn file_snapshot(root: &Path) -> BTreeMap<PathBuf, Vec<u8>> {
        fn visit(root: &Path, path: &Path, files: &mut BTreeMap<PathBuf, Vec<u8>>) {
            for entry in fs::read_dir(path).expect("read snapshot directory") {
                let entry = entry.expect("read snapshot entry");
                let metadata = fs::symlink_metadata(entry.path()).expect("read snapshot metadata");
                assert!(
                    !metadata.file_type().is_symlink(),
                    "fixture has no symlinks"
                );
                if metadata.is_dir() {
                    visit(root, &entry.path(), files);
                } else {
                    assert!(metadata.is_file(), "fixture has only regular files");
                    files.insert(
                        entry
                            .path()
                            .strip_prefix(root)
                            .expect("snapshot path is beneath root")
                            .to_path_buf(),
                        fs::read(entry.path()).expect("read snapshot file"),
                    );
                }
            }
        }

        let mut files = BTreeMap::new();
        visit(root, root, &mut files);
        files
    }

    #[test]
    fn blake3_digest_has_one_canonical_wire_format() {
        let digest = Blake3Digest::from_hash(blake3::hash(b"abc"));
        let encoded = serde_json::to_string(&digest).expect("encode digest");
        assert_eq!(
            encoded,
            "\"blake3:6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85\""
        );
        assert_eq!(
            serde_json::from_str::<Blake3Digest>(&encoded).expect("decode digest"),
            digest
        );
        assert!(
            serde_json::from_str::<Blake3Digest>(
                "\"6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85\""
            )
            .is_err()
        );
        assert!(
            serde_json::from_str::<Blake3Digest>(
                "\"blake3:6437B3AC38465133FFB63B75273A8DB548C558465D79DB03FD359C6CD5BD9D85\""
            )
            .is_err()
        );
    }

    #[test]
    fn imports_persistent_state_without_live_or_legacy_binaries() {
        let root = fixture_root("runtime-migration");
        let source = root.join("legacy");
        let product = AosHome::from_root(root.join("product"));
        write(&source, "keys/runtime.key", b"runtime-key");
        write(&source, "secrets/provider", b"provider-secret");
        write(&source, "var/state.db", b"state");
        write(&source, "wit/store/contracts.wit", b"wit");
        write(&source, "home/alice/.local/audit/chain", b"audit");
        write(&source, "lib/shared.wasm", b"shared-component");
        write(&source, "trust/unicity-ce.pub", b"distro-key");
        write(&source, "capsules/system/meta.toml", b"system-capsule");
        write(&source, "history", b"aos status\n");
        write(&source, "config.toml", b"[security]\nstrict = true\n");
        write(&source, "etc/config.toml", b"[runtime]\n");
        write(&source, "etc/servers.toml", b"[servers]\n");
        write(&source, "etc/gateway.toml", b"[gateway]\n");
        write(&source, "etc/gateway-http.toml", b"enabled = false\n");
        write(&source, "etc/layout-version", b"1");
        write(&source, "etc/groups.toml", b"[groups]\n");
        write(&source, "etc/invites.toml", b"[invites]\n");
        write(&source, "etc/pair-tokens.toml", b"[tokens]\n");
        write(&source, "etc/gateway-revocations.json", b"[]");
        write(&source, "etc/profiles/alice.toml", b"enabled = true\n");
        write(&source, "etc/hooks/audit.toml", b"enabled = true\n");
        write(&source, "bin/capsule.wasm", b"wasm");
        write(&source, "bin/astrid", b"legacy-binary");
        write(&source, "run/ready", b"live-state");
        write(&source, "log/daemon.log", b"log");
        install_product_runtime(&product);

        assert_eq!(
            migrate_runtime(&product, &source).expect("migration succeeds"),
            MigrationOutcome::Migrated
        );
        let runtime = product.runtime_home();
        assert_eq!(
            fs::read(runtime.join("keys/runtime.key")).unwrap(),
            b"runtime-key"
        );
        assert_eq!(
            fs::read(runtime.join("secrets/provider")).unwrap(),
            b"provider-secret"
        );
        assert_eq!(fs::read(runtime.join("bin/capsule.wasm")).unwrap(), b"wasm");
        assert_eq!(
            fs::read(runtime.join("etc/profiles/alice.toml")).unwrap(),
            b"enabled = true\n"
        );
        assert_eq!(
            fs::read(runtime.join("etc/groups.toml")).unwrap(),
            b"[groups]\n"
        );
        assert_eq!(
            fs::read(runtime.join("trust/unicity-ce.pub")).unwrap(),
            b"distro-key"
        );
        assert_eq!(fs::read(runtime.join("history")).unwrap(), b"aos status\n");
        assert_eq!(
            fs::read(runtime.join("config.toml")).unwrap(),
            b"[security]\nstrict = true\n"
        );
        assert_eq!(
            fs::read(runtime.join("bin/astrid")).unwrap(),
            b"bundled-astrid"
        );
        for name in crate::RUNTIME_EXECUTABLE_NAMES {
            assert_eq!(
                fs::read(runtime.join("bin").join(name)).unwrap(),
                format!("bundled-{name}").as_bytes(),
                "the supported product installer executable set must survive migration"
            );
        }
        assert!(!runtime.join("run").exists());
        assert_eq!(fs::read(runtime.join("log/daemon.log")).unwrap(), b"log");
        assert_eq!(
            fs::read(source.join("keys/runtime.key")).unwrap(),
            b"runtime-key"
        );
        assert!(
            product
                .root()
                .join("migrations/astrid-home-v1.json")
                .is_file()
        );
        let receipt: serde_json::Value = serde_json::from_slice(
            &fs::read(product.migration_receipt()).expect("read migration receipt"),
        )
        .expect("decode migration receipt");
        assert_eq!(receipt["migration_version"], MIGRATION_VERSION);
        assert_eq!(receipt["schema_version"], RECEIPT_SCHEMA_VERSION);
        assert!(receipt["entries"].as_array().is_some_and(|entries| {
            entries.iter().all(|entry| {
                entry["digest"].as_str().is_some_and(|digest| {
                    digest.len() == 71
                        && digest.starts_with("blake3:")
                        && digest[7..]
                            .bytes()
                            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
                }) && entry.get("sha256").is_none()
            })
        }));
        assert_eq!(
            migrate_runtime(&product, &source).expect("matching migration is idempotent"),
            MigrationOutcome::AlreadyMigrated
        );
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[cfg(unix)]
    #[test]
    fn imports_the_frozen_2026_07_15_094_home_shape_without_loss_and_self_heals_modes() {
        use std::os::unix::fs::PermissionsExt;

        let root = fixture_root("runtime-production-shape");
        let source = root.join("legacy");
        let product = AosHome::from_root(root.join("product"));

        for index in 0..63 {
            write(
                &source,
                &format!("bin/component-{index:02}.wasm"),
                format!("wasm-{index:02}").as_bytes(),
            );
        }
        write(&source, "etc/layout-version", b"1");
        write(&source, "etc/groups.toml", b"[groups]\n");
        for index in 0..7 {
            write(
                &source,
                &format!("etc/profiles/principal-{index}.toml"),
                format!("enabled = true\nindex = {index}\n").as_bytes(),
            );
        }
        fs::create_dir_all(source.join("etc/hooks")).expect("create empty hooks directory");
        write(
            &source,
            "home/alice/.config/distro.lock",
            b"[distro]\nid = \"astralis\"\nversion = \"0.2.2\"\n",
        );
        write(
            &source,
            "home/bob/.config/distro.lock",
            b"[distro]\nid = \"aos-ce\"\nversion = \"2026.1.0\"\n",
        );
        write(
            &source,
            "home/carol/.config/distro.lock",
            b"[distro]\nid = \"unicity-ce\"\nversion = \"2026.1.0\"\n",
        );
        write(
            &source,
            "home/dan/.config/distro.lock",
            b"[distro]\nid = \"other\"\nversion = \"1.0.0\"\n",
        );
        for index in 0..140 {
            write(
                &source,
                &format!("home/alice/.local/capsules/asset-{index:03}"),
                format!("capsule-payload-or-meta-{index:03}").as_bytes(),
            );
        }
        for index in 0..4 {
            write(
                &source,
                &format!("home/alice/.local/audit/record-{index:03}"),
                format!("audit-{index:03}").as_bytes(),
            );
        }
        for index in 0..26 {
            write(
                &source,
                &format!("home/alice/.config/env/override-{index:03}"),
                format!("ENV_{index:03}=preserved").as_bytes(),
            );
        }
        for index in 0..196 {
            write(
                &source,
                &format!("home/alice/.local/state/item-{index:03}"),
                format!("principal-state-{index:03}").as_bytes(),
            );
        }
        write(&source, "keys/runtime.key", b"runtime-key-material");
        for index in 0..7 {
            write(
                &source,
                &format!("keys/device-{index}.key"),
                format!("device-key-material-{index}").as_bytes(),
            );
        }
        write(&source, "secrets/providers.toml", b"token = \"secret\"\n");
        for index in 0..9 {
            write(
                &source,
                &format!("var/state-{index}"),
                format!("state-{index}").as_bytes(),
            );
        }
        for index in 0..11 {
            write(
                &source,
                &format!("wit/contract-{index}.wit"),
                format!("package fixture:contract{index};\n").as_bytes(),
            );
        }
        for index in 0..7 {
            write(
                &source,
                &format!("log/runtime-{index}.log"),
                format!("log-{index}").as_bytes(),
            );
        }
        for (path, bytes) in [
            ("run/.hud-health", b"stale HUD health".as_slice()),
            ("run/session.principal", b"transient-principal".as_slice()),
            ("run/system.lock", b"daemon-lock".as_slice()),
            ("run/system.pid", b"12345".as_slice()),
            ("run/system.token", b"ephemeral-credential".as_slice()),
        ] {
            write(&source, path, bytes);
        }
        for directory in ["bin", "etc", "home", "keys", "log", "run", "var", "wit"] {
            fs::set_permissions(source.join(directory), fs::Permissions::from_mode(0o700))
                .expect("set private source directory mode");
        }
        fs::set_permissions(&source, fs::Permissions::from_mode(0o700))
            .expect("set private source root mode");
        fs::set_permissions(source.join("secrets"), fs::Permissions::from_mode(0o755))
            .expect("model the legacy permissive secrets mode");
        fs::set_permissions(
            source.join("home/alice/.local/state/item-000"),
            fs::Permissions::from_mode(0o755),
        )
        .expect("model an executable private helper");
        install_product_runtime(&product);

        let source_before = file_snapshot(&source);
        assert_eq!(
            source_before.len(),
            483,
            "fixture tracks the frozen 2026-07-15 Astrid 0.9.4 home shape"
        );
        let expected_counts = [
            ("bin", 63),
            ("etc", 9),
            ("home", 370),
            ("keys", 8),
            ("log", 7),
            ("run", 5),
            ("secrets", 1),
            ("var", 9),
            ("wit", 11),
        ];
        for (top_level, expected) in expected_counts {
            assert_eq!(
                source_before
                    .keys()
                    .filter(|path| path.starts_with(top_level))
                    .count(),
                expected,
                "frozen fixture count changed for {top_level}"
            );
        }
        assert_eq!(
            source_before
                .keys()
                .filter(|path| path.starts_with("bin"))
                .count(),
            63
        );
        assert!(
            source_before
                .keys()
                .filter(|path| path.starts_with("bin"))
                .all(|path| path
                    .extension()
                    .is_some_and(|extension| extension == "wasm")),
            "the installed 0.9.4 bin shape contains WASM components only"
        );
        for name in crate::RUNTIME_EXECUTABLE_NAMES {
            assert!(
                !source.join("bin").join(name).exists(),
                "the installed 0.9.4 home has no managed {name} executable"
            );
        }
        assert_eq!(
            source_before
                .keys()
                .filter(|path| path.starts_with("etc"))
                .count(),
            9
        );
        assert!(source.join("etc/hooks").is_dir());
        assert!(
            fs::read_dir(source.join("etc/hooks"))
                .expect("read empty hooks directory")
                .next()
                .is_none()
        );

        assert_eq!(
            migrate_runtime(&product, &source).expect("production-shaped migration succeeds"),
            MigrationOutcome::Migrated
        );
        assert_eq!(
            file_snapshot(&source),
            source_before,
            "migration never mutates its source"
        );

        let runtime = product.runtime_home();
        for (relative, bytes) in &source_before {
            let persistent = matches!(
                relative.components().next(),
                Some(Component::Normal(name))
                    if ["etc", "home", "keys", "secrets", "var", "wit", "log"]
                        .iter()
                        .any(|expected| name == OsStr::new(expected))
            );
            let wasm = relative.starts_with("bin")
                && relative
                    .extension()
                    .is_some_and(|extension| extension == "wasm");
            if persistent || wasm {
                assert_eq!(
                    fs::read(runtime.join(relative)).expect("read migrated persistent file"),
                    *bytes,
                    "migrated bytes differ for {}",
                    relative.display()
                );
            }
        }
        assert!(!runtime.join("run").exists());
        for transient in [
            ".hud-health",
            "session.principal",
            "system.lock",
            "system.pid",
            "system.token",
        ] {
            assert!(source.join("run").join(transient).is_file());
            assert!(!runtime.join("run").join(transient).exists());
        }
        for representative in [
            "keys/runtime.key",
            "home/alice/.config/distro.lock",
            "home/dan/.config/distro.lock",
            "home/alice/.local/capsules/asset-139",
            "home/alice/.local/audit/record-003",
            "home/alice/.config/env/override-025",
            "var/state-8",
            "wit/contract-10.wit",
            "etc/profiles/principal-6.toml",
            "log/runtime-6.log",
        ] {
            assert_eq!(
                fs::read(runtime.join(representative)).unwrap(),
                fs::read(source.join(representative)).unwrap(),
                "representative live state was not preserved: {representative}"
            );
        }
        for name in crate::RUNTIME_EXECUTABLE_NAMES {
            let path = runtime.join("bin").join(name);
            assert_eq!(
                fs::read(&path).unwrap(),
                format!("bundled-{name}").as_bytes()
            );
            assert_eq!(
                fs::metadata(path).unwrap().permissions().mode() & 0o777,
                0o755
            );
        }
        for directory in ["etc", "home", "keys", "secrets", "var", "wit", "log"] {
            assert_eq!(
                fs::metadata(runtime.join(directory))
                    .unwrap()
                    .permissions()
                    .mode()
                    & 0o777,
                0o700,
                "private directory mode was not tightened for {directory}"
            );
        }
        assert_eq!(
            fs::metadata(source.join("home/alice/.local/state/item-000"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o755,
            "migration must not mutate the source mode"
        );
        assert_eq!(
            fs::metadata(&source).unwrap().permissions().mode() & 0o777,
            0o700,
            "the frozen standalone home root remains private"
        );
        assert_eq!(
            fs::metadata(runtime.join("home/alice/.local/state/item-000"))
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o700,
            "private executability must survive without group or world access"
        );
        let receipt_before = fs::read(product.migration_receipt()).expect("read receipt");
        let runtime_before = file_snapshot(&runtime);
        assert!(!product.migration_receipt().with_extension("tmp").exists());
        assert!(!product.root().join(".runtime-import").exists());
        assert_eq!(
            migrate_runtime(&product, &source).expect("idempotent migration succeeds"),
            MigrationOutcome::AlreadyMigrated
        );
        assert_eq!(file_snapshot(&runtime), runtime_before);
        assert_eq!(
            fs::read(product.migration_receipt()).unwrap(),
            receipt_before
        );
        assert_eq!(file_snapshot(&source), source_before);
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[test]
    fn excludes_stale_readiness_and_deferred_coordination_from_a_stopped_runtime() {
        let root = fixture_root("stale-runtime-coordination");
        let source = root.join("legacy");
        let product = AosHome::from_root(root.join("product"));
        write(&source, "keys/runtime.key", b"runtime-key");
        write(&source, "run/system.ready", b"stale-ready");
        write(&source, "run/deferred-requests.json", b"[\"stale\"]");
        install_product_runtime(&product);
        let source_before = file_snapshot(&source);

        assert_eq!(
            migrate_runtime(&product, &source).expect("stale coordination is self-healed"),
            MigrationOutcome::Migrated
        );
        assert_eq!(file_snapshot(&source), source_before);
        assert!(!product.runtime_home().join("run").exists());
        assert_eq!(
            fs::read(product.runtime_home().join("keys/runtime.key")).unwrap(),
            b"runtime-key"
        );
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[test]
    fn refuses_import_while_the_standalone_system_socket_is_present() {
        let root = fixture_root("live-runtime-socket");
        let source = root.join("legacy");
        let product = AosHome::from_root(root.join("product"));
        write(&source, "keys/runtime.key", b"runtime-key");
        write(&source, "run/system.sock", b"live-socket-placeholder");
        install_product_runtime(&product);
        let source_before = file_snapshot(&source);

        let error = migrate_runtime(&product, &source)
            .expect_err("a present system socket must require an explicit runtime stop");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("stop the standalone runtime"));
        assert_eq!(file_snapshot(&source), source_before);
        assert!(!product.migration_receipt().exists());
        assert!(!product.runtime_home().join("keys/runtime.key").exists());
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[test]
    fn refuses_to_merge_into_existing_runtime_state() {
        let root = fixture_root("runtime-existing-target");
        let source = root.join("legacy");
        let product = AosHome::from_root(root.join("product"));
        write(&source, "keys/runtime.key", b"runtime-key");
        install_product_runtime(&product);
        write(&product.runtime_home(), "var/state.db", b"existing-state");

        let error =
            migrate_runtime(&product, &source).expect_err("existing runtime state must not merge");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(
            fs::read(product.runtime_home().join("var/state.db")).unwrap(),
            b"existing-state"
        );
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[test]
    fn records_legacy_distro_locks_without_rewriting_them() {
        let root = fixture_root("legacy-distros");
        let source = root.join("legacy");
        let product = AosHome::from_root(root.join("product"));
        write(&source, "keys/runtime.key", b"runtime-key");
        install_product_runtime(&product);
        let astralis = b"[distro]\nid = \"astralis\"\nversion = \"0.2.2\"\n";
        let aos_ce = b"[distro]\nid = \"aos-ce\"\nversion = \"2026.1.0\"\n";
        write(&source, "home/alice/.config/distro.lock", astralis);
        write(&source, "home/bob/.config/distro.lock", aos_ce);
        write(
            &source,
            "home/carol/.config/distro.lock",
            b"[distro]\nid = \"unicity-ce\"\nversion = \"2026.1.0\"\n",
        );
        write(&source, "home/dan/.config/distro.lock", b"not toml");

        migrate_runtime(&product, &source).expect("migration succeeds");
        assert_eq!(
            product.imported_legacy_distros().expect("read receipt"),
            vec![
                super::LegacyDistro {
                    principal: "alice".into(),
                    id: "astralis".into(),
                    version: "0.2.2".into()
                },
                super::LegacyDistro {
                    principal: "bob".into(),
                    id: "aos-ce".into(),
                    version: "2026.1.0".into()
                },
            ]
        );
        assert_eq!(
            fs::read(
                product
                    .runtime_home()
                    .join("home/alice/.config/distro.lock")
            )
            .unwrap(),
            astralis
        );
        assert_eq!(
            fs::read(product.runtime_home().join("home/bob/.config/distro.lock")).unwrap(),
            aos_ce
        );
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[test]
    fn rejects_an_unversioned_unhashed_receipt() {
        let root = fixture_root("legacy-receipt");
        let source = root.join("legacy");
        let product = AosHome::from_root(root.join("product"));
        write(&source, "keys/runtime.key", b"runtime-key");
        write(&product.runtime_home(), "keys/runtime.key", b"runtime-key");

        let source = source.canonicalize().expect("canonical source");
        let receipt = serde_json::json!({
            "source": source,
            "entries": [{ "path": "keys/runtime.key", "bytes": 11 }],
        });
        let receipt_path = product.migration_receipt();
        fs::create_dir_all(receipt_path.parent().expect("receipt parent"))
            .expect("create receipt parent");
        fs::write(
            &receipt_path,
            serde_json::to_vec(&receipt).expect("serialize legacy receipt"),
        )
        .expect("write legacy receipt");

        let error = migrate_runtime(&product, &source)
            .expect_err("unversioned receipt must not bypass integrity validation");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[test]
    fn rejects_a_pre_release_sha256_receipt_without_blessing_the_target() {
        let root = fixture_root("sha256-receipt");
        let source = root.join("legacy");
        let product = AosHome::from_root(root.join("product"));
        write(&source, "keys/runtime.key", b"runtime-key");
        write(
            &product.runtime_home(),
            "keys/runtime.key",
            b"possibly-tampered",
        );

        let source = source.canonicalize().expect("canonical source");
        let receipt = serde_json::json!({
            "migration_version": MIGRATION_VERSION,
            "schema_version": 2,
            "source": source,
            "entries": [{
                "path": "keys/runtime.key",
                "bytes": 17,
                "sha256": "0".repeat(64),
            }],
        });
        let receipt_path = product.migration_receipt();
        fs::create_dir_all(receipt_path.parent().expect("receipt parent"))
            .expect("create receipt parent");
        fs::write(
            &receipt_path,
            serde_json::to_vec(&receipt).expect("serialize SHA-256 receipt"),
        )
        .expect("write SHA-256 receipt");

        let error = migrate_runtime(&product, &source)
            .expect_err("SHA-256 receipt must be rejected before trusting its target");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(
            fs::read(product.runtime_home().join("keys/runtime.key")).unwrap(),
            b"possibly-tampered"
        );
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[test]
    fn rejects_unknown_configuration_instead_of_silently_dropping_it() {
        let root = fixture_root("unknown-runtime-config");
        let source = root.join("legacy");
        let product = AosHome::from_root(root.join("product"));
        write(&source, "keys/runtime.key", b"runtime-key");
        write(&source, "etc/future-policy.toml", b"deny = true\n");
        install_product_runtime(&product);

        let error = migrate_runtime(&product, &source)
            .expect_err("unknown authorization state must stop migration");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("etc/future-policy.toml"));
        assert!(!product.runtime_home().join("keys/runtime.key").exists());
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[test]
    fn rejects_unknown_top_level_state_instead_of_silently_dropping_it() {
        let root = fixture_root("unknown-runtime-state");
        let source = root.join("legacy");
        let product = AosHome::from_root(root.join("product"));
        write(&source, "keys/runtime.key", b"runtime-key");
        write(&source, "future-store/state", b"important");
        install_product_runtime(&product);

        let error = migrate_runtime(&product, &source)
            .expect_err("unknown persistent state must stop migration");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("future-store"));
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[test]
    fn rejects_unknown_bin_state_instead_of_silently_dropping_it() {
        let root = fixture_root("unknown-runtime-bin-state");
        let source = root.join("legacy");
        let product = AosHome::from_root(root.join("product"));
        write(&source, "keys/runtime.key", b"runtime-key");
        write(&source, "bin/future-index", b"durable-index");
        install_product_runtime(&product);

        let error = migrate_runtime(&product, &source)
            .expect_err("unknown bin state must stop migration instead of being omitted");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(error.to_string().contains("bin contains unsupported state"));
        assert_eq!(
            fs::read(source.join("bin/future-index")).unwrap(),
            b"durable-index"
        );
        assert!(!product.runtime_home().join("keys/runtime.key").exists());
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[test]
    fn detects_same_length_content_tampering() {
        let root = fixture_root("runtime-content-tamper");
        let source = root.join("legacy");
        let product = AosHome::from_root(root.join("product"));
        write(&source, "keys/runtime.key", b"runtime-key");
        install_product_runtime(&product);
        migrate_runtime(&product, &source).expect("migration succeeds");

        write(&product.runtime_home(), "keys/runtime.key", b"tampered-ke");
        let error = migrate_runtime(&product, &source)
            .expect_err("same-length tampering must invalidate the receipt");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[test]
    fn detects_bundled_runtime_executable_tampering() {
        let root = fixture_root("runtime-binary-tamper");
        let source = root.join("legacy");
        let product = AosHome::from_root(root.join("product"));
        write(&source, "keys/runtime.key", b"runtime-key");
        install_product_runtime(&product);
        migrate_runtime(&product, &source).expect("migration succeeds");

        write(&product.runtime_home(), "bin/astrid", b"tamperd-binary");
        let error = migrate_runtime(&product, &source)
            .expect_err("runtime executable tampering must invalidate the receipt");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[test]
    fn rejects_a_lexical_alias_of_the_product_runtime_as_source() {
        let root = fixture_root("runtime-source-alias");
        fs::create_dir_all(root.join("alias")).expect("create alias path component");
        let product = AosHome::from_root(root.join("alias/../product"));
        install_product_runtime(&product);
        let source = root.join("product/runtime");

        let error = migrate_runtime(&product, &source)
            .expect_err("the product runtime cannot be imported through a path alias");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert_eq!(
            fs::read(source.join("bin/astrid")).unwrap(),
            b"bundled-astrid"
        );
        assert!(!product.migration_receipt().exists());
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[test]
    fn rejects_overlapping_roots_without_modifying_the_source() {
        let root = fixture_root("runtime-overlapping-roots");
        let source = root.join("legacy");
        let product = AosHome::from_root(source.join("product"));
        write(&source, "keys/runtime.key", b"runtime-key");
        install_product_runtime(&product);

        let error = migrate_runtime(&product, &source)
            .expect_err("product state must not be created beneath the source");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(!product.root().join("migrations").exists());
        assert!(!product.root().join(".runtime-import").exists());
        assert_eq!(
            fs::read(source.join("keys/runtime.key")).unwrap(),
            b"runtime-key"
        );
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[test]
    fn serializes_concurrent_runtime_migrations() {
        let root = fixture_root("runtime-concurrent");
        let source = root.join("legacy");
        let product = AosHome::from_root(root.join("product"));
        write(&source, "keys/runtime.key", b"runtime-key");
        install_product_runtime(&product);
        create_private_dir(&product.root().join("migrations")).expect("create migrations dir");
        let held = MigrationLock::acquire(&product).expect("hold migration lock");

        let error = migrate_runtime(&product, &source)
            .expect_err("a concurrent migration must fail before staging");
        assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
        assert!(!product.root().join(".runtime-import").exists());
        assert!(!product.migration_receipt().exists());
        assert_eq!(
            fs::read(product.runtime_home().join("bin/astrid")).unwrap(),
            b"bundled-astrid"
        );
        drop(held);
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[test]
    fn completes_an_interrupted_validated_cutover() {
        let root = fixture_root("runtime-cutover-complete");
        let source = root.join("legacy");
        let product = AosHome::from_root(root.join("product"));
        write(&source, "keys/runtime.key", b"runtime-key");
        install_product_runtime(&product);
        migrate_runtime(&product, &source).expect("migration succeeds");

        let receipt = product.migration_receipt();
        let staged_receipt = receipt.with_extension("tmp");
        fs::rename(&receipt, &staged_receipt).expect("stage committed receipt");
        write(
            product.root(),
            "runtime.pre-migration/bin/astrid",
            b"bundled-astrid",
        );
        let canonical_source = source.canonicalize().expect("canonical source");

        assert_eq!(
            migrate_runtime(&product, &canonical_source).expect("recover valid cutover"),
            MigrationOutcome::AlreadyMigrated
        );
        assert!(receipt.is_file());
        assert!(!staged_receipt.exists());
        assert!(!product.root().join("runtime.pre-migration").exists());
        assert_eq!(
            fs::read(product.runtime_home().join("keys/runtime.key")).unwrap(),
            b"runtime-key"
        );
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[test]
    fn rolls_back_an_interrupted_unvalidated_cutover() {
        let root = fixture_root("runtime-cutover-rollback");
        let source = root.join("legacy");
        let product = AosHome::from_root(root.join("product"));
        write(&source, "keys/runtime.key", b"runtime-key");
        install_product_runtime(&product);
        let target = product.runtime_home();
        let backup = product.root().join("runtime.pre-migration");
        fs::rename(&target, &backup).expect("move original to transaction backup");
        write(&target, "bin/astrid", b"partial-binary");
        write(
            product.root(),
            "migrations/astrid-home-v1.tmp",
            b"invalid receipt",
        );
        let canonical_source = source.canonicalize().expect("canonical source");

        recover_interrupted_transaction(
            &product,
            &target,
            &canonical_source,
            &product.migration_receipt(),
        )
        .expect("roll back invalid cutover");

        assert_eq!(
            fs::read(target.join("bin/astrid")).unwrap(),
            b"bundled-astrid"
        );
        assert!(!backup.exists());
        assert!(!product.migration_receipt().with_extension("tmp").exists());
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[test]
    fn migration_retry_recovers_a_partial_target_and_finishes_atomically() {
        let root = fixture_root("runtime-partial-target-retry");
        let source = root.join("legacy");
        let product = AosHome::from_root(root.join("product"));
        write(&source, "keys/runtime.key", b"runtime-key");
        write(&source, "secrets/provider", b"provider-secret");
        install_product_runtime(&product);
        let source_before = file_snapshot(&source);

        let target = product.runtime_home();
        let backup = product.root().join("runtime.pre-migration");
        fs::rename(&target, &backup).expect("move product runtime into transaction backup");
        write(&target, "bin/astrid", b"partial-cutover");
        write(
            product.root(),
            "migrations/astrid-home-v1.tmp",
            b"interrupted receipt",
        );

        assert_eq!(
            migrate_runtime(&product, &source).expect("retry self-heals and completes"),
            MigrationOutcome::Migrated
        );
        assert_eq!(file_snapshot(&source), source_before);
        assert_eq!(
            fs::read(product.runtime_home().join("keys/runtime.key")).unwrap(),
            b"runtime-key"
        );
        assert_eq!(
            fs::read(product.runtime_home().join("secrets/provider")).unwrap(),
            b"provider-secret"
        );
        for name in crate::RUNTIME_EXECUTABLE_NAMES {
            assert_eq!(
                fs::read(product.runtime_home().join("bin").join(name)).unwrap(),
                format!("bundled-{name}").as_bytes()
            );
        }
        assert!(product.migration_receipt().is_file());
        assert!(!product.migration_receipt().with_extension("tmp").exists());
        assert!(!backup.exists());
        assert!(!product.root().join("runtime.failed-migration").exists());
        assert_eq!(
            migrate_runtime(&product, &source).expect("recovered migration is idempotent"),
            MigrationOutcome::AlreadyMigrated
        );
        assert_eq!(file_snapshot(&source), source_before);
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[test]
    fn migration_retry_discards_uncommitted_staging_without_duplication() {
        let root = fixture_root("runtime-staging-retry");
        let source = root.join("legacy");
        let product = AosHome::from_root(root.join("product"));
        write(&source, "keys/runtime.key", b"runtime-key");
        install_product_runtime(&product);
        let source_before = file_snapshot(&source);
        write(
            product.root(),
            ".runtime-import/keys/runtime.key",
            b"partial-copy",
        );
        write(
            product.root(),
            "migrations/astrid-home-v1.tmp",
            b"partial-receipt",
        );

        assert_eq!(
            migrate_runtime(&product, &source).expect("retry replaces uncommitted staging"),
            MigrationOutcome::Migrated
        );
        assert_eq!(file_snapshot(&source), source_before);
        assert_eq!(
            fs::read(product.runtime_home().join("keys/runtime.key")).unwrap(),
            b"runtime-key"
        );
        assert!(!product.root().join(".runtime-import").exists());
        assert!(!product.migration_receipt().with_extension("tmp").exists());
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[cfg(unix)]
    #[test]
    fn refuses_symlinked_legacy_content() {
        use std::os::unix::fs::symlink;

        let root = fixture_root("runtime-symlink");
        let source = root.join("legacy");
        let product = AosHome::from_root(root.join("product"));
        write(&source, "outside-key", b"runtime-key");
        fs::create_dir_all(source.join("keys")).expect("create keys directory");
        symlink(source.join("outside-key"), source.join("keys/runtime.key"))
            .expect("create legacy symlink");
        install_product_runtime(&product);

        let error = migrate_runtime(&product, &source).expect_err("symlink must be rejected");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(!product.runtime_home().join("keys/runtime.key").exists());
        assert_eq!(
            fs::read(product.runtime_home().join("bin/astrid")).unwrap(),
            b"bundled-astrid"
        );
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlinked_bundled_runtime_executable() {
        use std::os::unix::fs::symlink;

        let root = fixture_root("runtime-binary-symlink");
        let source = root.join("legacy");
        let product = AosHome::from_root(root.join("product"));
        write(&source, "keys/runtime.key", b"runtime-key");
        install_product_runtime(&product);
        fs::remove_file(product.runtime_home().join("bin/astrid"))
            .expect("remove regular executable");
        write(&root, "runtime-target", b"bundled-astrid");
        symlink(
            root.join("runtime-target"),
            product.runtime_home().join("bin/astrid"),
        )
        .expect("create bundled binary symlink");

        let error =
            migrate_runtime(&product, &source).expect_err("binary symlink must be rejected");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(!product.runtime_home().join("keys/runtime.key").exists());
        fs::remove_dir_all(root).expect("remove fixture root");
    }

    #[cfg(unix)]
    #[test]
    fn refuses_a_symlinked_product_runtime_home() {
        use std::os::unix::fs::symlink;

        let root = fixture_root("runtime-home-symlink");
        let source = root.join("legacy");
        let product = AosHome::from_root(root.join("product"));
        let external_runtime = root.join("external-runtime");
        write(&source, "keys/runtime.key", b"runtime-key");
        write(&external_runtime, "bin/astrid", b"bundled-binary");
        fs::create_dir_all(product.root()).expect("create product root");
        symlink(&external_runtime, product.runtime_home()).expect("symlink product runtime home");

        let error = migrate_runtime(&product, &source).expect_err("runtime home symlink must fail");
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
        assert!(!external_runtime.join("keys/runtime.key").exists());
        fs::remove_dir_all(root).expect("remove fixture root");
    }
}
