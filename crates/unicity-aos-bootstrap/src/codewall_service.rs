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
use tokio::io::AsyncWriteExt;
use uuid::Uuid;

const MAX_ROUTE_BYTES: u64 = 8192;
const MAX_REPLY_BYTES: usize = 8192;
const GATE_TIMEOUT: Duration = Duration::from_secs(7);
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

async fn run_gate(
    mut command: tokio::process::Command,
    payload: &[u8],
    deadline: Duration,
) -> Result<std::process::Output, String> {
    tokio::time::timeout(deadline, async {
        let mut child = command
            .spawn()
            .map_err(|e| format!("Codewall gate unavailable: {e}"))?;
        let mut stdin = child
            .stdin
            .take()
            .ok_or("Codewall gate input unavailable")?;
        stdin
            .write_all(payload)
            .await
            .map_err(|e| format!("Codewall gate input failed: {e}"))?;
        drop(stdin);
        child
            .wait_with_output()
            .await
            .map_err(|e| format!("Codewall gate failed: {e}"))
    })
    .await
    .map_err(|_| "Codewall gate timed out".to_owned())?
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
    if !output.status.success() {
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

#[cfg(unix)]
fn safe_ancestors(path: &Path) -> Result<(), String> {
    let mut prefix = PathBuf::new();
    for component in path
        .parent()
        .ok_or("Codewall route has no parent")?
        .components()
    {
        prefix.push(component.as_os_str());
        let metadata =
            fs::symlink_metadata(&prefix).map_err(|e| format!("unsafe Codewall path: {e}"))?;
        if !metadata.is_dir()
            || metadata.file_type().is_symlink()
            || metadata.uid() != 0
            || metadata.mode() & 0o022 != 0
        {
            return Err(format!("unsafe Codewall directory: {}", prefix.display()));
        }
    }
    Ok(())
}

#[cfg(unix)]
fn safe_file(path: &Path, executable: bool) -> Result<(), String> {
    if !clean_absolute(path) {
        return Err("Codewall file path is not absolute and normalized".into());
    }
    safe_ancestors(path)?;
    let metadata =
        fs::symlink_metadata(path).map_err(|e| format!("Codewall file unavailable: {e}"))?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.uid() != 0
        || metadata.nlink() != 1
        || metadata.mode() & 0o022 != 0
        || (executable && metadata.mode() & 0o111 == 0)
    {
        return Err(format!("unsafe Codewall file: {}", path.display()));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
const ROUTE_ROOT: &str = "/private/etc/codewall/aos-routes";
#[cfg(all(unix, not(target_os = "macos")))]
const ROUTE_ROOT: &str = "/etc/codewall/aos-routes";

#[cfg(unix)]
pub(crate) fn load(principal: &str) -> Result<Option<Route>, String> {
    let uid = rustix::process::geteuid().as_raw();
    if principal.is_empty()
        || principal.len() > 128
        || !principal
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-'))
    {
        return Err("invalid Codewall route principal".into());
    }
    let path = Path::new(ROUTE_ROOT)
        .join(uid.to_string())
        .join(format!("{principal}.json"));
    match fs::symlink_metadata(&path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("Codewall route unavailable: {error}")),
        Ok(_) => {}
    }
    safe_file(&path, false)?;
    let metadata = fs::metadata(&path).map_err(|e| e.to_string())?;
    if metadata.len() > MAX_ROUTE_BYTES || metadata.mode() & 0o777 != 0o644 {
        return Err("Codewall route must be a small root-owned 0644 file".into());
    }
    let input = fs::read_to_string(&path).map_err(|e| format!("Codewall route unreadable: {e}"))?;
    let route = Route::parse(&input, principal, uid)?;
    safe_file(&route.gate, true)?;
    let bytes = fs::read(&route.gate).map_err(|e| format!("Codewall gate unreadable: {e}"))?;
    let digest = hex::encode(Sha256::digest(bytes));
    if digest != route.gate_sha256 {
        return Err("Codewall gate digest mismatch".into());
    }
    Ok(Some(route))
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
    if !output.status.success() {
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
