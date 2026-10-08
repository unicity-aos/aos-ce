//! Root-registered Codewall evaluation route shared by native hooks and MCP.

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn valid_route() -> serde_json::Value {
        json!({
            "schema_version": 1,
            "principal": "claude-code",
            "supervised_uid": 501,
            "gate": "/usr/local/libexec/codewall-gate",
            "gate_sha256": "a".repeat(64),
            "service_socket": "/var/run/codewall/gate.sock",
            "service_uid": 502,
            "installation_id": "11111111-1111-4111-8111-111111111111",
            "enforcer_source_id": "22222222-2222-4222-8222-222222222222"
        })
    }

    #[test]
    fn route_rejects_wrong_identity_and_unknown_fields() {
        let route = valid_route();
        assert!(Route::parse(&route.to_string(), "claude-code", 501).is_ok());
        assert!(Route::parse(&route.to_string(), "other", 501).is_err());
        assert!(Route::parse(&route.to_string(), "claude-code", 502).is_err());
        let mut extra = route;
        extra["signing_key"] = json!("attacker supplied");
        assert!(Route::parse(&extra.to_string(), "claude-code", 501).is_err());
    }

    #[test]
    fn route_rejects_replacement_paths_and_shared_service_identity() {
        let mut route = valid_route();
        route["gate"] = json!("/usr/local/../tmp/gate");
        assert!(Route::parse(&route.to_string(), "claude-code", 501).is_err());
        route["gate"] = json!("/usr/local/libexec/codewall-gate");
        route["service_uid"] = json!(501);
        assert!(Route::parse(&route.to_string(), "claude-code", 501).is_err());
    }

    #[test]
    fn gate_requests_use_neutral_tool_and_prompt_schema() {
        assert_eq!(
            gate_request(&Action::Tool {
                name: "Bash".into(),
                arguments: json!({"command":"pwd"})
            }),
            json!({"schema_version":1,"event":"pre_tool_use","action":{"kind":"tool","name":"Bash","arguments":{"command":"pwd"}}})
        );
        assert_eq!(
            gate_request(&Action::Prompt {
                text: "hello".into()
            }),
            json!({"schema_version":1,"event":"user_prompt_submit","action":{"kind":"prompt","text":"hello"}})
        );
    }

    #[test]
    fn malformed_gate_reply_cannot_grant_permission() {
        assert!(parse_gate_reply(b"{}", "pre_tool_use").is_err());
        assert!(parse_gate_reply(br#"{"schema_version":1,"event":"user_prompt_submit","decision":{"skip":false,"ask":false,"reason":null},"context":null}"#, "pre_tool_use").is_err());
        assert!(parse_gate_reply(br#"{"schema_version":1,"event":"pre_tool_use","decision":{"skip":false,"ask":false,"reason":null},"context":null}"#, "pre_tool_use").is_ok());
    }

    #[cfg(unix)]
    #[test]
    fn nonreading_gate_cannot_stall_stdin_past_deadline() {
        use std::os::unix::fs::PermissionsExt;
        let directory = tempfile::tempdir().expect("temporary gate");
        let gate = directory.path().join("gate");
        fs::write(&gate, "#!/bin/sh\nwhile :; do :; done\n").expect("fake gate");
        fs::set_permissions(&gate, fs::Permissions::from_mode(0o700)).expect("executable gate");
        let mut command = tokio::process::Command::new(&gate);
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime");
        let started = std::time::Instant::now();
        let result = runtime.block_on(run_gate(
            command,
            &vec![b'x'; 1024 * 1024],
            Duration::from_millis(100),
        ));
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
    }
    #[cfg(unix)]
    fn test_store(base: &Path) -> RouteStore {
        RouteStore {
            trust_base: base.to_path_buf(),
            root: base.join("aos-routes"),
            owner: rustix::process::geteuid().as_raw(),
        }
    }

    #[cfg(unix)]
    fn mkdir(path: &Path, mode: u32) {
        use std::os::unix::fs::PermissionsExt;
        fs::create_dir_all(path).expect("directory");
        fs::set_permissions(path, fs::Permissions::from_mode(mode)).expect("mode");
    }

    #[cfg(unix)]
    #[test]
    fn missing_route_directory_means_codewall_is_not_installed_for_uid() {
        let base = tempfile::tempdir().expect("base");
        let store = test_store(base.path());
        assert!(matches!(store.load("claude-code", 501), Ok(None)));
        mkdir(&store.root, 0o755);
        assert!(matches!(store.load("claude-code", 501), Ok(None)));
    }

    #[cfg(unix)]
    #[test]
    fn protected_uid_directory_makes_unrouted_principal_fail_closed() {
        let base = tempfile::tempdir().expect("base");
        let store = test_store(base.path());
        mkdir(&store.root.join("501"), 0o755);
        assert!(store.load("claude-code", 501).is_err());
        assert!(store.load("attacker-chosen", 501).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn ancestors_are_validated_before_missing_route_is_trusted() {
        let base = tempfile::tempdir().expect("base");
        let store = test_store(base.path());
        mkdir(&store.root, 0o777);
        assert!(store.load("claude-code", 501).is_err());
        mkdir(&store.root, 0o755);
        mkdir(&store.root.join("501"), 0o775);
        assert!(store.load("claude-code", 501).is_err());
        fs::remove_dir(store.root.join("501")).expect("remove");
        std::os::unix::fs::symlink(base.path(), store.root.join("501")).expect("symlink");
        assert!(store.load("claude-code", 501).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn user_hard_link_to_gate_does_not_disable_codewall() {
        use std::os::unix::fs::PermissionsExt;
        let base = tempfile::tempdir().expect("base");
        let store = test_store(base.path());
        let gate = base.path().join("gate");
        fs::write(&gate, "#!/bin/sh\n").expect("gate");
        fs::set_permissions(&gate, fs::Permissions::from_mode(0o755)).expect("mode");
        fs::hard_link(&gate, base.path().join("user-link")).expect("hard link");
        assert!(store.safe_file(&gate, true).is_ok());
    }

    #[test]
    fn evaluation_deadline_outlasts_gate_supervisor() {
        // codewall-gate's supervisor answers within 7.5 s; AOS must wait longer.
        assert_eq!(GATE_TIMEOUT, Duration::from_secs(10));
        assert_eq!(INVENTORY_TIMEOUT, Duration::from_secs(70));
    }

    #[cfg(unix)]
    fn script_gate(directory: &Path, body: &str) -> tokio::process::Command {
        use std::os::unix::fs::PermissionsExt;
        let gate = directory.join("gate");
        fs::write(&gate, format!("#!/bin/sh\n{body}\n")).expect("fake gate");
        fs::set_permissions(&gate, fs::Permissions::from_mode(0o700)).expect("executable gate");
        let mut command = tokio::process::Command::new(&gate);
        command
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        command
    }

    #[cfg(unix)]
    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("test runtime")
            .block_on(future)
    }

    #[cfg(unix)]
    #[test]
    fn gate_timeout_kills_the_whole_gate_process_group() {
        let directory = tempfile::tempdir().expect("temporary gate");
        let pidfile = directory.path().join("worker.pid");
        let command = script_gate(
            directory.path(),
            &format!("sleep 30 &\necho $! > '{}'\nwait", pidfile.display()),
        );
        let result = block_on(run_gate(command, b"{}", Duration::from_millis(300)));
        assert!(result.is_err());
        let worker: i32 = fs::read_to_string(&pidfile)
            .expect("worker pid")
            .trim()
            .parse()
            .expect("pid");
        let pid = rustix::process::Pid::from_raw(worker).expect("pid");
        let started = std::time::Instant::now();
        while rustix::process::test_kill_process(pid).is_ok() {
            assert!(
                started.elapsed() < Duration::from_secs(3),
                "gate worker survived the deadline"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    #[cfg(unix)]
    #[test]
    fn oversized_gate_reply_fails_without_buffering_until_deadline() {
        let directory = tempfile::tempdir().expect("temporary gate");
        let command = script_gate(directory.path(), "cat >/dev/null\nyes");
        let started = std::time::Instant::now();
        let result = block_on(run_gate(command, b"{}", Duration::from_secs(5)));
        assert!(result.is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[test]
    fn capability_probe_refuses_missing_runtime_assets() {
        let home = unicity_aos_bootstrap::AosHome::from_root("/nonexistent/aos-codewall-probe");
        assert!(capability(&home).is_err());
    }

    #[test]
    fn inventory_receipt_must_match_requested_identity() {
        let request = br#"{"schema_version":1,"request_id":"abc","context_id":"def","operation":"begin","report_json":null}"#;
        let input = validate_inventory_input(request).expect("input");
        let right = br#"{"schema_version":1,"request_id":"abc","context_id":"def","principal":"claude-code","status":"ready","report_sha256":null,"item_count":null,"error":null}"#;
        assert!(validate_inventory_receipt(right, &input, "claude-code").is_ok());
        let wrong = br#"{"schema_version":1,"request_id":"abc","context_id":"def","principal":"other","status":"ready","report_sha256":null,"item_count":null,"error":null}"#;
        assert!(validate_inventory_receipt(wrong, &input, "claude-code").is_err());
    }
}
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt;
use std::path::{Component, Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use uuid::Uuid;

const MAX_ROUTE_BYTES: u64 = 8192;
const MAX_REPLY_BYTES: usize = 8192;
/// Longer than codewall-gate's 7.5 s supervisor deadline, so a slow but valid
/// verdict is not converted into an AOS-side denial.
pub(crate) const GATE_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_INVENTORY_BYTES: u64 = 1024 * 1024;
const INVENTORY_TIMEOUT: Duration = Duration::from_secs(70);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct InventoryInput {
    schema_version: u8,
    request_id: String,
    context_id: String,
    operation: String,
    report_json: Option<String>,
}

fn validate_inventory_input(bytes: &[u8]) -> Result<InventoryInput, String> {
    if bytes.len() as u64 > MAX_INVENTORY_BYTES {
        return Err("Codewall inventory request too large".into());
    }
    let input: InventoryInput = serde_json::from_slice(bytes)
        .map_err(|e| format!("invalid Codewall inventory request: {e}"))?;
    if input.schema_version != 1
        || input.request_id.is_empty()
        || input.request_id.len() > 128
        || input.context_id.is_empty()
        || input.context_id.len() > 128
        || !matches!(input.operation.as_str(), "begin" | "cancel" | "submit")
        || (input.operation == "submit") != input.report_json.is_some()
    {
        return Err("unsupported Codewall inventory request".into());
    }
    Ok(input)
}

fn validate_inventory_receipt(
    bytes: &[u8],
    input: &InventoryInput,
    principal: &str,
) -> Result<(), String> {
    if bytes.len() > MAX_REPLY_BYTES {
        return Err("Codewall inventory receipt too large".into());
    }
    let receipt: Value = serde_json::from_slice(bytes)
        .map_err(|e| format!("invalid Codewall inventory receipt: {e}"))?;
    if receipt.get("schema_version").and_then(Value::as_u64) != Some(1)
        || receipt.get("principal").and_then(Value::as_str) != Some(principal)
        || receipt.get("request_id").and_then(Value::as_str) != Some(input.request_id.as_str())
        || receipt.get("context_id").and_then(Value::as_str) != Some(input.context_id.as_str())
        || !receipt.get("error").is_some_and(Value::is_null)
        || receipt.get("status").and_then(Value::as_str).is_none()
    {
        return Err("Codewall inventory receipt does not match the request".into());
    }
    Ok(())
}

struct GateOutput {
    success: bool,
    stdout: Vec<u8>,
}

/// Run the gate in its own process group. On any failure, including the
/// deadline, the whole group (gate supervisor and its worker) is killed and the
/// gate is reaped. Stdout is read with a bound rather than buffered in full.
async fn run_gate(
    mut command: tokio::process::Command,
    payload: &[u8],
    deadline: Duration,
) -> Result<GateOutput, String> {
    #[cfg(unix)]
    command.process_group(0);
    let mut child = command
        .spawn()
        .map_err(|e| format!("Codewall gate unavailable: {e}"))?;
    let group = child.id();
    let result = tokio::time::timeout(deadline, async {
        let mut stdin = child
            .stdin
            .take()
            .ok_or("Codewall gate input unavailable")?;
        let stdout = child
            .stdout
            .take()
            .ok_or("Codewall gate output unavailable")?;
        let write = async {
            stdin
                .write_all(payload)
                .await
                .map_err(|e| format!("Codewall gate input failed: {e}"))?;
            drop(stdin);
            Ok::<_, String>(())
        };
        let read = async {
            let mut bytes = Vec::new();
            stdout
                .take(MAX_REPLY_BYTES as u64 + 1)
                .read_to_end(&mut bytes)
                .await
                .map_err(|e| format!("Codewall gate output failed: {e}"))?;
            if bytes.len() > MAX_REPLY_BYTES {
                return Err("Codewall gate reply too large".to_owned());
            }
            Ok(bytes)
        };
        let ((), stdout) = tokio::try_join!(write, read)?;
        let status = child
            .wait()
            .await
            .map_err(|e| format!("Codewall gate failed: {e}"))?;
        Ok(GateOutput {
            success: status.success(),
            stdout,
        })
    })
    .await
    .unwrap_or_else(|_| Err("Codewall gate timed out".to_owned()));
    if result.is_err() {
        // The group id stays reserved while the unreaped leader or any member
        // exists, so this cannot signal an unrelated process group.
        #[cfg(unix)]
        if let Some(group) = group
            .and_then(|pid| i32::try_from(pid).ok())
            .and_then(rustix::process::Pid::from_raw)
        {
            let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
        }
        #[cfg(not(unix))]
        let _ = group;
        let _ = child.kill().await;
    }
    result
}

pub(crate) async fn inventory(principal: &str, bytes: &[u8]) -> Result<Vec<u8>, String> {
    let input = validate_inventory_input(bytes)?;
    let route = load(principal)?.ok_or("Codewall service route is not installed")?;
    let mut command = tokio::process::Command::new(&route.gate);
    command
        .args([
            "--inventory",
            "--principal",
            &route.principal,
            "--enforcer-source-id",
            &route.enforcer_source_id.to_string(),
            "--service-socket",
            route.service_socket.to_str().ok_or("invalid socket path")?,
            "--service-uid",
            &route.service_uid.to_string(),
            "--service-installation-id",
            &route.installation_id.to_string(),
        ])
        .env_clear()
        .current_dir("/")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let output = run_gate(command, bytes, INVENTORY_TIMEOUT).await?;
    if !output.success {
        return Err("Codewall inventory gate exited unsuccessfully".into());
    }
    validate_inventory_receipt(&output.stdout, &input, principal)?;
    Ok(output.stdout)
}

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Route {
    schema_version: u8,
    principal: String,
    supervised_uid: u32,
    gate: PathBuf,
    gate_sha256: String,
    service_socket: PathBuf,
    service_uid: u32,
    installation_id: Uuid,
    enforcer_source_id: Uuid,
}

#[derive(Clone, Debug)]
pub(crate) enum Action {
    Tool { name: String, arguments: Value },
    Prompt { text: String },
}

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Decision {
    pub skip: bool,
    pub ask: bool,
    pub reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct GateReply {
    schema_version: u8,
    event: String,
    decision: Decision,
    context: Option<String>,
}

impl Route {
    fn parse(input: &str, principal: &str, uid: u32) -> Result<Self, String> {
        let route: Self =
            serde_json::from_str(input).map_err(|e| format!("invalid Codewall route: {e}"))?;
        if route.schema_version != 1
            || route.principal != principal
            || route.supervised_uid != uid
            || route.service_uid == 0
            || route.service_uid == uid
            || route.installation_id.is_nil()
            || route.enforcer_source_id.is_nil()
            || !clean_absolute(&route.gate)
            || !clean_absolute(&route.service_socket)
            || route.gate_sha256.len() != 64
            || !route
                .gate_sha256
                .bytes()
                .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
        {
            return Err("Codewall route does not match this installation".into());
        }
        Ok(route)
    }
}

fn clean_absolute(path: &Path) -> bool {
    path.is_absolute()
        && path
            .components()
            .all(|part| matches!(part, Component::RootDir | Component::Normal(_)))
}

#[cfg(target_os = "macos")]
const ROUTE_ROOT: &str = "/private/etc/codewall/aos-routes";
#[cfg(all(unix, not(target_os = "macos")))]
const ROUTE_ROOT: &str = "/etc/codewall/aos-routes";

/// Where root-registered routes live and who must own every directory on the
/// way to them. Production trusts only root from `/` down.
#[cfg(unix)]
struct RouteStore {
    trust_base: PathBuf,
    root: PathBuf,
    owner: u32,
}

#[cfg(unix)]
impl RouteStore {
    fn system() -> Self {
        Self {
            trust_base: PathBuf::from("/"),
            root: PathBuf::from(ROUTE_ROOT),
            owner: 0,
        }
    }

    fn safe_directory(&self, path: &Path, metadata: &fs::Metadata) -> Result<(), String> {
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.uid() != self.owner
            || metadata.mode() & 0o022 != 0
        {
            return Err(format!("unsafe Codewall directory: {}", path.display()));
        }
        Ok(())
    }

    /// Validate every existing directory from the trust base down to `path`.
    /// Returns `false` at the first missing component, after every component
    /// above it was proven safe.
    fn safe_existing_directories(&self, path: &Path) -> Result<bool, String> {
        let mut prefix = PathBuf::new();
        for component in path.components() {
            prefix.push(component.as_os_str());
            if !prefix.starts_with(&self.trust_base) {
                continue;
            }
            match fs::symlink_metadata(&prefix) {
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
                Err(error) => return Err(format!("unsafe Codewall path: {error}")),
                Ok(metadata) => self.safe_directory(&prefix, &metadata)?,
            }
        }
        Ok(true)
    }

    /// The link count is deliberately not checked: on a shared volume a user
    /// can hard-link any readable root-owned file into their home, which must
    /// not disable Codewall. Ownership and mode live on the inode, the parent
    /// directories forbid substitution, and the gate's SHA-256 is pinned.
    fn safe_file(&self, path: &Path, executable: bool) -> Result<(), String> {
        if !clean_absolute(path) {
            return Err("Codewall file path is not absolute and normalized".into());
        }
        let parent = path.parent().ok_or("Codewall file has no parent")?;
        if !self.safe_existing_directories(parent)? {
            return Err(format!("Codewall directory missing: {}", parent.display()));
        }
        let metadata =
            fs::symlink_metadata(path).map_err(|e| format!("Codewall file unavailable: {e}"))?;
        if !metadata.is_file()
            || metadata.file_type().is_symlink()
            || metadata.uid() != self.owner
            || metadata.mode() & 0o022 != 0
            || (executable && metadata.mode() & 0o111 == 0)
        {
            return Err(format!("unsafe Codewall file: {}", path.display()));
        }
        Ok(())
    }

    /// `Ok(None)` only when no protected route directory exists for `uid`.
    /// Once `<root>/<uid>/` exists, every principal must have a valid route.
    fn load(&self, principal: &str, uid: u32) -> Result<Option<Route>, String> {
        if principal.is_empty()
            || principal.len() > 128
            || !principal
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
        {
            return Err("invalid Codewall route principal".into());
        }
        let directory = self.root.join(uid.to_string());
        if !self.safe_existing_directories(&directory)? {
            return Ok(None);
        }
        let path = directory.join(format!("{principal}.json"));
        match fs::symlink_metadata(&path) {
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Err(format!(
                    "Codewall supervises this user but principal {principal} has no route"
                ));
            }
            Err(error) => return Err(format!("Codewall route unavailable: {error}")),
            Ok(_) => {}
        }
        self.safe_file(&path, false)?;
        let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
        if metadata.len() > MAX_ROUTE_BYTES || metadata.mode() & 0o777 != 0o644 {
            return Err("Codewall route must be a small root-owned 0644 file".into());
        }
        let input =
            fs::read_to_string(&path).map_err(|e| format!("Codewall route unreadable: {e}"))?;
        let route = Route::parse(&input, principal, uid)?;
        self.safe_file(&route.gate, true)?;
        let bytes = fs::read(&route.gate).map_err(|e| format!("Codewall gate unreadable: {e}"))?;
        let digest = hex::encode(Sha256::digest(bytes));
        if digest != route.gate_sha256 {
            return Err("Codewall gate digest mismatch".into());
        }
        Ok(Some(route))
    }
}

#[cfg(unix)]
pub(crate) fn load(principal: &str) -> Result<Option<Route>, String> {
    RouteStore::system().load(principal, rustix::process::geteuid().as_raw())
}

#[cfg(not(unix))]
pub(crate) fn load(_principal: &str) -> Result<Option<Route>, String> {
    Ok(None)
}

pub(crate) fn capability(home: &unicity_aos_bootstrap::AosHome) -> Result<Value, String> {
    if !cfg!(unix) {
        return Err("Codewall protected routing requires a Unix host".into());
    }
    let cli = home.runtime_binary();
    let daemon = home.runtime_daemon_binary();
    let capsule = home
        .capsule_dir()
        .map_err(|e| format!("Codewall service capability: AOS capsule assets unavailable: {e}"))?
        .join("aos-cli.capsule");
    for (label, path, executable) in [
        ("runtime CLI", &cli, true),
        ("runtime daemon", &daemon, true),
        ("aos-cli capsule", &capsule, false),
    ] {
        if !clean_absolute(path) {
            return Err(format!(
                "Codewall service capability: {label} path is not absolute"
            ));
        }
        let metadata = fs::symlink_metadata(path)
            .map_err(|e| format!("Codewall service capability: {label} unavailable: {e}"))?;
        if !metadata.is_file() || metadata.file_type().is_symlink() {
            return Err(format!(
                "Codewall service capability: {label} is not a regular file"
            ));
        }
        #[cfg(unix)]
        if executable && metadata.mode() & 0o111 == 0 {
            return Err(format!(
                "Codewall service capability: {label} is not executable"
            ));
        }
        #[cfg(not(unix))]
        let _ = executable;
    }
    Ok(
        json!({"schema_version":1,"protected_routing":true,"product_version":env!("CARGO_PKG_VERSION"),"runtime_cli":cli,"runtime_daemon":daemon,"cli_capsule":capsule}),
    )
}

fn gate_request(action: &Action) -> Value {
    match action {
        Action::Tool { name, arguments } => {
            json!({"schema_version":1,"event":"pre_tool_use","action":{"kind":"tool","name":name,"arguments":arguments}})
        }
        Action::Prompt { text } => {
            json!({"schema_version":1,"event":"user_prompt_submit","action":{"kind":"prompt","text":text}})
        }
    }
}

fn parse_gate_reply(bytes: &[u8], expected_event: &str) -> Result<Decision, String> {
    if bytes.len() > MAX_REPLY_BYTES {
        return Err("Codewall gate reply too large".into());
    }
    let reply: GateReply =
        serde_json::from_slice(bytes).map_err(|e| format!("malformed Codewall reply: {e}"))?;
    if reply.schema_version != 1
        || reply.event != expected_event
        || reply.context.is_some()
        || (reply.decision.skip && reply.decision.ask)
        || reply
            .decision
            .reason
            .as_ref()
            .is_some_and(|reason| reason.len() > 4096)
    {
        return Err("invalid Codewall gate reply".into());
    }
    Ok(reply.decision)
}

pub(crate) async fn evaluate(route: &Route, action: &Action) -> Result<Decision, String> {
    let request = gate_request(action);
    let event = request["event"].as_str().ok_or("missing Codewall event")?;
    let mut command = tokio::process::Command::new(&route.gate);
    command
        .args([
            "--format",
            "oracle-json",
            "--event",
            if event == "pre_tool_use" {
                "pre-tool-use"
            } else {
                "user-prompt-submit"
            },
            "--principal",
            &route.principal,
            "--enforcer-source-id",
            &route.enforcer_source_id.to_string(),
            "--service-socket",
            route.service_socket.to_str().ok_or("invalid socket path")?,
            "--service-uid",
            &route.service_uid.to_string(),
            "--service-installation-id",
            &route.installation_id.to_string(),
        ])
        .env_clear()
        .current_dir("/")
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .kill_on_drop(true);
    let payload = serde_json::to_vec(&request).map_err(|e| e.to_string())?;
    let output = run_gate(command, &payload, GATE_TIMEOUT).await?;
    if !output.success {
        return Err("Codewall gate exited unsuccessfully".into());
    }
    parse_gate_reply(&output.stdout, event)
}

pub(crate) fn unavailable() -> Decision {
    Decision {
        skip: true,
        ask: false,
        reason: Some("Codewall policy unavailable; retry when its service is ready.".into()),
    }
}
