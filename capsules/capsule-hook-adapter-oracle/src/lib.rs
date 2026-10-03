#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![warn(missing_docs)]

//! Authenticated Oracle frontend hooks to canonical AOS hook events.
//!
//! `capsule-mcp` authenticates the host route and strips its bearer token.
//! This capsule then binds each exact validated topic to its expected frontend,
//! verifies the kernel-stamped principal, translates the frontend event, and
//! returns only the response shape that frontend transport supports.

use astrid_sdk::contracts::hook::HookEventRequest;
use astrid_sdk::prelude::*;
use serde::{Deserialize, Serialize};

mod decision;
use decision::{Decision, RequiredReplies};

const HOST_HOOK_COLLECT_DEADLINE_MS: u64 = 1_000;
const HOOK_QUIESCENCE_MS: u64 = 25;
const MAX_HOST_PAYLOAD_BYTES: usize = 1024 * 1024;
const MAX_CANONICAL_EVENT_BYTES: usize = 1024 * 1024;
const MAX_HOST_CONTEXT_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Frontend {
    Codex,
    Claude,
    Grok,
}

impl Frontend {
    const fn name(self) -> &'static str {
        match self {
            Self::Codex => "codex",
            Self::Claude => "claude",
            Self::Grok => "grok",
        }
    }

    fn mapping(self, event: &str) -> Option<HookMapping> {
        // Separate tables are deliberate. The upstream plugins normalize onto
        // common names today, but each frontend may evolve independently
        // without weakening another adapter's accepted surface.
        match self {
            Self::Codex => codex_mapping(event),
            Self::Claude => claude_mapping(event),
            Self::Grok => grok_mapping(event),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ResponseMode {
    /// Publish the canonical event but do not solicit a reply.
    Observe,
    /// Require a verdict from every configured policy source before returning.
    Binding,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct HookMapping {
    hook: &'static str,
    response: ResponseMode,
}

const fn observe(hook: &'static str) -> HookMapping {
    HookMapping {
        hook,
        response: ResponseMode::Observe,
    }
}

fn common_mapping(event: &str) -> Option<HookMapping> {
    match event {
        "session_start" => Some(observe("session_start")),
        "session_end" => Some(observe("session_end")),
        "post_tool_use" => Some(observe("after_tool_call")),
        "pre_compact" => Some(observe("on_compaction_started")),
        "post_compact" => Some(observe("on_compaction_completed")),
        "subagent_start" => Some(observe("subagent_start")),
        "subagent_stop" => Some(observe("subagent_stop")),
        _ => None,
    }
}

const fn binding(hook: &'static str) -> HookMapping {
    HookMapping {
        hook,
        response: ResponseMode::Binding,
    }
}

fn codex_mapping(event: &str) -> Option<HookMapping> {
    match event {
        "pre_tool_use" | "permission_request" => Some(binding("before_tool_call")),
        "user_prompt_submit" => Some(binding("message_received")),
        // Codex Stop is per-turn and carries `last_assistant_message`.
        "stop" => Some(observe("message_sent")),
        _ => common_mapping(event),
    }
}

fn claude_mapping(event: &str) -> Option<HookMapping> {
    match event {
        "pre_tool_use" | "permission_request" => Some(binding("before_tool_call")),
        "user_prompt_submit" => Some(binding("message_received")),
        // Claude Stop is per-turn and carries `last_assistant_message`.
        "stop" => Some(observe("message_sent")),
        // Claude MessageDisplay carries response text in `delta` while it is
        // rendered. It is observation-only on this relay.
        "message_display" => Some(observe("message_displayed")),
        _ => common_mapping(event),
    }
}

fn grok_mapping(event: &str) -> Option<HookMapping> {
    match event {
        "pre_tool_use" => Some(binding("before_tool_call")),
        "user_prompt_submit" => Some(binding("message_received")),
        // Grok has PermissionDenied observations, not PermissionRequest.
        "permission_request" => None,
        // Grok Stop completes a turn, not the session. Retiring the route here
        // loses authentication for subsequent events in that same session.
        "stop" => Some(observe("message_sent")),
        _ => common_mapping(event),
    }
}

#[derive(Debug, Deserialize)]
struct OracleHookEvent {
    schema_version: u8,
    principal_id: String,
    host: String,
    session_id: String,
    event: String,
    correlation_id: String,
    route_id: String,
    delivery_id: String,
    #[serde(default)]
    turn_id: Option<String>,
    #[serde(default)]
    workspace_id: Option<String>,
    payload: serde_json::Value,
}

#[derive(Debug, Serialize)]
struct OracleHookResponse<'a> {
    schema_version: u8,
    principal_id: &'a str,
    host: &'a str,
    session_id: &'a str,
    /// Canonical lifecycle/observation classification selected by this
    /// frontend adapter. The source frontend event remains in `event`.
    canonical_hook: &'a str,
    event: &'a str,
    correlation_id: &'a str,
    route_id: &'a str,
    delivery_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    decision: Option<Decision>,
}

#[derive(Debug, Serialize)]
struct CanonicalOraclePayload<'a> {
    principal_id: &'a str,
    host: &'a str,
    session_id: &'a str,
    source_event: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    turn_id: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    workspace_id: Option<&'a str>,
    payload: &'a serde_json::Value,
}

fn is_clean_segment(value: &str, max: usize) -> bool {
    !value.is_empty()
        && value.len() <= max
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
}

fn is_lower_hex(value: &str, expected_len: usize) -> bool {
    value.len() == expected_len
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn validate_oracle_hook(
    expected: Frontend,
    event: &OracleHookEvent,
) -> Result<HookMapping, &'static str> {
    if event.schema_version != 1 {
        return Err("unsupported schema version");
    }
    // Bind the payload host to the exact topic handler. A valid `claude`
    // envelope delivered on the Codex topic is still invalid.
    if event.host != expected.name() {
        return Err("host does not match validated topic");
    }
    let Some(mapping) = expected.mapping(&event.event) else {
        return Err("unsupported host event");
    };
    if !is_clean_segment(&event.session_id, 128)
        || !is_clean_segment(&event.event, 128)
        || !is_clean_segment(&event.delivery_id, 128)
    {
        return Err("invalid routed segment");
    }
    if !is_lower_hex(&event.route_id, 64) || !is_lower_hex(&event.correlation_id, 32) {
        return Err("invalid route identifier");
    }
    if event.delivery_id != format!("{}-{}", event.route_id, event.correlation_id) {
        return Err("delivery identifier does not bind route and correlation");
    }
    if event
        .turn_id
        .as_deref()
        .is_some_and(|value| value.is_empty() || value.len() > 256)
        || event
            .workspace_id
            .as_deref()
            .is_some_and(|value| !is_clean_segment(value, 128))
    {
        return Err("invalid optional routing metadata");
    }
    if serde_json::to_vec(&event.payload)
        .map_or(true, |payload| payload.len() > MAX_HOST_PAYLOAD_BYTES)
    {
        return Err("host payload exceeds limit");
    }
    Ok(mapping)
}

fn push_context(contexts: &mut Vec<String>, total: &mut usize, context: &str) -> bool {
    let separator = usize::from(!contexts.is_empty()) * 2;
    let Some(next_total) = total
        .checked_add(separator)
        .and_then(|value| value.checked_add(context.len()))
    else {
        return false;
    };
    if next_total > MAX_HOST_CONTEXT_BYTES {
        return false;
    }
    contexts.push(context.to_owned());
    *total = next_total;
    true
}

fn collect_additional_context(
    subscription: &ipc::Subscription,
    reply_topic: &str,
    principal: &str,
) -> Result<Option<String>, SysError> {
    let mut contexts = Vec::new();
    let mut context_bytes = 0;
    let start = time::monotonic();
    loop {
        let elapsed_ms = u64::try_from((time::monotonic().saturating_sub(start)).as_millis())
            .unwrap_or(HOST_HOOK_COLLECT_DEADLINE_MS);
        if elapsed_ms >= HOST_HOOK_COLLECT_DEADLINE_MS {
            break;
        }
        let remaining = if contexts.is_empty() {
            HOST_HOOK_COLLECT_DEADLINE_MS - elapsed_ms
        } else {
            HOOK_QUIESCENCE_MS.min(HOST_HOOK_COLLECT_DEADLINE_MS - elapsed_ms)
        };
        match subscription.recv(remaining) {
            Ok(poll) if poll.messages.is_empty() => break,
            Ok(poll) => {
                if poll.dropped != 0 || poll.lagged != 0 {
                    log::warn(format!(
                        "hook-adapter-oracle: incomplete context fan-out on {reply_topic}; dropping all partial context"
                    ));
                    return Ok(None);
                }
                for message in poll.messages {
                    if message.topic != reply_topic
                        || message.principal.verified() != Some(principal)
                    {
                        log::warn(format!(
                            "hook-adapter-oracle: dropping mismatched context reply on {reply_topic}"
                        ));
                        continue;
                    }
                    match serde_json::from_str::<serde_json::Value>(&message.payload) {
                        Ok(value) => {
                            if let Some(context) = value
                                .get("additional_context")
                                .and_then(serde_json::Value::as_str)
                                .filter(|context| !context.trim().is_empty())
                                && !push_context(&mut contexts, &mut context_bytes, context)
                            {
                                log::warn(format!(
                                    "hook-adapter-oracle: dropping context beyond {MAX_HOST_CONTEXT_BYTES} bytes"
                                ));
                            }
                        }
                        Err(error) => log::warn(format!(
                            "hook-adapter-oracle: dropping malformed reply on {reply_topic}: {error}"
                        )),
                    }
                }
            }
            Err(SysError::HostError(message)) if message.contains("Timeout") => break,
            Err(error) => return Err(error),
        }
    }
    if contexts.is_empty() {
        Ok(None)
    } else {
        Ok(Some(contexts.join("\n\n")))
    }
}

fn canonical_request(
    event: &OracleHookEvent,
    mapping: HookMapping,
) -> Result<HookEventRequest, SysError> {
    let payload = CanonicalOraclePayload {
        principal_id: &event.principal_id,
        host: &event.host,
        session_id: &event.session_id,
        source_event: &event.event,
        turn_id: event.turn_id.as_deref(),
        workspace_id: event.workspace_id.as_deref(),
        payload: &event.payload,
    };
    let request = HookEventRequest {
        hook: mapping.hook.to_owned(),
        payload: serde_json::to_string(&payload)?,
        correlation_id: (!matches!(mapping.response, ResponseMode::Observe))
            .then(|| event.correlation_id.clone()),
    };
    if serde_json::to_vec(&request)?.len() > MAX_CANONICAL_EVENT_BYTES {
        return Err(SysError::HostError(
            "canonical hook event exceeds IPC payload limit".to_owned(),
        ));
    }
    Ok(request)
}

fn dispatch_oracle_hook(
    event: &OracleHookEvent,
    mapping: HookMapping,
) -> Result<(Option<String>, Option<Decision>), SysError> {
    let event_topic = format!("hook.v1.event.{}", mapping.hook);
    let request = canonical_request(event, mapping)?;
    if matches!(mapping.response, ResponseMode::Observe) {
        ipc::publish_json(&event_topic, &request)?;
        return Ok((None, None));
    }

    let reply_topic = format!("hook.v1.response.{}.{}", mapping.hook, event.correlation_id);
    let subscription = ipc::subscribe(&reply_topic)?;
    ipc::publish_json(&event_topic, &request)?;
    collect_binding_response(&subscription, &reply_topic, &event.principal_id)
}

fn collect_binding_response(
    subscription: &ipc::Subscription,
    reply_topic: &str,
    principal: &str,
) -> Result<(Option<String>, Option<Decision>), SysError> {
    // Read the invocation's principal overlay, never a process-global cache.
    let config = match env::var("AOS_ORACLE_REQUIRED_HOOK_SOURCES") {
        Ok(config) => config,
        Err(_) => return Ok((None, Some(Decision::unavailable()))),
    };
    let Ok(mut replies) = RequiredReplies::parse(&config) else {
        return Ok((None, Some(Decision::unavailable())));
    };
    if replies.complete() {
        return collect_additional_context(subscription, reply_topic, principal)
            .map(|context| (context, Some(Decision::default())));
    }
    let mut contexts = Vec::new();
    let mut context_bytes = 0;
    let start = time::monotonic();
    loop {
        let elapsed = u64::try_from(time::monotonic().saturating_sub(start).as_millis())
            .unwrap_or(HOST_HOOK_COLLECT_DEADLINE_MS);
        let Some(wait) =
            replies.next_wait(elapsed, HOST_HOOK_COLLECT_DEADLINE_MS, HOOK_QUIESCENCE_MS)
        else {
            break;
        };
        let poll = match subscription.recv(wait) {
            Ok(poll) if poll.messages.is_empty() => break,
            Ok(poll) if poll.dropped == 0 && poll.lagged == 0 => poll,
            _ => return Ok((None, Some(Decision::unavailable()))),
        };
        for message in poll.messages {
            if message.topic != reply_topic || message.principal.verified() != Some(principal) {
                continue;
            }
            replies.accept(&message.source_id, &message.payload);
            if let Ok(value) = serde_json::from_str::<serde_json::Value>(&message.payload)
                && let Some(context) = value
                    .get("additional_context")
                    .and_then(serde_json::Value::as_str)
            {
                push_context(&mut contexts, &mut context_bytes, context);
            }
        }
    }
    Ok((
        (!contexts.is_empty()).then(|| contexts.join("\n\n")),
        Some(replies.finish()),
    ))
}

fn handle_oracle_hook(expected: Frontend, payload: serde_json::Value) -> Result<(), SysError> {
    let event: OracleHookEvent = match serde_json::from_value(payload) {
        Ok(event) => event,
        Err(error) => {
            log::warn(format!(
                "hook-adapter-oracle: dropping malformed {} hook: {error}",
                expected.name()
            ));
            return Ok(());
        }
    };
    let mapping = match validate_oracle_hook(expected, &event) {
        Ok(mapping) => mapping,
        Err(reason) => {
            log::warn(format!(
                "hook-adapter-oracle: dropping invalid {} hook '{}': {reason}",
                expected.name(),
                event.event
            ));
            return Ok(());
        }
    };
    let caller = runtime::caller()?;
    if caller.principal.as_deref() != Some(event.principal_id.as_str()) {
        log::warn(format!(
            "hook-adapter-oracle: dropping principal mismatch for {}",
            expected.name()
        ));
        return Ok(());
    }

    let (context, decision) = dispatch_oracle_hook(&event, mapping)?;
    ipc::publish_json(
        &format!("oracle.v1.hook.response.{}", event.delivery_id),
        &OracleHookResponse {
            schema_version: 1,
            principal_id: &event.principal_id,
            host: &event.host,
            session_id: &event.session_id,
            canonical_hook: mapping.hook,
            event: &event.event,
            correlation_id: &event.correlation_id,
            route_id: &event.route_id,
            delivery_id: &event.delivery_id,
            context,
            decision,
        },
    )
}

/// Oracle hook protocol adapter.
#[derive(Default)]
pub struct OracleHookAdapter;

#[capsule]
impl OracleHookAdapter {
    /// Translate a token-validated Codex hook.
    #[astrid::interceptor("on_codex_hook")]
    pub fn on_codex_hook(&self, payload: serde_json::Value) -> Result<(), SysError> {
        handle_oracle_hook(Frontend::Codex, payload)
    }

    /// Translate a token-validated Claude hook.
    #[astrid::interceptor("on_claude_hook")]
    pub fn on_claude_hook(&self, payload: serde_json::Value) -> Result<(), SysError> {
        handle_oracle_hook(Frontend::Claude, payload)
    }

    /// Translate a token-validated Grok hook.
    #[astrid::interceptor("on_grok_hook")]
    pub fn on_grok_hook(&self, payload: serde_json::Value) -> Result<(), SysError> {
        handle_oracle_hook(Frontend::Grok, payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host_event(host: &str) -> OracleHookEvent {
        let route_id = "a".repeat(64);
        let correlation_id = "b".repeat(32);
        OracleHookEvent {
            schema_version: 1,
            principal_id: "codex-code".to_owned(),
            host: host.to_owned(),
            session_id: "codex-session".to_owned(),
            event: "user_prompt_submit".to_owned(),
            delivery_id: format!("{route_id}-{correlation_id}"),
            correlation_id,
            route_id,
            turn_id: Some("turn-one".to_owned()),
            workspace_id: Some("workspace-one".to_owned()),
            payload: serde_json::json!({"prompt": "hello"}),
        }
    }

    #[test]
    fn each_validated_topic_binds_its_exact_host() {
        assert!(validate_oracle_hook(Frontend::Codex, &host_event("codex")).is_ok());
        assert_eq!(
            validate_oracle_hook(Frontend::Codex, &host_event("claude")),
            Err("host does not match validated topic")
        );
    }

    #[test]
    fn claude_prompt_and_pretool_carry_binding_decisions() {
        assert_eq!(
            Frontend::Claude.mapping("user_prompt_submit"),
            Some(HookMapping {
                hook: "message_received",
                response: ResponseMode::Binding
            })
        );
        assert_eq!(
            Frontend::Claude.mapping("pre_tool_use"),
            Some(HookMapping {
                hook: "before_tool_call",
                response: ResponseMode::Binding
            })
        );
    }

    #[test]
    fn every_host_collects_prompt_and_pretool_verdicts() {
        for host in [Frontend::Codex, Frontend::Claude, Frontend::Grok] {
            assert_eq!(
                host.mapping("pre_tool_use"),
                Some(binding("before_tool_call"))
            );
            assert_eq!(
                host.mapping("user_prompt_submit"),
                Some(binding("message_received"))
            );
        }
        for host in [Frontend::Codex, Frontend::Claude] {
            assert_eq!(
                host.mapping("permission_request"),
                Some(binding("before_tool_call"))
            );
        }
        assert_eq!(Frontend::Grok.mapping("permission_request"), None);
    }

    #[test]
    fn codex_and_claude_stop_are_response_events_not_session_termination() {
        assert_eq!(
            Frontend::Codex.mapping("stop"),
            Some(observe("message_sent"))
        );
        assert_eq!(
            Frontend::Claude.mapping("stop"),
            Some(observe("message_sent"))
        );
        assert_eq!(
            Frontend::Codex.mapping("session_end"),
            Some(observe("session_end"))
        );
        assert_eq!(
            Frontend::Claude.mapping("session_end"),
            Some(observe("session_end"))
        );
    }

    #[test]
    fn grok_stop_preserves_the_session_route() {
        assert_eq!(
            Frontend::Grok.mapping("stop"),
            Some(observe("message_sent"))
        );
        assert_eq!(
            Frontend::Grok.mapping("session_end"),
            Some(observe("session_end"))
        );
    }

    #[test]
    fn claude_message_display_is_response_egress_only() {
        assert_eq!(
            Frontend::Claude.mapping("message_display"),
            Some(observe("message_displayed"))
        );
        assert_eq!(Frontend::Codex.mapping("message_display"), None);
    }

    #[test]
    fn canonical_events_retain_user_prompt_and_assistant_response_text() {
        let mut prompt = host_event("codex");
        prompt.payload = serde_json::json!({"prompt": "inspect this input"});
        let request = canonical_request(
            &prompt,
            Frontend::Codex.mapping("user_prompt_submit").unwrap(),
        )
        .unwrap();
        let payload: serde_json::Value = serde_json::from_str(&request.payload).unwrap();
        assert_eq!(payload["source_event"], "user_prompt_submit");
        assert_eq!(payload["payload"]["prompt"], "inspect this input");

        let mut response = host_event("codex");
        response.event = "stop".to_owned();
        response.payload = serde_json::json!({
            "last_assistant_message": "the final assistant response"
        });
        let request =
            canonical_request(&response, Frontend::Codex.mapping("stop").unwrap()).unwrap();
        assert_eq!(request.hook, "message_sent");
        let payload: serde_json::Value = serde_json::from_str(&request.payload).unwrap();
        assert_eq!(payload["source_event"], "stop");
        assert_eq!(
            payload["payload"]["last_assistant_message"],
            "the final assistant response"
        );
    }

    #[test]
    fn canonical_claude_display_event_retains_streamed_delta() {
        let mut event = host_event("claude");
        event.event = "message_display".to_owned();
        event.payload = serde_json::json!({
            "message_id": "message-one",
            "index": 0,
            "final": true,
            "delta": "rendered assistant response"
        });
        let request =
            canonical_request(&event, Frontend::Claude.mapping("message_display").unwrap())
                .unwrap();
        assert_eq!(request.hook, "message_displayed");
        let payload: serde_json::Value = serde_json::from_str(&request.payload).unwrap();
        assert_eq!(payload["payload"]["delta"], "rendered assistant response");
    }

    #[test]
    fn delivery_binds_route_and_correlation() {
        let mut event = host_event("grok");
        assert!(validate_oracle_hook(Frontend::Grok, &event).is_ok());
        event.delivery_id = format!("{}-{}", "c".repeat(64), event.correlation_id);
        assert_eq!(
            validate_oracle_hook(Frontend::Grok, &event),
            Err("delivery identifier does not bind route and correlation")
        );
    }

    #[test]
    fn event_and_session_cannot_add_topic_segments() {
        let mut event = host_event("codex");
        event.session_id = "codex.other".to_owned();
        assert_eq!(
            validate_oracle_hook(Frontend::Codex, &event),
            Err("invalid routed segment")
        );
    }

    #[test]
    fn combined_context_stays_inside_relay_limit() {
        let mut contexts = Vec::new();
        let mut total = 0;
        assert!(push_context(
            &mut contexts,
            &mut total,
            &"a".repeat(MAX_HOST_CONTEXT_BYTES - 3)
        ));
        assert!(push_context(&mut contexts, &mut total, "b"));
        assert_eq!(contexts.join("\n\n").len(), MAX_HOST_CONTEXT_BYTES);
        assert!(!push_context(&mut contexts, &mut total, "c"));
    }

    #[test]
    fn canonical_request_keeps_observation_uncorrelated() {
        let event = host_event("codex");
        let request = canonical_request(&event, observe("before_tool_call")).unwrap();
        assert!(request.correlation_id.is_none());
        let request = canonical_request(&event, binding("message_received")).unwrap();
        assert_eq!(request.correlation_id, Some(event.correlation_id));
    }
}
