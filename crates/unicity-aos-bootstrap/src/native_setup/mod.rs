//! Local-personal native-input first-run setup.
//!
//! Pairing enrolls a dedicated device; it does not select a native responder
//! or grant hosted acting authority. This module writes only the tray
//! connection file and a missing `[native_input.responders]` binding.
//!
//! Failed setup removes only files this attempt created. Preexisting key
//! artifacts and connections are refused, not overwritten or deleted. A
//! successful `pair-device redeem` is not undone; this path does not call
//! `pair-device revoke` and does not provide transactional daemon rollback.

use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};

use astrid_core::{DeviceKeyId, PrincipalId};
use fs2::FileExt;
use serde::Serialize;

use unicity_aos_bootstrap::AosHome;

pub(crate) const DEVICE_NAME: &str = "aos-tray";
pub(crate) const UNSUPPORTED_MESSAGE: &str = "native-input setup is not supported by this runtime";

const CONNECTION_CAPACITY: i32 = 8;
const INPUT_TIMEOUT_SECONDS: f64 = 120.0;
const IO_TIMEOUT_SECONDS: f64 = 5.0;
const READ_TIMEOUT_SECONDS: f64 = 3600.0;
const SETUP_LOCK_FILE: &str = "setup.lock";

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SetupDocument {
    pub scope: &'static str,
    pub authority: &'static str,
    pub principal: String,
    pub connection_path: String,
    pub restart_required: bool,
    pub connected: bool,
}

impl SetupDocument {
    fn local(principal: &PrincipalId, connection_path: &Path) -> Self {
        Self {
            scope: "local-personal",
            authority: "setup",
            principal: principal.to_string(),
            connection_path: connection_path.display().to_string(),
            restart_required: true,
            connected: false,
        }
    }
}

#[derive(Debug)]
pub(crate) enum SetupError {
    Unsupported,
    Failed(String),
}

struct SetupLock {
    _file: File,
}

impl Drop for SetupLock {
    fn drop(&mut self) {
        // Release at the operation boundary, not when the last duplicate file
        // description closes (a concurrent fork may retain one until exec).
        if let Err(error) = FileExt::unlock(&self._file) {
            eprintln!("warning: failed to release native-input setup lock: {error}");
        }
    }
}

struct KeyArtifacts {
    private: PathBuf,
    public_hex: PathBuf,
    meta: PathBuf,
}

struct KeyPresence {
    private: bool,
    public_hex: bool,
    meta: bool,
}

#[derive(Default)]
struct OwnedCleanup {
    connection: Option<PathBuf>,
    private: Option<PathBuf>,
    public_hex: Option<PathBuf>,
    meta: Option<PathBuf>,
}

/// Resolve `AOS_HOME` to a real directory spelling before any pairing mutation.
///
/// Longest existing prefixes are canonicalized so macOS `/tmp` and `/var`
/// aliases match the tray loader's `O_NOFOLLOW` walk. Missing suffix names are
/// appended without creating them. An existing prefix that cannot be
/// canonicalized fails closed and is not skipped.
pub(crate) fn canonical_setup_home(root: &Path) -> Result<PathBuf, SetupError> {
    if !root.is_absolute() {
        return Err(SetupError::Failed(
            "AOS_HOME must be an absolute path".to_owned(),
        ));
    }
    if root.as_os_str().as_encoded_bytes().contains(&0) {
        return Err(SetupError::Failed(
            "AOS_HOME must not contain a NUL byte".to_owned(),
        ));
    }
    // Path::components() skips '.' except at the start; inspect raw names so
    // `/tmp/foo/./nested` cannot normalize into a pairing home.
    for (index, part) in root
        .as_os_str()
        .as_bytes()
        .split(|byte| *byte == b'/')
        .enumerate()
    {
        if part.is_empty() {
            if index == 0 {
                continue;
            }
            return Err(SetupError::Failed(
                "AOS_HOME must not contain empty path components".to_owned(),
            ));
        }
        if part == b"." || part == b".." {
            return Err(SetupError::Failed(
                "AOS_HOME must not contain '.' or '..' path components".to_owned(),
            ));
        }
    }

    let mut current = root.to_path_buf();
    let mut missing = Vec::new();
    loop {
        match fs::symlink_metadata(&current) {
            Ok(_) => {
                let canonical = fs::canonicalize(&current).map_err(|error| {
                    SetupError::Failed(format!("AOS_HOME cannot be resolved: {error}"))
                })?;
                let metadata = fs::metadata(&canonical).map_err(|error| {
                    SetupError::Failed(format!("AOS_HOME is not a directory: {error}"))
                })?;
                if !metadata.is_dir() {
                    return Err(SetupError::Failed(
                        "AOS_HOME must resolve to a directory".to_owned(),
                    ));
                }
                let mut resolved = canonical;
                for name in missing.into_iter().rev() {
                    resolved.push(name);
                }
                return Ok(resolved);
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                let Some(name) = current.file_name() else {
                    return Err(SetupError::Failed("AOS_HOME cannot be resolved".to_owned()));
                };
                missing.push(name.to_os_string());
                if !current.pop() {
                    return Err(SetupError::Failed("AOS_HOME cannot be resolved".to_owned()));
                }
            }
            Err(error) => return Err(SetupError::Failed(error.to_string())),
        }
    }
}

pub(crate) fn connection_path(home: &AosHome) -> PathBuf {
    home.root().join("native-input/connection.json")
}

pub(crate) fn operator_config_path(home: &AosHome) -> PathBuf {
    home.runtime_home().join("config.toml")
}

pub(crate) fn private_key_path(home: &AosHome) -> PathBuf {
    home.runtime_home()
        .join("keys/local")
        .join(format!("{DEVICE_NAME}.ed25519"))
}

pub(crate) fn socket_path(home: &AosHome) -> PathBuf {
    home.run_root().join("system.sock")
}

pub(crate) fn token_path(home: &AosHome) -> PathBuf {
    home.run_root().join("system.token")
}

pub(crate) fn runtime_generate_args(principal: &PrincipalId) -> Vec<OsString> {
    vec![
        OsString::from("--principal"),
        OsString::from(principal.as_str()),
        OsString::from("keypair"),
        OsString::from("generate"),
        OsString::from("--name"),
        OsString::from(DEVICE_NAME),
        OsString::from("--raw"),
    ]
}

pub(crate) fn runtime_issue_args(principal: &PrincipalId) -> Vec<OsString> {
    vec![
        OsString::from("--principal"),
        OsString::from(principal.as_str()),
        OsString::from("pair-device"),
        OsString::from("issue"),
        OsString::from("--scope"),
        OsString::from("use-only"),
        OsString::from("--label"),
        OsString::from(DEVICE_NAME),
        OsString::from("--raw"),
    ]
}

pub(crate) fn runtime_redeem_args(
    principal: &PrincipalId,
    public_key: &str,
) -> Result<Vec<OsString>, SetupError> {
    validate_public_key(public_key)?;
    Ok(vec![
        OsString::from("--principal"),
        OsString::from(principal.as_str()),
        OsString::from("pair-device"),
        OsString::from("redeem"),
        OsString::from("--public-key"),
        OsString::from(public_key),
    ])
}

pub(crate) fn run(home: &AosHome, principal: &PrincipalId) -> Result<SetupDocument, SetupError> {
    if *principal == PrincipalId::anonymous() {
        return Err(SetupError::Failed(
            "native-input setup requires a non-anonymous owned principal".to_owned(),
        ));
    }
    let home = AosHome::from_root(canonical_setup_home(home.root())?);
    let connection = connection_path(&home);
    let config_path = operator_config_path(&home);
    let artifacts = KeyArtifacts::from_private(private_key_path(&home))?;
    let _lock = SetupLock::acquire(&home)?;
    refuse_existing(&connection, &config_path, &artifacts)?;
    let mut owned = OwnedCleanup::default();
    match enroll(
        &home,
        principal,
        &connection,
        &config_path,
        &artifacts,
        &mut owned,
    ) {
        Ok(document) => Ok(document),
        Err(error) => {
            owned.rollback();
            Err(error)
        }
    }
}

fn enroll(
    home: &AosHome,
    principal: &PrincipalId,
    connection: &Path,
    config_path: &Path,
    artifacts: &KeyArtifacts,
    owned: &mut OwnedCleanup,
) -> Result<SetupDocument, SetupError> {
    let public_key = generate_device_key(home, principal, artifacts, owned)?;
    let mut token =
        parse_pair_token(&run_runtime(home, runtime_issue_args(principal), None)?.stdout)?;
    let redeemed = match run_runtime(
        home,
        runtime_redeem_args(principal, &public_key)?,
        Some(&token),
    ) {
        Ok(output) => {
            token.fill(0);
            parse_redeemed(&output.stdout, principal)?
        }
        Err(error) => {
            token.fill(0);
            return Err(error);
        }
    };
    write_connection(home, principal, connection, &artifacts.private)?;
    owned.claim_connection(connection);
    // Redeem already happened; this path does not call pair-device revoke.
    write_responder(config_path, principal, &redeemed.key_id)?;
    Ok(SetupDocument::local(principal, connection))
}

fn generate_device_key(
    home: &AosHome,
    principal: &PrincipalId,
    artifacts: &KeyArtifacts,
    owned: &mut OwnedCleanup,
) -> Result<String, SetupError> {
    let before = artifacts.presence()?;
    let generated = run_runtime(home, runtime_generate_args(principal), None);
    owned.claim_new_keys(artifacts, &before)?;
    parse_public_key(&generated?.stdout)
}

impl KeyArtifacts {
    fn from_private(private: PathBuf) -> Result<Self, SetupError> {
        let parent = private.parent().ok_or_else(|| {
            SetupError::Failed("device key path has no parent directory".to_owned())
        })?;
        let name = private
            .file_stem()
            .and_then(|name| name.to_str())
            .ok_or_else(|| SetupError::Failed("device key name is invalid".to_owned()))?;
        Ok(Self {
            public_hex: parent.join(format!("{name}.pub.hex")),
            meta: parent.join(format!("{name}.meta.toml")),
            private,
        })
    }

    fn presence(&self) -> Result<KeyPresence, SetupError> {
        Ok(KeyPresence {
            private: path_exists(&self.private)?,
            public_hex: path_exists(&self.public_hex)?,
            meta: path_exists(&self.meta)?,
        })
    }
}

impl KeyPresence {
    fn any(&self) -> bool {
        self.private || self.public_hex || self.meta
    }
}

impl OwnedCleanup {
    fn claim_new_keys(
        &mut self,
        artifacts: &KeyArtifacts,
        before: &KeyPresence,
    ) -> Result<(), SetupError> {
        let now = artifacts.presence()?;
        if !before.private && now.private {
            self.private = Some(artifacts.private.clone());
        }
        if !before.public_hex && now.public_hex {
            self.public_hex = Some(artifacts.public_hex.clone());
        }
        if !before.meta && now.meta {
            self.meta = Some(artifacts.meta.clone());
        }
        Ok(())
    }

    fn claim_connection(&mut self, connection: &Path) {
        self.connection = Some(connection.to_path_buf());
    }

    fn rollback(&self) {
        for path in [
            self.connection.as_deref(),
            self.private.as_deref(),
            self.public_hex.as_deref(),
            self.meta.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            let _ = fs::remove_file(path);
        }
    }
}

impl SetupLock {
    fn acquire(home: &AosHome) -> Result<Self, SetupError> {
        let dir = home.root().join("native-input");
        create_private_dir(&dir)?;
        let path = dir.join(SETUP_LOCK_FILE);
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .mode(0o600)
            .open(&path)
            .map_err(|error| SetupError::Failed(error.to_string()))?;
        let path_metadata =
            fs::symlink_metadata(&path).map_err(|error| SetupError::Failed(error.to_string()))?;
        if path_metadata.file_type().is_symlink() || !path_metadata.is_file() {
            return Err(SetupError::Failed(
                "native-input setup lock must be a real regular file".to_owned(),
            ));
        }
        let file_metadata = file
            .metadata()
            .map_err(|error| SetupError::Failed(error.to_string()))?;
        if !file_metadata.is_file()
            || path_metadata.dev() != file_metadata.dev()
            || path_metadata.ino() != file_metadata.ino()
        {
            return Err(SetupError::Failed(
                "native-input setup lock changed while it was opened".to_owned(),
            ));
        }
        file.try_lock_exclusive().map_err(|error| {
            if error.kind() == io::ErrorKind::WouldBlock {
                SetupError::Failed("another native-input setup is already in progress".to_owned())
            } else {
                SetupError::Failed(error.to_string())
            }
        })?;
        Ok(Self { _file: file })
    }
}

pub(crate) fn runtime_unsupported(output: &Output) -> bool {
    if output.status.success() {
        return false;
    }
    let text = format!(
        "{}\n{}",
        String::from_utf8_lossy(&output.stderr),
        String::from_utf8_lossy(&output.stdout)
    )
    .to_ascii_lowercase();
    let unknown = text.contains("unexpected argument")
        || text.contains("unknown argument")
        || text.contains("unrecognized")
        || text.contains("invalid option")
        || text.contains("wasn't expected")
        || text.contains("was not expected")
        || text.contains("unrecognized subcommand")
        || text.contains("cannot find binary")
        || text.contains("no such file");
    let mentions_setup = text.contains("keypair")
        || text.contains("pair-device")
        || text.contains("pair_device")
        || text.contains("--raw")
        || text.contains("--scope")
        || text.contains("--public-key");
    unknown && mentions_setup
}

fn refuse_existing(
    connection: &Path,
    config: &Path,
    artifacts: &KeyArtifacts,
) -> Result<(), SetupError> {
    if path_exists(connection)? {
        return Err(SetupError::Failed(
            "a native-input connection already exists; not overwritten".to_owned(),
        ));
    }
    if artifacts.presence()?.any() {
        return Err(SetupError::Failed(
            "device key aos-tray already exists; not overwritten".to_owned(),
        ));
    }
    if path_exists(config)? {
        let existing = read_regular_file(config)?;
        if toml_has_native_input(&existing)? {
            return Err(SetupError::Failed(
                "native-input operator routing already exists; not overwritten".to_owned(),
            ));
        }
    }
    Ok(())
}

fn run_runtime(
    home: &AosHome,
    args: Vec<OsString>,
    stdin: Option<&[u8]>,
) -> Result<Output, SetupError> {
    let mut command = home
        .runtime_command_with_args(args)
        .map_err(|error| SetupError::Failed(error.to_string()))?;
    command.stdout(Stdio::piped()).stderr(Stdio::piped());
    if stdin.is_some() {
        command.stdin(Stdio::piped());
    } else {
        command.stdin(Stdio::null());
    }
    let mut child = command
        .spawn()
        .map_err(|error| SetupError::Failed(error.to_string()))?;
    if let Some(token) = stdin {
        let mut handle = child.stdin.take().ok_or_else(|| {
            SetupError::Failed("runtime did not accept the pairing token on stdin".to_owned())
        })?;
        handle
            .write_all(token)
            .and_then(|()| handle.write_all(b"\n"))
            .map_err(|error| SetupError::Failed(error.to_string()))?;
    }
    let output = child
        .wait_with_output()
        .map_err(|error| SetupError::Failed(error.to_string()))?;
    if output.status.success() {
        return Ok(output);
    }
    if runtime_unsupported(&output) {
        return Err(SetupError::Unsupported);
    }
    let detail = String::from_utf8_lossy(&output.stderr);
    let detail = detail.trim();
    if detail.is_empty() {
        Err(SetupError::Failed("native-input setup failed".to_owned()))
    } else {
        Err(SetupError::Failed(format!(
            "native-input setup failed: {detail}"
        )))
    }
}

fn parse_public_key(bytes: &[u8]) -> Result<String, SetupError> {
    let text = String::from_utf8(bytes.to_vec())
        .map_err(|_| SetupError::Failed("runtime did not return a device public key".to_owned()))?;
    let key = text.trim();
    validate_public_key(key)?;
    Ok(key.to_owned())
}

fn validate_public_key(key: &str) -> Result<(), SetupError> {
    if key.len() == 64
        && key
            .bytes()
            .all(|byte| matches!(byte, b'0'..=b'9' | b'a'..=b'f'))
    {
        Ok(())
    } else {
        Err(SetupError::Failed(
            "runtime did not return a device public key".to_owned(),
        ))
    }
}

fn parse_pair_token(bytes: &[u8]) -> Result<Vec<u8>, SetupError> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| SetupError::Failed("runtime did not return a pairing token".to_owned()))?;
    let token = text.trim();
    if token.starts_with("astrid_pair_")
        && token.len() > "astrid_pair_".len()
        && token.len() <= 256
        && token.bytes().all(|byte| byte.is_ascii_graphic())
    {
        Ok(token.as_bytes().to_vec())
    } else {
        Err(SetupError::Failed(
            "runtime did not return a pairing token".to_owned(),
        ))
    }
}

#[derive(Debug, serde::Deserialize)]
struct RedeemedDevice {
    principal: String,
    key_id: String,
}

fn parse_redeemed(bytes: &[u8], expected: &PrincipalId) -> Result<RedeemedDevice, SetupError> {
    let redeemed: RedeemedDevice = serde_json::from_slice(bytes)
        .map_err(|_| SetupError::Failed("runtime did not return a pairing result".to_owned()))?;
    let principal = PrincipalId::new(redeemed.principal.clone())
        .map_err(|error| SetupError::Failed(format!("invalid paired principal: {error}")))?;
    if principal != *expected {
        return Err(SetupError::Failed(
            "pairing result did not match the selected principal".to_owned(),
        ));
    }
    DeviceKeyId::new(redeemed.key_id.clone())
        .map_err(|error| SetupError::Failed(format!("invalid paired device: {error}")))?;
    Ok(redeemed)
}

fn write_connection(
    home: &AosHome,
    principal: &PrincipalId,
    connection: &Path,
    key_path: &Path,
) -> Result<(), SetupError> {
    let document = serde_json::json!({
        "socketPath": socket_path(home),
        "principal": principal.to_string(),
        "privateKeyPath": key_path,
        "tokenPath": token_path(home),
        "capacity": CONNECTION_CAPACITY,
        "inputTimeoutSeconds": INPUT_TIMEOUT_SECONDS,
        "ioTimeoutSeconds": IO_TIMEOUT_SECONDS,
        "readTimeoutSeconds": READ_TIMEOUT_SECONDS,
    });
    let bytes = serde_json::to_vec_pretty(&document)
        .map_err(|error| SetupError::Failed(error.to_string()))?;
    write_private_create_new(connection, &bytes)
}

fn write_responder(
    config_path: &Path,
    principal: &PrincipalId,
    key_id: &str,
) -> Result<(), SetupError> {
    DeviceKeyId::new(key_id.to_owned())
        .map_err(|error| SetupError::Failed(format!("invalid paired device: {error}")))?;
    let existing = if path_exists(config_path)? {
        read_regular_file(config_path)?
    } else {
        String::new()
    };
    let rendered = insert_responder(&existing, principal, key_id)?;
    write_private_replace(config_path, rendered.as_bytes())
}

fn insert_responder(
    existing: &str,
    principal: &PrincipalId,
    key_id: &str,
) -> Result<String, SetupError> {
    let mut table = if existing.trim().is_empty() {
        toml::Table::new()
    } else {
        existing.parse::<toml::Table>().map_err(|_| {
            SetupError::Failed("operator configuration is not valid TOML".to_owned())
        })?
    };
    if table.contains_key("native_input") {
        return Err(SetupError::Failed(
            "native-input operator routing already exists; not overwritten".to_owned(),
        ));
    }
    let mut responders = toml::Table::new();
    responders.insert(
        principal.to_string(),
        toml::Value::String(key_id.to_owned()),
    );
    let mut native_input = toml::Table::new();
    native_input.insert("responders".to_owned(), toml::Value::Table(responders));
    table.insert("native_input".to_owned(), toml::Value::Table(native_input));
    toml::to_string(&table).map_err(|error| SetupError::Failed(error.to_string()))
}

fn toml_has_native_input(existing: &str) -> Result<bool, SetupError> {
    if existing.trim().is_empty() {
        return Ok(false);
    }
    let table = existing
        .parse::<toml::Table>()
        .map_err(|_| SetupError::Failed("operator configuration is not valid TOML".to_owned()))?;
    Ok(table.contains_key("native_input"))
}

fn path_exists(path: &Path) -> Result<bool, SetupError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(SetupError::Failed(error.to_string())),
    }
}

fn read_regular_file(path: &Path) -> Result<String, SetupError> {
    let metadata =
        fs::symlink_metadata(path).map_err(|error| SetupError::Failed(error.to_string()))?;
    if !metadata.is_file() {
        return Err(SetupError::Failed(format!(
            "expected a regular file at {}",
            path.display()
        )));
    }
    fs::read_to_string(path).map_err(|error| SetupError::Failed(error.to_string()))
}

fn write_private_create_new(path: &Path, bytes: &[u8]) -> Result<(), SetupError> {
    let parent = path.parent().ok_or_else(|| {
        SetupError::Failed("native-input connection path has no parent directory".to_owned())
    })?;
    create_private_dir(parent)?;
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map_err(|error| {
            if error.kind() == io::ErrorKind::AlreadyExists {
                SetupError::Failed(
                    "a native-input connection already exists; not overwritten".to_owned(),
                )
            } else {
                SetupError::Failed(error.to_string())
            }
        })?;
    file.write_all(bytes)
        .map_err(|error| SetupError::Failed(error.to_string()))
}

fn write_private_replace(path: &Path, bytes: &[u8]) -> Result<(), SetupError> {
    let parent = path.parent().ok_or_else(|| {
        SetupError::Failed("operator configuration path has no parent directory".to_owned())
    })?;
    create_private_dir(parent)?;
    let temporary = parent.join(format!(
        ".{}.tmp-{}",
        path.file_name()
            .and_then(|name| name.to_str())
            .ok_or_else(|| SetupError::Failed(
                "operator configuration name is invalid".to_owned()
            ))?,
        std::process::id()
    ));
    let write = (|| {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(bytes)?;
        file.sync_all()
    })();
    if let Err(error) = write {
        let _ = fs::remove_file(&temporary);
        return Err(SetupError::Failed(error.to_string()));
    }
    fs::rename(&temporary, path).map_err(|error| {
        let _ = fs::remove_file(&temporary);
        SetupError::Failed(error.to_string())
    })
}

fn create_private_dir(path: &Path) -> Result<(), SetupError> {
    fs::create_dir_all(path).map_err(|error| SetupError::Failed(error.to_string()))?;
    let metadata =
        fs::symlink_metadata(path).map_err(|error| SetupError::Failed(error.to_string()))?;
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(SetupError::Failed(format!(
            "AOS managed path must be a real directory: {}",
            path.display()
        )));
    }
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))
        .map_err(|error| SetupError::Failed(error.to_string()))
}

#[cfg(test)]
mod tests;
