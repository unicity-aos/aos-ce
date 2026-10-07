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
use sha2::{Digest, Sha256};

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
    /// Collect optional context without turning an observation into authority.
    Context,
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
        "session_start" => Some(context("session_start")),
        "session_end" => Some(observe("session_end")),
        "post_tool_use" => Some(binding("after_tool_call")),
        "pre_compact" => Some(observe("on_compaction_started")),
        "post_compact" => Some(observe("on_compaction_completed")),
        "subagent_start" => Some(context("subagent_start")),
        "subagent_stop" => Some(binding("subagent_stop")),
        _ => None,
    }
}

const fn context(hook: &'static str) -> HookMapping {
    HookMapping {
        hook,
        response: ResponseMode::Context,
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
        "stop" => Some(binding("message_sent")),
        "interrupt" => Some(observe("message_cancelled")),
        "pre_compact" => Some(binding("on_compaction_started")),
        _ => common_mapping(event),
    }
}

fn claude_mapping(event: &str) -> Option<HookMapping> {
    match event {
        "pre_tool_use" | "permission_request" => Some(binding("before_tool_call")),
        "user_prompt_submit" => Some(binding("message_received")),
        // Claude Stop is per-turn and carries `last_assistant_message`.
        "stop" => Some(binding("message_sent")),
        "setup" => Some(observe("session_setup")),
        "user_prompt_expansion" => Some(binding("message_expanded")),
        "permission_denied" => Some(observe("permission_denied")),
        "post_tool_use_failure" => Some(binding("after_tool_call_failed")),
        "post_tool_batch" => Some(binding("after_tool_batch")),
        "notification" => Some(observe("notification")),
        "task_created" => Some(binding("task_created")),
        "task_completed" => Some(binding("task_completed")),
        "teammate_idle" => Some(binding("teammate_idle")),
        "stop_failure" => Some(observe("message_failed")),
        "instructions_loaded" => Some(observe("instructions_loaded")),
        "config_change" => Some(binding("config_changed")),
        "cwd_changed" => Some(observe("cwd_changed")),
        "directory_added" => Some(observe("directory_added")),
        "file_changed" => Some(observe("file_changed")),
        "worktree_create" => Some(observe("worktree_create_requested")),
        "worktree_remove" => Some(observe("worktree_removed")),
        "pre_model_switch" => Some(binding("before_model_switch")),
        "post_model_switch" => Some(context("after_model_switch")),
        "elicitation" => Some(binding("elicitation_requested")),
        "elicitation_result" => Some(binding("elicitation_resolved")),
        "pre_compact" => Some(binding("on_compaction_started")),
        // Claude MessageDisplay carries response text in `delta` while it is
        // rendered. It is observation-only on this relay.
        "message_display" => Some(observe("message_displayed")),
        _ => common_mapping(event),
    }
}

fn grok_mapping(event: &str) -> Option<HookMapping> {
    match event {
        "pre_tool_use" => Some(binding("before_tool_call")),
        // Grok ignores stdout for passive events; only pre-tool and stopping
        // events support decisions. Do not collect unusable replies.
        "user_prompt_submit" => Some(observe("message_received")),
        "session_start" => Some(observe("session_start")),
        "subagent_start" => Some(observe("subagent_start")),
        // Grok has PermissionDenied observations, not PermissionRequest.
        "permission_request" => None,
        // Grok Stop completes a turn, not the session. Retiring the route here
        // loses authentication for subsequent events in that same session.
        "stop" => Some(binding("message_sent")),
        "post_tool_use" => Some(observe("after_tool_call")),
        "post_tool_use_failure" => Some(observe("after_tool_call_failed")),
        "permission_denied" => Some(observe("permission_denied")),
        "stop_failure" => Some(observe("message_failed")),
        "stop_cancelled" => Some(observe("message_cancelled")),
        "notification" => Some(observe("notification")),
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
    #[serde(skip_serializing_if = "Option::is_none")]
    collection: Option<PluginCollectionProvenance<'a>>,
}

#[derive(Debug, Serialize)]
struct PluginCollectionProvenance<'a> {
    schema: &'static str,
    host: &'a str,
    session_id: &'a str,
    route_id: &'a str,
    invocation_id: &'a str,
    source_event: &'a str,
    payload_sha256: String,
    semantics: &'static str,
}

// Matches Codewall's content canonical JSON: sorted object keys, compact UTF-8,
// integer numbers only. Unsupported input still reaches ordinary hook handling;
// it cannot claim collection provenance. No bearer token reaches this adapter.
fn collection_payload_digest(value: &serde_json::Value) -> Option<String> {
    fn sorted(value: &serde_json::Value) -> Option<serde_json::Value> {
        match value {
            serde_json::Value::Object(map) => {
                let fields: std::collections::BTreeMap<_, _> = map
                    .iter()
                    .map(|(key, value)| Some((key.clone(), sorted(value)?)))
                    .collect::<Option<_>>()?;
                Some(serde_json::Value::Object(fields.into_iter().collect()))
            }
            serde_json::Value::Array(values) => Some(serde_json::Value::Array(
                values.iter().map(sorted).collect::<Option<_>>()?,
            )),
            serde_json::Value::Number(number) if number.is_f64() => None,
            other => Some(other.clone()),
        }
    }
    let bytes = serde_json::to_vec(&sorted(value)?).ok()?;
    Some(
        Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect(),
    )
}

fn plugin_collection<'a>(
    event: &'a OracleHookEvent,
    payload: &serde_json::Value,
) -> Option<PluginCollectionProvenance<'a>> {
    if event.host != "claude"
        || !matches!(event.event.as_str(), "user_prompt_submit" | "pre_tool_use")
    {
        return None;
    }
    Some(PluginCollectionProvenance {
        schema: "oracle.content_source.v1",
        host: &event.host,
        session_id: &event.session_id,
        route_id: &event.route_id,
        invocation_id: &event.correlation_id,
        source_event: &event.event,
        payload_sha256: collection_payload_digest(payload)?,
        semantics: "observation",
    })
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
    let mut normalized = normalized_payload(&event.payload)?;
    if event.event == "elicitation_result" && normalized.get("content").is_some() {
        normalized["content"] = serde_json::json!({"redacted": true});
    }
    let payload = CanonicalOraclePayload {
        principal_id: &event.principal_id,
        host: &event.host,
        session_id: &event.session_id,
        source_event: &event.event,
        turn_id: event.turn_id.as_deref(),
        workspace_id: event.workspace_id.as_deref(),
        payload: &normalized,
        collection: plugin_collection(event, &normalized),
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

/// Keep native fields for compatibility, adding consistent names for consumers.
/// Conflicting aliases are ambiguous policy input, never a precedence rule.
fn normalized_payload(payload: &serde_json::Value) -> Result<serde_json::Value, SysError> {
    let mut normalized = payload.clone();
    let object = normalized
        .as_object_mut()
        .ok_or_else(|| SysError::HostError("host hook payload must be an object".to_owned()))?;
    for (canonical, native) in [
        ("tool_name", "toolName"),
        ("tool_input", "toolInput"),
        ("tool_use_id", "toolUseId"),
        ("tool_response", "toolResponse"),
        ("permission_mode", "permissionMode"),
        ("tool_input_truncated", "toolInputTruncated"),
        ("workspace_root", "workspaceRoot"),
    ] {
        if let Some(value) = object.get(native).cloned() {
            if object
                .get(canonical)
                .is_some_and(|existing| existing != &value)
            {
                return Err(SysError::HostError(format!(
                    "conflicting host hook fields: {canonical}/{native}"
                )));
            }
            object.insert(canonical.to_owned(), value);
        }
    }
    Ok(normalized)
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
    if mapping.response == ResponseMode::Context {
        return collect_additional_context(&subscription, &reply_topic, &event.principal_id).map(
            |context| {
                (
                    context,
                    matches!(event.event.as_str(), "stop" | "subagent_stop")
                        .then(Decision::default),
                )
            },
        );
    }
    collect_binding_response(
        &subscription,
        &reply_topic,
        &event.principal_id,
        mapping.hook,
    )
}

fn collect_binding_response(
    subscription: &ipc::Subscription,
    reply_topic: &str,
    principal: &str,
    hook: &str,
) -> Result<(Option<String>, Option<Decision>), SysError> {
    // Read the invocation's principal overlay, never a process-global cache.
    let config = match env::var("AOS_ORACLE_REQUIRED_HOOK_SOURCES") {
        Ok(config) => config,
        Err(_) => return Ok((None, Some(Decision::unavailable()))),
    };
    // Legacy registrations protect prompt/pre-tool only. Expanding the event
    // inventory must not require those responders to answer unrelated hooks.
    let scoped = env::var("AOS_ORACLE_REQUIRED_HOOK_POLICIES").unwrap_or_else(|_| "{}".into());
    let Ok(config) = decision::registrations_for_hook(hook, &config, &scoped) else {
        return Ok((None, Some(Decision::unavailable())));
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
    let mapping = lifecycle_mapping(expected, &event, mapping);
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

fn lifecycle_mapping(
    expected: Frontend,
    event: &OracleHookEvent,
    mut mapping: HookMapping,
) -> HookMapping {
    // Stop hooks can ask the model to continue, which invokes Stop again. The
    // client's explicit recursion marker makes that second event observational.
    // This does not affect pre-tool decisions or authorization.
    if matches!(event.event.as_str(), "stop" | "subagent_stop")
        && (event
            .payload
            .get("stop_hook_active")
            .and_then(serde_json::Value::as_bool)
            == Some(true)
            || event
                .payload
                .get("stopHookActive")
                .and_then(serde_json::Value::as_bool)
                == Some(true))
    {
        mapping.response = ResponseMode::Context;
    }
    // A child's teardown must not retire the parent's shared host route.
    if expected == Frontend::Grok
        && event.event == "session_end"
        && event
            .payload
            .get("subagentType")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|value| !value.is_empty())
    {
        mapping = observe("subagent_session_end");
    }
    mapping
}

/// Oracle hook protocol adapter.
#[derive(Default)]
pub struct OracleHookAdapter;

#[capsule]
impl OracleHookAdapter {
    /// Report the collection contract to an authenticated, correlation-scoped probe.
    #[astrid::interceptor("content_source_capability_v1")]
    pub fn content_source_capability_v1(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let caller = runtime::caller()?;
        let Some(principal) = caller.principal.as_deref() else {
            return Ok(());
        };
        let Some(id) = payload
            .get("request_id")
            .and_then(serde_json::Value::as_str)
        else {
            return Ok(());
        };
        if id.len() != 32
            || !id.bytes().all(|c| c.is_ascii_hexdigit())
            || payload.get("principal").and_then(serde_json::Value::as_str) != Some(principal)
        {
            return Ok(());
        }
        ipc::publish_json(
            &format!("oracle.v1.content.capability.reply.{id}"),
            &serde_json::json!({
                "schema":"oracle.content_source.v1", "request_id":id, "principal":principal,
                "host":"claude", "events":["user_prompt_submit", "pre_tool_use"],
                "semantics":"observation", "digest":"sha256_canonical_json_v1"
            }),
        )
    }

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

    #[test]
    fn every_native_codec_event_has_matching_canonical_response_semantics() {
        // Generated from Oracles' shipped codec, not a second handwritten map.
        let inventory: serde_json::Value =
            serde_json::from_str(include_str!("../testdata/native-hook-inventory.json")).unwrap();
        for host in [Frontend::Claude, Frontend::Codex, Frontend::Grok] {
            for (event, entry) in inventory[host.name()].as_object().unwrap() {
                let mode = entry[2].as_str().unwrap();
                let expected = match mode {
                    "observe" | "worktree" => ResponseMode::Observe,
                    "context" => ResponseMode::Context,
                    _ => ResponseMode::Binding,
                };
                let mapping = host.mapping(event).unwrap();
                assert_eq!(mapping.hook, entry[1].as_str().unwrap(), "{host:?}/{event}");
                assert_eq!(mapping.response, expected, "{host:?}/{event}");
            }
            assert_eq!(host.mapping("invented_event"), None);
        }
    }

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
    fn plugin_collection_identity_survives_retry() {
        let event = host_event("claude");
        let first = collection(&event);
        assert_eq!(first["schema"], "oracle.content_source.v1");
        assert_eq!(first["invocation_id"], event.correlation_id);
        assert_eq!(
            first["payload_sha256"],
            "8a44725210b9dcd4fefd9f0eca07b70ae45e69274a3105fb25eb426a2cf8bbf4"
        );
        assert_eq!(first, collection(&event));
    }

    fn collection(event: &OracleHookEvent) -> serde_json::Value {
        let request =
            canonical_request(event, Frontend::Claude.mapping(&event.event).unwrap()).unwrap();
        let payload: serde_json::Value = serde_json::from_str(&request.payload).unwrap();
        payload["collection"].clone()
    }

    #[test]
    fn plugin_collection_distinct_invocations() {
        let mut event = host_event("claude");
        let first = collection(&event);
        event.correlation_id = "c".repeat(32);
        event.delivery_id = format!("{}-{}", event.route_id, event.correlation_id);
        let second = collection(&event);
        assert_ne!(first["invocation_id"], second["invocation_id"]);
        assert_eq!(first["payload_sha256"], second["payload_sha256"]);
    }

    #[test]
    fn plugin_collection_spoofed_metadata() {
        let mut event = host_event("claude");
        let original = collection(&event);
        event.payload["collection"] = serde_json::json!({
            "host": "codex", "source_event": "permission_request",
            "invocation_id": "forged", "payload_sha256": "forged"
        });
        let actual = collection(&event);
        assert_eq!(actual["host"], "claude");
        assert_eq!(actual["source_event"], "user_prompt_submit");
        assert_eq!(actual["invocation_id"], event.correlation_id);
        assert_ne!(actual["payload_sha256"], original["payload_sha256"]);
    }

    #[test]
    fn plugin_collection_tool_event() {
        let mut event = host_event("claude");
        event.event = "pre_tool_use".to_owned();
        event.payload =
            serde_json::json!({"tool_name": "Bash", "tool_input": {"command": "echo hello"}});
        let request = canonical_request(&event, observe("before_tool_call")).unwrap();
        assert!(request.correlation_id.is_none());
        let actual = collection(&event);
        assert_eq!(actual["invocation_id"], event.correlation_id);
        assert_eq!(actual["semantics"], "observation");
        event.event = "permission_request".to_owned();
        assert!(collection(&event).is_null());
        event.event = "user_prompt_submit".to_owned();
        event.host = "codex".to_owned();
        assert!(collection(&event).is_null());
    }

    #[test]
    fn plugin_collection_unsupported_numbers_preserve_host_event() {
        let mut event = host_event("claude");
        event.payload = serde_json::json!({"prompt": "hello", "fraction": 1.5});
        let request = canonical_request(&event, context("message_received")).unwrap();
        let payload: serde_json::Value = serde_json::from_str(&request.payload).unwrap();
        assert_eq!(payload["payload"], event.payload);
        assert!(payload.get("collection").is_none());
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
    fn every_host_collects_pretool_and_supported_prompt_verdicts() {
        for host in [Frontend::Codex, Frontend::Claude, Frontend::Grok] {
            assert_eq!(
                host.mapping("pre_tool_use"),
                Some(binding("before_tool_call"))
            );
            assert_eq!(
                host.mapping("user_prompt_submit"),
                Some(if host == Frontend::Grok {
                    observe("message_received")
                } else {
                    binding("message_received")
                })
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
    fn one_pretool_consumer_receives_identical_fields_from_every_frontend() {
        let fields = serde_json::json!({
            "tool_name": "Bash", "tool_input": {"command": "printf hello"},
            "tool_use_id": "call-one"
        });
        for host in [Frontend::Codex, Frontend::Claude, Frontend::Grok] {
            let mut event = host_event(host.name());
            event.event = "pre_tool_use".to_owned();
            event.payload = if host == Frontend::Grok {
                serde_json::json!({
                    "toolName": "Bash", "toolInput": {"command": "printf hello"},
                    "toolUseId": "call-one"
                })
            } else {
                fields.clone()
            };
            let request = canonical_request(&event, host.mapping(&event.event).unwrap()).unwrap();
            assert_eq!(request.hook, "before_tool_call");
            assert_eq!(
                request.correlation_id.as_deref(),
                Some(event.correlation_id.as_str())
            );
            let value: serde_json::Value = serde_json::from_str(&request.payload).unwrap();
            for key in ["tool_name", "tool_input", "tool_use_id"] {
                assert_eq!(value["payload"][key], fields[key]);
            }
            assert_eq!(value["principal_id"], event.principal_id);
            assert_eq!(value["session_id"], event.session_id);
        }
    }

    #[test]
    fn normalization_preserves_extensions_and_rejects_conflicting_aliases() {
        let payload = serde_json::json!({
            "toolName": "Bash", "tool_name": "Bash", "extension": {"custom": true}
        });
        assert_eq!(normalized_payload(&payload).unwrap(), payload);
        assert!(
            normalized_payload(&serde_json::json!({
                "toolName": "Bash", "tool_name": "Read"
            }))
            .is_err()
        );
        assert!(normalized_payload(&serde_json::json!(["Bash"])).is_err());
    }

    #[test]
    fn codex_and_claude_stop_are_response_events_not_session_termination() {
        assert_eq!(
            Frontend::Codex.mapping("stop"),
            Some(binding("message_sent"))
        );
        assert_eq!(
            Frontend::Claude.mapping("stop"),
            Some(binding("message_sent"))
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
            Some(binding("message_sent"))
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

    #[test]
    fn repeated_stop_is_context_only_and_child_teardown_keeps_parent_route() {
        for host in [Frontend::Codex, Frontend::Claude, Frontend::Grok] {
            let mut event = host_event(host.name());
            event.event = "stop".into();
            event.payload = serde_json::json!({"stop_hook_active": true});
            assert_eq!(
                lifecycle_mapping(host, &event, host.mapping("stop").unwrap()),
                context("message_sent")
            );
        }
        let mut child = host_event("grok");
        child.event = "session_end".into();
        child.payload = serde_json::json!({"subagentType": "explore"});
        assert_eq!(
            lifecycle_mapping(Frontend::Grok, &child, grok_mapping("session_end").unwrap()),
            observe("subagent_session_end")
        );
    }

    #[test]
    fn elicitation_values_are_not_published_to_generic_subscribers() {
        let mut event = host_event("claude");
        event.event = "elicitation_result".into();
        event.payload = serde_json::json!({"mcp_server_name": "test", "action": "accept", "content": {"password": "not-on-bus"}});
        let request = canonical_request(&event, claude_mapping(&event.event).unwrap()).unwrap();
        assert!(!request.payload.contains("not-on-bus"));
        let value: serde_json::Value = serde_json::from_str(&request.payload).unwrap();
        assert_eq!(
            value["payload"]["content"],
            serde_json::json!({"redacted": true})
        );
        assert_eq!(value["payload"]["action"], "accept");
    }
}
