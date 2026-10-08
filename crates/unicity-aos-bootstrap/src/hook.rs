//! Authenticated host-hook ingress for AOS plugins.

use std::io::Read;
use std::time::Duration;

use astrid_core::kernel_api::{KernelRequest, KernelResponse};
use astrid_core::{PrincipalId, SessionId};
use astrid_types::Topic;
use astrid_types::ipc::{IpcMessage, IpcPayload};
use astrid_uplink::{KernelClient, SocketClient};
use clap::{Args, ValueEnum};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

const MAX_PAYLOAD_BYTES: u64 = 1024 * 1024;
const MAX_TIMEOUT_MS: u64 = 30_000;
const DEFAULT_TIMEOUT_MS: u64 = 5_000;

mod output;

fn codewall_action(
    event: &str,
    payload: &Value,
) -> Result<Option<crate::codewall_service::Action>, String> {
    match event {
        "pre_tool_use" | "permission_request" => {
            let name = payload
                .get("tool_name")
                .or_else(|| payload.get("toolName"))
                .and_then(Value::as_str)
                .filter(|s| !s.is_empty())
                .ok_or("Codewall tool name missing")?;
            let arguments = payload
                .get("tool_input")
                .or_else(|| payload.get("toolInput"))
                .filter(|v| v.is_object())
                .cloned()
                .ok_or("Codewall tool arguments missing")?;
            Ok(Some(crate::codewall_service::Action::Tool {
                name: name.to_owned(),
                arguments,
            }))
        }
        "user_prompt_submit" => {
            let text = payload
                .get("prompt")
                .and_then(Value::as_str)
                .ok_or("Codewall prompt missing")?;
            Ok(Some(crate::codewall_service::Action::Prompt {
                text: text.to_owned(),
            }))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod codewall_tests {
    #[test]
    fn codewall_ask_survives_oracle_failure_but_keeps_timely_oracle_reply() {
        let ask = crate::codewall_service::Decision {
            skip: false,
            ask: true,
            reason: Some("review required".into()),
        };
        let recovered = binding_result("pre_tool_use", Some(&ask), Err("runtime stopped".into()))
            .expect("binding fallback")
            .expect("neutral output");
        let parsed: serde_json::Value = serde_json::from_str(&recovered).expect("json");
        assert_eq!(parsed["decision"]["ask"], true);
        assert_eq!(parsed["decision"]["reason"], "review required");
        let oracle_deny = r#"{"decision":{"skip":true,"ask":false,"reason":"other policy"}}"#;
        assert_eq!(
            binding_result("pre_tool_use", Some(&ask), Ok(Some(oracle_deny.into())))
                .expect("oracle reply"),
            Some(oracle_deny.into())
        );
        assert!(binding_result("pre_tool_use", None, Err("runtime stopped".into())).is_err());
    }

    use super::*;
    use serde_json::json;

    #[test]
    fn native_hook_extracts_tool_and_prompt_without_authority_fields() {
        let tool = codewall_action(
            "pre_tool_use",
            &json!({"tool_name":"Bash","tool_input":{"command":"pwd"},"principal_id":"attacker"}),
        )
        .expect("tool");
        assert!(
            matches!(tool, Some(crate::codewall_service::Action::Tool { name, arguments }) if name == "Bash" && arguments == json!({"command":"pwd"}))
        );
        let prompt =
            codewall_action("user_prompt_submit", &json!({"prompt":"hello"})).expect("prompt");
        assert!(
            matches!(prompt, Some(crate::codewall_service::Action::Prompt { text }) if text == "hello")
        );
    }

    #[test]
    fn configured_binding_event_rejects_missing_action_fields() {
        assert!(codewall_action("pre_tool_use", &json!({})).is_err());
        assert!(codewall_action("user_prompt_submit", &json!({"prompt":42})).is_err());
        assert!(
            codewall_action("stop", &json!({}))
                .expect("other event")
                .is_none()
        );
    }
}

#[derive(Debug, Clone, Copy, Default, ValueEnum)]
enum OutputFormat {
    #[default]
    Context,
    Json,
}

/// A private, session-targeted hook delivery from a host plugin.
#[derive(Debug, Args)]
pub struct HookArgs {
    /// Adapter identifier producing this event.
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
    /// Return a host-neutral authenticated reply instead of context text.
    #[arg(long, value_enum, default_value_t = OutputFormat::Context)]
    format: OutputFormat,
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
    #[serde(default)]
    decision: Option<output::Decision>,
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
    let timeout = Duration::from_millis(args.timeout_ms);
    runtime.block_on(async {
        let deadline = tokio::time::Instant::now() + timeout;
        let codewall = match codewall_action(&request.event, &request.payload) {
            Ok(Some(action)) => match crate::codewall_service::load(&request.principal_id) {
                Ok(Some(route)) => {
                    if !matches!(args.format, OutputFormat::Json) {
                        return Err("Codewall binding hooks require --format json".into());
                    }
                    Some(
                        tokio::time::timeout_at(
                            deadline,
                            crate::codewall_service::evaluate(&route, &action),
                        )
                        .await
                        .ok()
                        .and_then(Result::ok)
                        .unwrap_or_else(crate::codewall_service::unavailable),
                    )
                }
                Ok(None) => None,
                Err(_) => Some(crate::codewall_service::unavailable()),
            },
            Err(_) => match crate::codewall_service::load(&request.principal_id) {
                Ok(None) => None,
                _ => Some(crate::codewall_service::unavailable()),
            },
            Ok(None) => None,
        };
        let fallback = codewall.as_ref().filter(|decision| decision.ask).cloned();
        let event = request.event.clone();
        let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
        let result = tokio::time::timeout_at(
            deadline,
            deliver(principal, request, remaining, args.format, codewall),
        )
        .await;
        let result = match result {
            Ok(result) => result,
            Err(_) => Err("hook delivery exceeded its deadline".into()),
        };
        binding_result(&event, fallback.as_ref(), result)
    })
}

fn binding_result(
    event: &str,
    binding_ask: Option<&crate::codewall_service::Decision>,
    result: Result<Option<String>, String>,
) -> Result<Option<String>, String> {
    match result {
        Ok(reply) => Ok(reply),
        Err(error) => {
            let Some(decision) = binding_ask else {
                return Err(error);
            };
            output::reply(
                event,
                Some(&output::Decision {
                    skip: false,
                    ask: true,
                    reason: decision.reason.clone(),
                }),
                None,
            )
        }
    }
}

async fn installed_responder(principal: PrincipalId) -> Result<Uuid, String> {
    // The daemon registry is authoritative. Do not trust a UUID supplied by
    // another capsule, a hook payload, or a disposable filesystem projection.
    let mut client = KernelClient::connect(principal)
        .await
        .map_err(|error| format!("could not discover hook responder: {error}"))?;
    let response = client
        .request(KernelRequest::GetCapsuleMetadata)
        .await
        .map_err(|error| format!("could not read hook responder metadata: {error}"))?;
    match response {
        KernelResponse::CapsuleMetadata(entries) => select_responder(
            entries
                .iter()
                .map(|entry| (entry.name.as_str(), entry.source_id)),
        ),
        _ => Err("runtime did not return hook responder metadata".into()),
    }
}

fn select_responder<'a>(
    entries: impl Iterator<Item = (&'a str, Option<Uuid>)>,
) -> Result<Uuid, String> {
    let mut matches = entries.filter(|(name, _)| *name == "aos-mcp");
    let source = matches.next().and_then(|(_, source)| source);
    if matches.next().is_some() {
        return Err("ambiguous aos-mcp registry identity".into());
    }
    source.ok_or_else(|| "aos-mcp is not loaded for the hook principal".into())
}

async fn deliver(
    principal: PrincipalId,
    request: HostHookRequest,
    timeout: Duration,
    format: OutputFormat,
    codewall: Option<crate::codewall_service::Decision>,
) -> Result<Option<String>, String> {
    // A protected veto remains binding even if the user runtime is stopped.
    if let Some(decision) = &codewall
        && decision.skip
    {
        return output::reply(
            &request.event,
            Some(&output::Decision {
                skip: true,
                ask: false,
                reason: decision.reason.clone(),
            }),
            None,
        );
    }
    let responder_source = if matches!(format, OutputFormat::Json) {
        Some(installed_responder(principal.clone()).await?)
    } else {
        None
    };
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
    if matches!(format, OutputFormat::Json) {
        validate_source(&raw, responder_source)?;
        if response.event.as_deref() != Some(request.event.as_str()) {
            return Err("hook response is missing the exact event".into());
        }
        let combined = codewall.map(|cw| {
            output::combine(
                response.decision.clone(),
                output::Decision {
                    skip: cw.skip,
                    ask: cw.ask,
                    reason: cw.reason,
                },
            )
        });
        return output::reply(
            &request.event,
            combined.as_ref().or(response.decision.as_ref()),
            response.context.as_deref(),
        );
    }
    Ok(response
        .context
        .filter(|context| !context.trim().is_empty()))
}

fn validate_source(raw: &Value, expected: Option<Uuid>) -> Result<(), String> {
    let actual = raw
        .get("source_id")
        .and_then(Value::as_str)
        .and_then(|source| Uuid::parse_str(source).ok());
    if expected.is_some() && actual == expected {
        Ok(())
    } else {
        Err("hook response did not come from the installed aos-mcp source".into())
    }
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
    parse_segment(value)
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
    fn adapter_identifiers_are_not_a_host_product_allowlist() {
        assert_eq!(parse_host("custom_editor").as_deref(), Ok("custom_editor"));
        assert!(parse_host("custom.editor").is_err());
        assert!(parse_host("../editor").is_err());
    }

    #[test]
    fn responder_must_be_one_loaded_registry_entry() {
        let source = Uuid::new_v4();
        assert_eq!(
            select_responder([("aos-mcp", Some(source))].into_iter()).unwrap(),
            source
        );
        assert!(select_responder([("other", Some(source))].into_iter()).is_err());
        assert!(select_responder([("aos-mcp", None)].into_iter()).is_err());
        assert!(
            select_responder([("aos-mcp", Some(source)), ("aos-mcp", None)].into_iter()).is_err()
        );
    }

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
            decision: None,
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

    #[test]
    fn native_response_requires_kernel_stamped_relay_identity() {
        let expected = Uuid::new_v4();
        assert!(
            validate_source(&serde_json::json!({"source_id": expected}), Some(expected)).is_ok()
        );
        assert!(
            validate_source(
                &serde_json::json!({"source_id": Uuid::new_v4()}),
                Some(expected)
            )
            .is_err()
        );
        assert!(
            validate_source(
                &serde_json::json!({"payload": {"source_id": expected}}),
                Some(expected)
            )
            .is_err()
        );
        assert!(validate_source(&serde_json::json!({}), None).is_err());
    }
}
