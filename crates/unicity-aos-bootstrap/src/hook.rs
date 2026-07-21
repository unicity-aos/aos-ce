//! Authenticated host-hook ingress for AOS plugins.

use std::io::Read;
use std::time::Duration;

use astrid_core::{PrincipalId, SessionId};
use astrid_types::Topic;
use astrid_types::ipc::{IpcMessage, IpcPayload};
use astrid_uplink::SocketClient;
use clap::Args;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

const MAX_PAYLOAD_BYTES: u64 = 1024 * 1024;
const MAX_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_TIMEOUT_MS: u64 = 5_000;

/// A private, session-targeted hook delivery from a host plugin.
#[derive(Debug, Args)]
pub struct HookArgs {
    /// Host adapter producing this event: codex, claude, or grok.
    #[arg(long, value_parser = parse_host)]
    host: String,
    /// Exact host session receiving any returned context.
    #[arg(long, value_parser = parse_segment)]
    session: String,
    /// Normalized host event name.
    #[arg(long, value_parser = parse_segment)]
    event: String,
    /// Optional workspace identifier for observability and future routing.
    #[arg(long, value_parser = parse_segment)]
    workspace: Option<String>,
    /// Maximum time to wait for a targeted hook response.
    #[arg(long, default_value_t = DEFAULT_TIMEOUT_MS, value_parser = parse_timeout)]
    timeout_ms: u64,
}

#[derive(Debug, Serialize)]
struct HostHookRequest {
    schema_version: u8,
    principal_id: String,
    host: String,
    session_id: String,
    event: String,
    correlation_id: String,
    route_id: String,
    delivery_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    turn_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_id: Option<String>,
    payload: Value,
    token: String,
}

#[derive(Debug, Deserialize)]
struct HostHookResponse {
    schema_version: u8,
    principal_id: String,
    host: String,
    session_id: String,
    #[serde(default)]
    event: Option<String>,
    correlation_id: String,
    route_id: String,
    delivery_id: String,
    #[serde(default)]
    context: Option<String>,
}

pub(crate) fn handle(principal: String, args: HookArgs) -> Result<Option<String>, String> {
    let token = std::env::var("ASTRID_HOOK_TOKEN")
        .map_err(|_| "ASTRID_HOOK_TOKEN is required for `aos hook`".to_owned())?;
    validate_token(&token)?;

    let mut input = Vec::new();
    std::io::stdin()
        .take(MAX_PAYLOAD_BYTES + 1)
        .read_to_end(&mut input)
        .map_err(|error| format!("could not read hook payload: {error}"))?;
    if input.len() as u64 > MAX_PAYLOAD_BYTES {
        return Err(format!(
            "hook payload exceeds the {MAX_PAYLOAD_BYTES}-byte limit"
        ));
    }
    let payload = if input.iter().all(u8::is_ascii_whitespace) {
        Value::Object(serde_json::Map::new())
    } else {
        serde_json::from_slice(&input)
            .map_err(|error| format!("hook payload is not valid JSON: {error}"))?
    };

    let principal = PrincipalId::new(principal.clone())
        .map_err(|error| format!("invalid hook principal: {error}"))?;
    let correlation_id = Uuid::new_v4().simple().to_string();
    let route_id = derive_route_id(&args.host, &args.session, &token);
    let delivery_id = delivery_id(&route_id, &correlation_id);
    let turn_id = payload
        .get("turn_id")
        .or_else(|| payload.get("turnId"))
        .and_then(value_as_identifier);

    let request = HostHookRequest {
        schema_version: 1,
        principal_id: principal.to_string(),
        host: args.host.clone(),
        session_id: args.session.clone(),
        event: args.event,
        correlation_id,
        route_id,
        delivery_id,
        turn_id,
        workspace_id: args.workspace,
        payload,
        token,
    };

    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|error| format!("could not start hook client: {error}"))?;
    runtime.block_on(deliver(
        principal,
        request,
        Duration::from_millis(args.timeout_ms),
    ))
}

async fn deliver(
    principal: PrincipalId,
    request: HostHookRequest,
    timeout: Duration,
) -> Result<Option<String>, String> {
    let connection_id = Uuid::new_v4();
    let mut client = SocketClient::connect(SessionId::from_uuid(connection_id), principal.clone())
        .await
        .map_err(|error| format!("could not connect to the AOS runtime: {error}"))?;
    if !client.is_authenticated() {
        return Err(format!(
            "the AOS runtime did not authenticate principal {principal}"
        ));
    }

    let ingress_topic = format!("astrid.v1.request.mcp.hook.{}", request.host);
    let response_topic = format!("astrid.v1.response.{}", request.delivery_id);
    let message = IpcMessage::new(
        Topic::from_raw(ingress_topic),
        IpcPayload::RawJson(
            serde_json::to_value(&request)
                .map_err(|error| format!("could not encode hook request: {error}"))?,
        ),
        connection_id,
    );
    client
        .send_message(message)
        .await
        .map_err(|error| format!("could not publish hook request: {error}"))?;

    let raw = client
        .read_until_topic(&response_topic, timeout)
        .await
        .map_err(|error| format!("hook response unavailable: {error}"))?;
    let response: HostHookResponse = serde_json::from_value(extract_raw_payload(&raw)?)
        .map_err(|error| format!("hook response is malformed: {error}"))?;
    validate_response(&request, &response)?;
    Ok(response
        .context
        .filter(|context| !context.trim().is_empty()))
}

fn extract_raw_payload(raw: &Value) -> Result<Value, String> {
    let payload = raw
        .get("payload")
        .ok_or_else(|| "hook response has no payload".to_owned())?;
    if payload.get("type").and_then(Value::as_str) == Some("raw_json") {
        payload
            .get("value")
            .cloned()
            .ok_or_else(|| "hook response raw payload has no value".to_owned())
    } else {
        Ok(payload.clone())
    }
}

fn validate_response(request: &HostHookRequest, response: &HostHookResponse) -> Result<(), String> {
    let matches = response.schema_version == request.schema_version
        && response.principal_id == request.principal_id
        && response.host == request.host
        && response.session_id == request.session_id
        && response
            .event
            .as_deref()
            .is_none_or(|event| event == request.event)
        && response.correlation_id == request.correlation_id
        && response.route_id == request.route_id
        && response.delivery_id == request.delivery_id;
    if matches {
        Ok(())
    } else {
        Err("hook response did not match the exact host session route".to_owned())
    }
}

fn derive_route_id(host: &str, session: &str, token: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"unicity-aos-hook-route-v1\0");
    hasher.update(host.as_bytes());
    hasher.update(b"\0");
    hasher.update(session.as_bytes());
    hasher.update(b"\0");
    hasher.update(token.as_bytes());
    hasher.finalize().to_hex().to_string()
}

fn delivery_id(route_id: &str, correlation_id: &str) -> String {
    format!("{route_id}-{correlation_id}")
}

fn value_as_identifier(value: &Value) -> Option<String> {
    match value {
        Value::String(value) if !value.is_empty() => Some(value.clone()),
        Value::Number(value) => Some(value.to_string()),
        _ => None,
    }
}

fn parse_host(value: &str) -> Result<String, String> {
    match value {
        "codex" | "claude" | "grok" => Ok(value.to_owned()),
        _ => Err("host must be codex, claude, or grok".to_owned()),
    }
}

fn parse_segment(value: &str) -> Result<String, String> {
    if is_segment(value) {
        Ok(value.to_owned())
    } else {
        Err("expected 1-128 ASCII letters, digits, underscores, or hyphens".to_owned())
    }
}

fn is_segment(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn validate_token(token: &str) -> Result<(), String> {
    if token.len() < 32
        || token.len() > 128
        || !token.bytes().all(|byte| byte.is_ascii_alphanumeric())
    {
        return Err(
            "ASTRID_HOOK_TOKEN must be 32-128 ASCII letters or digits with no whitespace"
                .to_owned(),
        );
    }
    Ok(())
}

fn parse_timeout(value: &str) -> Result<u64, String> {
    let timeout = value
        .parse::<u64>()
        .map_err(|_| "timeout must be an integer number of milliseconds".to_owned())?;
    if timeout == 0 || timeout > MAX_TIMEOUT_MS {
        return Err(format!("timeout must be between 1 and {MAX_TIMEOUT_MS} ms"));
    }
    Ok(timeout)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn route_is_stable_and_secret_bound() {
        let first = derive_route_id("codex", "codex-session", &"a".repeat(64));
        assert_eq!(
            first,
            derive_route_id("codex", "codex-session", &"a".repeat(64))
        );
        assert_ne!(
            first,
            derive_route_id("codex", "codex-session", &"b".repeat(64))
        );
        assert_ne!(
            first,
            derive_route_id("codex", "other-session", &"a".repeat(64))
        );
    }

    #[test]
    fn delivery_topic_has_one_dynamic_segment() {
        let delivery = delivery_id(&"a".repeat(64), &"b".repeat(32));
        assert!(is_segment(&delivery));
        assert_eq!(delivery.split('.').count(), 1);
    }

    #[test]
    fn response_must_match_every_routing_dimension() {
        let request = HostHookRequest {
            schema_version: 1,
            principal_id: "codex-code".to_owned(),
            host: "codex".to_owned(),
            session_id: "codex-one".to_owned(),
            event: "user_prompt_submit".to_owned(),
            correlation_id: "correlation".to_owned(),
            route_id: "route".to_owned(),
            delivery_id: "delivery".to_owned(),
            turn_id: None,
            workspace_id: None,
            payload: serde_json::json!({}),
            token: "a".repeat(64),
        };
        let mut response = HostHookResponse {
            schema_version: 1,
            principal_id: request.principal_id.clone(),
            host: request.host.clone(),
            session_id: request.session_id.clone(),
            event: Some(request.event.clone()),
            correlation_id: request.correlation_id.clone(),
            route_id: request.route_id.clone(),
            delivery_id: request.delivery_id.clone(),
            context: Some("context".to_owned()),
        };
        assert!(validate_response(&request, &response).is_ok());
        response.event = None;
        assert!(validate_response(&request, &response).is_ok());
        response.session_id = "codex-two".to_owned();
        assert!(validate_response(&request, &response).is_err());
    }

    #[test]
    fn topic_segments_reject_smuggling() {
        assert!(parse_segment("codex-session_1").is_ok());
        assert!(parse_segment("codex.session").is_err());
        assert!(parse_segment("../session").is_err());
        assert!(parse_segment("").is_err());
    }
}
