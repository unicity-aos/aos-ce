//! Local-personal native-input first-run setup.
//!
//! Pairing enrolls a dedicated device; it does not select a native responder
//! or grant hosted acting authority. This module writes only the tray
//! connection file and a missing `[native_input.responders]` binding.

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Output, Stdio};

use astrid_core::{DeviceKeyId, PrincipalId};
use serde::Serialize;

use unicity_aos_bootstrap::AosHome;

pub(crate) const DEVICE_NAME: &str = "aos-tray";
pub(crate) const UNSUPPORTED_MESSAGE: &str = "native-input setup is not supported by this runtime";

const CONNECTION_CAPACITY: i32 = 8;
const INPUT_TIMEOUT_SECONDS: f64 = 120.0;
const IO_TIMEOUT_SECONDS: f64 = 5.0;
const READ_TIMEOUT_SECONDS: f64 = 3600.0;

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
    let connection = connection_path(home);
    let config_path = operator_config_path(home);
    let key_path = private_key_path(home);
    refuse_existing(&connection, &config_path, &key_path)?;
    match enroll(home, principal, &connection, &config_path, &key_path) {
        Ok(document) => Ok(document),
        Err(error) => {
            rollback_generated(&connection, &key_path);
            Err(error)
        }
    }
}

fn enroll(
    home: &AosHome,
    principal: &PrincipalId,
    connection: &Path,
    config_path: &Path,
    key_path: &Path,
) -> Result<SetupDocument, SetupError> {
    let public_key =
        parse_public_key(&run_runtime(home, runtime_generate_args(principal), None)?.stdout)?;
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
    write_connection(home, principal, connection, key_path)?;
    write_responder(config_path, principal, &redeemed.key_id)?;
    Ok(SetupDocument::local(principal, connection))
}

fn rollback_generated(connection: &Path, key_path: &Path) {
    let _ = fs::remove_file(connection);
    let _ = fs::remove_file(key_path);
    if let (Some(parent), Some(name)) = (
        key_path.parent(),
        key_path.file_stem().and_then(|name| name.to_str()),
    ) {
        let _ = fs::remove_file(parent.join(format!("{name}.pub.hex")));
        let _ = fs::remove_file(parent.join(format!("{name}.meta.toml")));
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

fn refuse_existing(connection: &Path, config: &Path, key: &Path) -> Result<(), SetupError> {
    if path_exists(connection)? {
        return Err(SetupError::Failed(
            "a native-input connection already exists; not overwritten".to_owned(),
        ));
    }
    if path_exists(key)? {
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
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::process::ExitStatusExt;

    fn principal() -> PrincipalId {
        PrincipalId::new("alice").expect("valid principal")
    }

    #[test]
    fn pairing_args_never_include_a_token_or_force_flag() {
        let principal = principal();
        let generate: Vec<String> = runtime_generate_args(&principal)
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let issue: Vec<String> = runtime_issue_args(&principal)
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        let redeem: Vec<String> = runtime_redeem_args(&principal, &"ab".repeat(32))
            .expect("valid public key")
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            generate,
            [
                "--principal",
                "alice",
                "keypair",
                "generate",
                "--name",
                "aos-tray",
                "--raw"
            ]
        );
        assert_eq!(
            issue,
            [
                "--principal",
                "alice",
                "pair-device",
                "issue",
                "--scope",
                "use-only",
                "--label",
                "aos-tray",
                "--raw"
            ]
        );
        assert_eq!(
            redeem,
            [
                "--principal",
                "alice",
                "pair-device",
                "redeem",
                "--public-key",
                &"ab".repeat(32)
            ]
        );
        for args in [&generate, &issue, &redeem] {
            assert!(!args.iter().any(|arg| arg.contains("astrid_pair_")));
            assert!(!args.iter().any(|arg| arg == "--force"));
            assert!(!args.iter().any(|arg| arg.contains("token")));
        }
    }

    #[test]
    fn product_paths_stay_under_the_aos_home() {
        let home = AosHome::from_root("/tmp/aos-setup-home");
        assert_eq!(
            connection_path(&home),
            PathBuf::from("/tmp/aos-setup-home/native-input/connection.json")
        );
        assert_eq!(
            operator_config_path(&home),
            PathBuf::from("/tmp/aos-setup-home/runtime/config.toml")
        );
        assert_eq!(
            private_key_path(&home),
            PathBuf::from("/tmp/aos-setup-home/runtime/keys/local/aos-tray.ed25519")
        );
        assert_eq!(
            socket_path(&home),
            PathBuf::from("/tmp/aos-setup-home/run/system.sock")
        );
        assert_eq!(
            token_path(&home),
            PathBuf::from("/tmp/aos-setup-home/run/system.token")
        );
        assert_ne!(
            socket_path(&home),
            home.runtime_home().join("run/system.sock")
        );
    }

    #[test]
    fn responder_insert_is_narrow_and_refuses_existing_native_input() {
        let principal = principal();
        let written = insert_responder("", &principal, "0123456789abcdef").expect("insert");
        assert!(written.contains("[native_input.responders]"));
        assert!(written.contains("alice"));
        assert!(written.contains("0123456789abcdef"));

        let preserved = insert_responder("strict = true\n", &principal, "0123456789abcdef")
            .expect("insert beside unrelated keys");
        assert!(preserved.contains("strict = true"));
        assert!(preserved.contains("[native_input.responders]"));

        let error = insert_responder(
            "[native_input.responders]\nbob = 'fedcba9876543210'\n",
            &principal,
            "0123456789abcdef",
        )
        .expect_err("existing native_input");
        match error {
            SetupError::Failed(message) => {
                assert!(message.contains("already exists; not overwritten"));
            }
            SetupError::Unsupported => panic!("existing routing is not unsupported"),
        }
    }

    #[test]
    fn public_key_token_and_redeem_parsing_stay_strict() {
        let principal = principal();
        assert_eq!(
            parse_public_key(format!("{}\n", "ab".repeat(32)).as_bytes()).expect("key"),
            "ab".repeat(32)
        );
        assert!(parse_public_key(b"short").is_err());
        assert!(parse_pair_token(b"astrid_pair_device-token\n").is_ok());
        assert!(parse_pair_token(b"astrid_inv_device-token\n").is_err());
        let redeemed = parse_redeemed(
            br#"{"principal":"alice","public_key_fingerprint":"blake3:aa","key_id":"0123456789abcdef"}"#,
            &principal,
        )
        .expect("redeem");
        assert_eq!(redeemed.key_id, "0123456789abcdef");
        assert!(
            parse_redeemed(
                br#"{"principal":"bob","public_key_fingerprint":"blake3:aa","key_id":"0123456789abcdef"}"#,
                &principal,
            )
            .is_err()
        );
    }

    #[test]
    fn unsupported_runtime_is_classified_without_a_global_fallback() {
        let unsupported = Output {
            status: ExitStatusExt::from_raw(2 << 8),
            stdout: Vec::new(),
            stderr: b"error: unexpected argument '--scope' found".to_vec(),
        };
        assert!(runtime_unsupported(&unsupported));
        let failed = Output {
            status: ExitStatusExt::from_raw(1 << 8),
            stdout: Vec::new(),
            stderr: b"principal requires a delegated device".to_vec(),
        };
        assert!(!runtime_unsupported(&failed));
    }

    #[test]
    fn private_connection_file_is_created_0600_and_not_overwritten() {
        let root = std::env::temp_dir().join(format!(
            "aos-native-setup-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("temp");
        let path = root.join("connection.json");
        write_private_create_new(&path, b"{\"principal\":\"alice\"}").expect("write");
        let mode = fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        let error = write_private_create_new(&path, b"other").expect_err("overwrite");
        match error {
            SetupError::Failed(message) => assert!(message.contains("already exists")),
            SetupError::Unsupported => panic!("existing file is not unsupported"),
        }
        assert_eq!(fs::read(&path).expect("read"), b"{\"principal\":\"alice\"}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn generated_key_sidecars_are_removed_on_later_failure() {
        let root = std::env::temp_dir().join(format!(
            "aos-native-setup-rollback-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("temp");
        let connection = root.join("connection.json");
        let key = root.join("aos-tray.ed25519");
        let public_hex = root.join("aos-tray.pub.hex");
        let meta = root.join("aos-tray.meta.toml");
        let keep = root.join("unrelated");
        fs::write(&connection, b"partial").expect("connection");
        fs::write(&key, [1u8; 32]).expect("key");
        fs::write(&public_hex, b"ab").expect("pub");
        fs::write(&meta, b"note = true\n").expect("meta");
        fs::write(&keep, b"keep").expect("keep");
        rollback_generated(&connection, &key);
        assert!(!connection.exists());
        assert!(!key.exists());
        assert!(!public_hex.exists());
        assert!(!meta.exists());
        assert_eq!(fs::read(&keep).expect("kept"), b"keep");
        let _ = fs::remove_dir_all(&root);
    }
}
