#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]

//! Prompt Builder capsule — assembles LLM prompts with plugin hook interception.
//!
//! This capsule owns the prompt assembly pipeline. When the react loop needs
//! a prompt assembled, it publishes to `prompt_builder.v1.assemble`. The prompt
//! builder then:
//!
//! 1. Fires `prompt_builder.v1.hook.before_build` to all plugin capsules via IPC
//! 2. Collects plugin responses (`prependSystemContext`, `appendSystemContext`,
//!    `systemPrompt` override, `prependContext`)
//! 3. Merges them according to OpenClaw-compatible semantics
//! 4. Returns the assembled prompt on `prompt_builder.v1.response.assemble`
//! 5. Fires `prompt_builder.v1.hook.after_build` as a notification
//!
//! # Merge Semantics
//!
//! 1. `prependContext` — concatenated in order, becomes `user_context_prefix`
//! 2. `systemPrompt` — last non-null value wins (full override)
//! 3. `prependSystemContext` — concatenated in order, prepended to system prompt
//! 4. `appendSystemContext` — concatenated in order, appended to system prompt

use astrid_sdk::prelude::*;
use serde::{Deserialize, Serialize};

/// Runtime configuration loaded from capsule config at startup.
struct Config {
    /// Maximum time (in milliseconds) to wait for plugin hook responses.
    hook_timeout_ms: u64,
}

impl Config {
    /// Load configuration from the capsule's config store, falling back to defaults.
    fn load() -> Self {
        let hook_timeout_ms = env::var("hook_timeout_ms")
            .ok()
            .and_then(|s| s.trim().trim_matches('"').parse::<u64>().ok())
            .unwrap_or(DEFAULT_HOOK_POLL_TIMEOUT_MS);

        Self { hook_timeout_ms }
    }
}

/// Request from the react loop to assemble a prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct AssembleRequest {
    /// The conversation messages.
    #[serde(default)]
    messages: serde_json::Value,
    /// The current system prompt before plugin modifications.
    #[serde(default)]
    system_prompt: String,
    /// Unique request identifier for correlation.
    request_id: String,
    /// The target LLM model identifier.
    #[serde(default)]
    model: String,
    /// The LLM provider identifier.
    #[serde(default)]
    provider: String,
    /// Session ID echoed back for react loop correlation.
    #[serde(default)]
    session_id: Option<String>,
}

/// Response returned to the react loop with the assembled prompt.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct AssembleResponse {
    /// The final assembled system prompt.
    system_prompt: String,
    /// Text to prepend to the user's message (from `prependContext` hooks).
    user_context_prefix: String,
    /// The original request ID for correlation.
    request_id: String,
    /// Session ID echoed from the request for react loop correlation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    session_id: Option<String>,
    /// Collected tool schemas from all tool-providing capsules.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tools: Vec<serde_json::Value>,
    /// Session conversation history messages.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    messages: Vec<serde_json::Value>,
}

/// Payload sent to plugin capsules via the `prompt_builder.v1.hook.before_build` interceptor.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct BeforePromptBuildPayload {
    messages: serde_json::Value,
    system_prompt: String,
    request_id: String,
    model: String,
    provider: String,
    /// Topic where plugins should publish their hook responses.
    response_topic: String,
}

/// A single plugin's response to the `prompt_builder.v1.hook.before_build` hook.
///
/// All fields are optional. The prompt builder merges responses from
/// multiple plugins according to the documented merge semantics.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct HookResponse {
    /// Text to prepend to the system prompt.
    #[serde(default)]
    prepend_system_context: Option<String>,
    /// Text to append to the system prompt.
    #[serde(default)]
    append_system_context: Option<String>,
    /// Full system prompt override (last non-null wins).
    #[serde(default)]
    system_prompt: Option<String>,
    /// Text to prepend to the user's message.
    #[serde(default)]
    prepend_context: Option<String>,
}

impl HookResponse {
    /// Returns `true` if at least one field is set.
    fn has_any_field(&self) -> bool {
        self.prepend_system_context.is_some()
            || self.append_system_context.is_some()
            || self.system_prompt.is_some()
            || self.prepend_context.is_some()
    }
}

/// Notification payload sent after prompt assembly completes.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
struct AfterPromptBuildPayload {
    system_prompt: String,
    user_context_prefix: String,
    request_id: String,
}

/// Default maximum time (in milliseconds) to wait for plugin hook responses.
/// Overridable via the `hook_timeout_ms` capsule config key.
const DEFAULT_HOOK_POLL_TIMEOUT_MS: u64 = 2000;

/// Maximum number of hook responses to collect before proceeding.
const MAX_HOOK_RESPONSES: usize = 50;

/// A hook response paired with its source capsule identifier.
///
/// Used for permission gating: plugins with `allowPromptInjection: false`
/// should have their prompt-mutating fields discarded.
struct SourcedHookResponse {
    /// The capsule/session ID that sent this response.
    source_id: Option<String>,
    /// The parsed hook response.
    response: HookResponse,
}

/// Filter hook responses based on prompt injection permissions.
///
/// Plugins without prompt injection permission retain only `prependContext`
/// (user-visible context), while `systemPrompt`, `prependSystemContext`,
/// and `appendSystemContext` are stripped.
///
/// The `has_permission` closure receives the `source_id` (if present) and
/// returns whether the source capsule is allowed to mutate the system
/// prompt. This closure is parameterized for testability - the production
/// call site queries the kernel via `capabilities::check`.
fn filter_by_permission(
    sourced: Vec<SourcedHookResponse>,
    mut has_permission: impl FnMut(Option<&str>) -> bool,
) -> Vec<HookResponse> {
    sourced
        .into_iter()
        .map(|s| {
            if has_permission(s.source_id.as_deref()) {
                s.response
            } else {
                // Strip prompt-mutating fields; only user-visible context passes.
                if s.response.system_prompt.is_some()
                    || s.response.prepend_system_context.is_some()
                    || s.response.append_system_context.is_some()
                {
                    log::warn(format!(
                        "Stripped prompt-mutating fields from capsule {:?} \
                         (missing allow_prompt_injection capability)",
                        s.source_id
                    ));
                }
                HookResponse {
                    prepend_context: s.response.prepend_context,
                    ..Default::default()
                }
            }
        })
        .collect()
}

/// Merge collected hook responses into a final assembled prompt.
///
/// Merge order (matches OpenClaw documented behaviour):
/// 1. `prependContext` — concatenated in interceptor order
/// 2. `systemPrompt` — last non-null value wins as full override
/// 3. `prependSystemContext` — concatenated, prepended to (possibly overridden) prompt
/// 4. `appendSystemContext` — concatenated, appended to system prompt
fn merge_hook_responses(original_system_prompt: &str, responses: &[HookResponse]) -> MergedPrompt {
    let mut prepend_contexts: Vec<&str> = Vec::new();
    let mut prepend_system_contexts: Vec<&str> = Vec::new();
    let mut append_system_contexts: Vec<&str> = Vec::new();
    let mut system_prompt_override: Option<&str> = None;

    for resp in responses {
        if let Some(ref ctx) = resp.prepend_context
            && !ctx.is_empty()
        {
            prepend_contexts.push(ctx);
        }
        if let Some(ref prompt) = resp.system_prompt
            && !prompt.is_empty()
        {
            // Last non-empty wins — intentionally overwrites previous overrides.
            // An empty string is treated as "no override" to prevent accidentally
            // wiping the system prompt.
            system_prompt_override = Some(prompt);
        }
        if let Some(ref ctx) = resp.prepend_system_context
            && !ctx.is_empty()
        {
            prepend_system_contexts.push(ctx);
        }
        if let Some(ref ctx) = resp.append_system_context
            && !ctx.is_empty()
        {
            append_system_contexts.push(ctx);
        }
    }

    // Step 2: Determine the base system prompt (override or original).
    let base_prompt = system_prompt_override.unwrap_or(original_system_prompt);

    // Step 3-4: Prepend + base + append, joined with newlines.
    let mut parts: Vec<&str> = Vec::new();
    parts.extend_from_slice(&prepend_system_contexts);
    if !base_prompt.is_empty() {
        parts.push(base_prompt);
    }
    parts.extend_from_slice(&append_system_contexts);
    let final_prompt = parts.join("\n");

    // Step 1: Build user context prefix.
    let user_context_prefix = prepend_contexts.join("\n");

    MergedPrompt {
        system_prompt: final_prompt,
        user_context_prefix,
    }
}

/// The result of merging all hook responses.
struct MergedPrompt {
    system_prompt: String,
    user_context_prefix: String,
}

/// Fire the `prompt_builder.v1.hook.before_build` interceptor and collect plugin responses.
///
/// Publishes the hook event on the `prompt_builder.v1.hook.before_build` IPC topic and polls
/// a dedicated response topic for plugin contributions. Returns all collected
/// responses within the timeout window, filtered by permission gating.
fn fire_before_prompt_build(request: &AssembleRequest, config: &Config) -> Vec<HookResponse> {
    let response_topic = format!("prompt_builder.v1.hook_response.{}", request.request_id);

    // Subscribe BEFORE publishing to avoid missing fast responses.
    let sub = match ipc::subscribe(&response_topic) {
        Ok(h) => h,
        Err(e) => {
            log::error(format!("Failed to subscribe to hook response topic: {e}"));
            return Vec::new();
        }
    };

    let payload = BeforePromptBuildPayload {
        messages: request.messages.clone(),
        system_prompt: request.system_prompt.clone(),
        request_id: request.request_id.clone(),
        model: request.model.clone(),
        provider: request.provider.clone(),
        response_topic: response_topic.clone(),
    };

    if let Err(e) = ipc::publish_json("prompt_builder.v1.hook.before_build", &payload) {
        log::error(format!(
            "Failed to publish prompt_builder.v1.hook.before_build event: {e}"
        ));
        // sub dropped on return — kernel-side resource released automatically.
        return Vec::new();
    }

    // Block-wait for hook responses within the configured timeout.
    // `std::time::Instant::now()` panics on `wasm32-unknown-unknown`
    // (the Astrid-canonical capsule target); track the deadline as a
    // host-monotonic instant via `astrid_sdk::time::monotonic`, which
    // routes through `astrid:sys.clock-monotonic-ns`.
    let mut sourced_responses = Vec::new();
    let start = astrid_sdk::time::monotonic();
    let timeout_dur = std::time::Duration::from_millis(config.hook_timeout_ms);

    while astrid_sdk::time::monotonic().saturating_sub(start) < timeout_dur
        && sourced_responses.len() < MAX_HOOK_RESPONSES
    {
        let elapsed = astrid_sdk::time::monotonic().saturating_sub(start);
        let remaining_ms = timeout_dur.saturating_sub(elapsed).as_millis();
        if remaining_ms == 0 {
            break;
        }
        let timeout = u64::try_from(remaining_ms).unwrap_or(u64::MAX);

        match sub.recv(timeout) {
            Ok(result) => {
                if result.messages.is_empty() {
                    break;
                }
                for msg in &result.messages {
                    if let Some(new_responses) = parse_hook_message(msg) {
                        sourced_responses.extend(new_responses);
                    }
                }
            }
            _ => break,
        }
    }

    // sub drops here, releasing the kernel-side subscription.

    log::info(format!(
        "Collected {} hook responses for request {}",
        sourced_responses.len(),
        request.request_id
    ));

    // Cache capability results per-UUID to avoid redundant host function calls.
    // Multiple hook responses can come from the same capsule.
    let mut cache = std::collections::HashMap::<String, bool>::new();
    filter_by_permission(sourced_responses, |source_id| {
        let Some(uuid) = source_id else {
            return false;
        };
        *cache.entry(uuid.to_owned()).or_insert_with(|| {
            capabilities::check(uuid, "allow_prompt_injection")
                .inspect_err(|e| {
                    log::warn(format!("capability check failed for {uuid}: {e}, denying"));
                })
                .unwrap_or(false)
        })
    })
}

/// Parse a single IPC message and extract hook responses with source capsule IDs.
fn parse_hook_message(msg: &ipc::Message) -> Option<Vec<SourcedHookResponse>> {
    let payload: serde_json::Value = match serde_json::from_str(&msg.payload) {
        Ok(v) => v,
        Err(e) => {
            log::warn(format!("failed to deserialize hook response payload: {e}"));
            return None;
        }
    };

    let source_id = if msg.source_id.is_empty() {
        None
    } else {
        Some(msg.source_id.clone())
    };

    // Try to parse the payload directly as a HookResponse.
    // Since all fields are optional, an unrelated JSON object would
    // parse as an empty HookResponse — check `has_any_field()` to
    // distinguish real responses from false positives.
    // Plugins may wrap it in various IPC payload envelopes, so we
    // also check inside `data` for Custom payloads.
    let maybe_response = serde_json::from_value::<HookResponse>(payload.clone())
        .ok()
        .filter(HookResponse::has_any_field)
        .or_else(|| {
            payload
                .get("data")
                .and_then(|data| serde_json::from_value::<HookResponse>(data.clone()).ok())
                .filter(HookResponse::has_any_field)
        });

    if let Some(response) = maybe_response {
        Some(vec![SourcedHookResponse {
            source_id,
            response,
        }])
    } else {
        None
    }
}

/// Parse the poll envelope and extract hook responses with source capsule IDs.
///
/// Retained for unit tests that construct raw JSON envelopes.
#[cfg(test)]
fn parse_hook_responses(poll_bytes: &[u8]) -> Option<Vec<SourcedHookResponse>> {
    let envelope: serde_json::Value = match serde_json::from_slice(poll_bytes) {
        Ok(v) => v,
        Err(e) => {
            log::warn(format!("failed to deserialize hook response envelope: {e}"));
            return None;
        }
    };

    let messages = envelope.get("messages")?.as_array()?;
    let mut responses = Vec::new();

    for msg in messages {
        let payload = match msg.get("payload") {
            Some(p) => p,
            None => continue,
        };

        // Track the source capsule for permission gating.
        let source_id = msg
            .get("source_id")
            .and_then(|s| s.as_str())
            .map(String::from);

        let maybe_response = serde_json::from_value::<HookResponse>(payload.clone())
            .ok()
            .filter(HookResponse::has_any_field)
            .or_else(|| {
                payload
                    .get("data")
                    .and_then(|data| serde_json::from_value::<HookResponse>(data.clone()).ok())
                    .filter(HookResponse::has_any_field)
            });

        if let Some(response) = maybe_response {
            responses.push(SourcedHookResponse {
                source_id,
                response,
            });
        }
    }

    if responses.is_empty() {
        None
    } else {
        Some(responses)
    }
}

/// Fire the `prompt_builder.v1.hook.after_build` notification (fire-and-forget).
fn fire_after_prompt_build(system_prompt: &str, user_context_prefix: &str, request_id: &str) {
    let payload = AfterPromptBuildPayload {
        system_prompt: system_prompt.to_string(),
        user_context_prefix: user_context_prefix.to_string(),
        request_id: request_id.to_string(),
    };
    let _ = ipc::publish_json("prompt_builder.v1.hook.after_build", &payload);
}

/// KV key for cached tool schemas. First call populates it via IPC broadcast;
/// subsequent calls read directly from KV until invalidated.
const TOOL_SCHEMA_CACHE_KEY: &str = "__tool_schema_cache";

/// Timeout (ms) for fetching session messages from the session capsule.
const SESSION_FETCH_TIMEOUT_MS: u64 = 5000;

/// Timeout (ms) for fanning out the tool-describe request and collecting
/// responses from every tool-providing capsule.
const TOOL_DESCRIBE_FANOUT_TIMEOUT_MS: u64 = 2000;

/// Maximum number of tool-describe responses to collect before proceeding.
const MAX_TOOL_DESCRIBE_RESPONSES: usize = 256;

/// Collect tool schemas from all capsules via IPC fan-out.
///
/// Checks `__tool_schema_cache` in KV first. On cache miss, subscribes to
/// `tool.v1.response.describe.*` and publishes a `tool.v1.request.describe`
/// event. Tool-providing capsules respond on their own
/// `tool.v1.response.describe.<source_id>` topic. Responses are collected
/// within `TOOL_DESCRIBE_FANOUT_TIMEOUT_MS` and deduplicated by tool name.
///
/// The pre-#752 implementation used `hooks::trigger`, which has been removed
/// from the host ABI surface. This IPC-based fan-out replaces it; the same
/// `{ "tools": [...] }` envelope (from SDK macro `tool_describe` and
/// `astrid_bridge.mjs`) is honoured.
fn collect_tool_schemas() -> Vec<serde_json::Value> {
    // Check KV cache first.
    if let Ok(cached) = kv::get_json::<Vec<serde_json::Value>>(TOOL_SCHEMA_CACHE_KEY)
        && !cached.is_empty()
    {
        log::debug(format!("Returning {} cached tool schemas", cached.len()));
        return cached;
    }

    // Subscribe BEFORE publishing so we don't miss fast responders.
    let sub = match ipc::subscribe("tool.v1.response.describe.*") {
        Ok(s) => s,
        Err(e) => {
            log::error(format!(
                "Failed to subscribe to tool.v1.response.describe.*: {e}"
            ));
            return Vec::new();
        }
    };

    // Fire the fan-out request. Empty payload — every responder publishes its
    // own tool schema set onto `tool.v1.response.describe.<source_id>`.
    if let Err(e) = ipc::publish("tool.v1.request.describe", "{}") {
        log::error(format!("Failed to publish tool.v1.request.describe: {e}"));
        return Vec::new();
    }

    // Collect responses until we time out or hit the cap. Monotonic
    // clock via `astrid_sdk::time` — `std::time::Instant::now()`
    // panics on `wasm32-unknown-unknown`.
    let mut all_tools: Vec<serde_json::Value> = Vec::new();
    let start = astrid_sdk::time::monotonic();
    let timeout_dur = std::time::Duration::from_millis(TOOL_DESCRIBE_FANOUT_TIMEOUT_MS);

    while astrid_sdk::time::monotonic().saturating_sub(start) < timeout_dur
        && all_tools.len() < MAX_TOOL_DESCRIBE_RESPONSES
    {
        let elapsed = astrid_sdk::time::monotonic().saturating_sub(start);
        let remaining_ms = timeout_dur.saturating_sub(elapsed).as_millis();
        if remaining_ms == 0 {
            break;
        }
        let timeout = u64::try_from(remaining_ms).unwrap_or(u64::MAX);

        match sub.recv(timeout) {
            Ok(result) => {
                if result.messages.is_empty() {
                    break;
                }
                for msg in &result.messages {
                    if let Some(tools) = extract_tools_from_response(&msg.payload) {
                        all_tools.extend(tools);
                    }
                }
            }
            _ => break,
        }
    }

    // Deduplicate by tool name (first occurrence wins).
    let mut seen = std::collections::HashSet::new();
    all_tools.retain(|tool| {
        if let Some(name) = tool.get("name").and_then(|n| n.as_str()) {
            seen.insert(name.to_string())
        } else {
            true
        }
    });

    log::info(format!(
        "Collected {} tool schemas via tool.v1.request.describe fan-out",
        all_tools.len()
    ));

    // Cache the result for subsequent calls.
    if let Err(e) = kv::set_json(TOOL_SCHEMA_CACHE_KEY, &all_tools) {
        log::warn(format!("Failed to cache tool schemas in KV: {e}"));
    }

    all_tools
}

/// Extract the `tools` array from a `tool.v1.response.describe.*` payload.
///
/// Honours both the direct envelope (`{ "tools": [...] }`, emitted by the
/// SDK macro `tool_describe` and `astrid_bridge.mjs`) and the wrapped
/// `{ "data": { "tools": [...] } }` envelope used by some Custom payload
/// publishers.
fn extract_tools_from_response(payload: &str) -> Option<Vec<serde_json::Value>> {
    let value: serde_json::Value = serde_json::from_str(payload).ok()?;
    let tools = value
        .get("tools")
        .or_else(|| value.get("data").and_then(|d| d.get("tools")))
        .and_then(|t| t.as_array())?;
    Some(tools.clone())
}

/// Fetch session conversation history from the session capsule via IPC.
///
/// Uses [`ipc::request_response`], which generates a v4 correlation ID,
/// subscribes to the scoped reply topic *before* publishing, injects the
/// correlation ID into the request body, and tears the subscription down
/// on every return path via the [`ipc::Subscription`] Drop. Returns an
/// empty vec on timeout, parse failure, or transport error.
fn fetch_session_messages(session_id: &str) -> Vec<serde_json::Value> {
    let request = serde_json::json!({ "session_id": session_id });

    // Responder publishes onto `session.v1.response.get_messages.<corr_id>`.
    let raw: serde_json::Value = match ipc::request_response(
        "session.v1.request.get_messages",
        "session.v1.response.get_messages",
        &request,
        SESSION_FETCH_TIMEOUT_MS,
    ) {
        Ok(v) => v,
        Err(e) => {
            log::warn(format!(
                "session.v1.request.get_messages failed (timeout={SESSION_FETCH_TIMEOUT_MS}ms): {e}"
            ));
            return Vec::new();
        }
    };

    // The session capsule may wrap the array in a Custom `data` envelope.
    let messages_value = raw
        .get("messages")
        .or_else(|| raw.get("data").and_then(|d| d.get("messages")));

    let result = match messages_value {
        Some(messages) => serde_json::from_value::<Vec<serde_json::Value>>(messages.clone())
            .unwrap_or_else(|e| {
                log::warn(format!("Failed to parse session messages array: {e}"));
                Vec::new()
            }),
        None => {
            log::warn("Session response missing `messages` field");
            Vec::new()
        }
    };

    log::debug(format!(
        "Fetched {} session messages for session {session_id}",
        result.len()
    ));

    result
}

/// Invalidate the cached tool schemas in KV.
///
/// Called when capsules are loaded or unloaded to ensure the next
/// `collect_tool_schemas()` call fetches fresh data from all capsules.
fn invalidate_tool_cache() {
    let _ = kv::delete(TOOL_SCHEMA_CACHE_KEY);
    log::info("Tool schema cache invalidated");
}

/// Handle a single `prompt_builder.v1.assemble` request.
fn handle_assemble(payload: &serde_json::Value, config: &Config) {
    // Extract from Custom payload envelope or direct.
    let request_value = payload.get("data").unwrap_or(payload);

    let request: AssembleRequest = match serde_json::from_value(request_value.clone()) {
        Ok(r) => r,
        Err(e) => {
            log::error(format!("Failed to parse assemble request: {e}"));
            let _ = ipc::publish_json(
                "prompt_builder.v1.response.assemble",
                &serde_json::json!({"error": format!("invalid request: {e}")}),
            );
            return;
        }
    };

    if request.request_id.is_empty() {
        let _ = ipc::publish_json(
            "prompt_builder.v1.response.assemble",
            &serde_json::json!({"error": "missing request_id"}),
        );
        return;
    }

    // Fire interceptor hooks and collect responses.
    let hook_responses = fire_before_prompt_build(&request, config);

    // Merge all responses into the final prompt.
    let merged = merge_hook_responses(&request.system_prompt, &hook_responses);

    // Collect tool schemas (cached after first call).
    let tools = collect_tool_schemas();

    // Fetch session messages if a session_id was provided.
    let messages = request
        .session_id
        .as_deref()
        .map(fetch_session_messages)
        .unwrap_or_default();

    // Publish the assembled result.
    let response = AssembleResponse {
        system_prompt: merged.system_prompt.clone(),
        user_context_prefix: merged.user_context_prefix.clone(),
        request_id: request.request_id.clone(),
        session_id: request.session_id.clone(),
        tools,
        messages,
    };

    let _ = ipc::publish_json("prompt_builder.v1.response.assemble", &response);

    // Fire after_prompt_build notification (fire-and-forget).
    fire_after_prompt_build(
        &merged.system_prompt,
        &merged.user_context_prefix,
        &request.request_id,
    );
}

/// Returns `true` if the topic should be dispatched (not a self-echo).
///
/// Filters out our own response topics, hook response topics, and the
/// interceptor topics we publish. Only `prompt_builder.v1.assemble` passes.
fn should_dispatch_topic(topic: &str) -> bool {
    !topic.starts_with("prompt_builder.v1.response.")
        && !topic.starts_with("prompt_builder.v1.hook_response.")
        && topic != "prompt_builder.v1.hook.before_build"
        && topic != "prompt_builder.v1.hook.after_build"
}

/// Dispatch IPC messages from a PollResult to appropriate handlers.
fn handle_poll_result(result: &ipc::PollResult, config: &Config) {
    if result.dropped > 0 {
        log::warn(format!(
            "Event bus dropped {} messages in prompt builder poll",
            result.dropped
        ));
    }

    for msg in &result.messages {
        if !should_dispatch_topic(&msg.topic) {
            continue;
        }

        if msg.topic == "prompt_builder.v1.assemble" {
            let payload: serde_json::Value = match serde_json::from_str(&msg.payload) {
                Ok(v) => v,
                Err(_) => continue,
            };
            handle_assemble(&payload, config);
        } else if msg.topic == "prompt_builder.v1.invalidate_tool_cache" {
            invalidate_tool_cache();
        }
    }
}

#[derive(Default)]
struct PromptBuilder;

#[capsule]
impl PromptBuilder {
    #[astrid::run]
    fn run(&self) -> Result<(), SysError> {
        log::info("Prompt Builder capsule starting");

        let config = Config::load();
        log::info(format!("Hook timeout: {}ms", config.hook_timeout_ms));

        let sub =
            ipc::subscribe("prompt_builder.v1.*").map_err(|e| SysError::ApiError(e.to_string()))?;

        // Also subscribe to our own hook topics so we can filter them out.
        let hook_sub = ipc::subscribe("prompt_builder.v1.hook.before_build")
            .map_err(|e| SysError::ApiError(e.to_string()))?;
        let after_sub = ipc::subscribe("prompt_builder.v1.hook.after_build")
            .map_err(|e| SysError::ApiError(e.to_string()))?;

        // Invalidate the tool-schema cache whenever the capsule set changes.
        // The kernel broadcasts `astrid.v1.capsules_loaded` after (un)loading
        // capsules. Nothing publishes `prompt_builder.v1.invalidate_tool_cache`,
        // so without this the cache is stale forever — it lives in KV and so
        // survives even a daemon restart, meaning newly installed tool capsules
        // never reach the LLM until the cache is cleared by hand.
        let loaded_sub = ipc::subscribe("astrid.v1.capsules_loaded")
            .map_err(|e| SysError::ApiError(e.to_string()))?;

        // Signal readiness so the kernel can proceed with loading dependent capsules.
        // Best-effort: failure means the host mutex is poisoned (unrecoverable).
        let _ = runtime::signal_ready();

        log::info("Prompt Builder capsule ready");

        loop {
            // Block until a message arrives (up to 60s), eliminating busy-spin polling.
            match sub.recv(60_000) {
                Ok(result) => {
                    // Honour a pending capsule (un)load before serving this
                    // request, so the assemble we are about to handle collects a
                    // fresh tool set rather than a stale cached one.
                    if let Ok(loaded) = loaded_sub.poll()
                        && !loaded.messages.is_empty()
                    {
                        invalidate_tool_cache();
                    }
                    if !result.messages.is_empty() {
                        handle_poll_result(&result, &config);
                    }
                }
                Err(_) => break,
            }

            // Drain hook/after topics to prevent backpressure.
            let _ = hook_sub.poll();
            let _ = after_sub.poll();
        }

        // sub / hook_sub / after_sub drop here — RAII releases the kernel
        // subscriptions; no explicit unsubscribe needed.
        Ok(())
    }
}

#[cfg(test)]
mod tests;
