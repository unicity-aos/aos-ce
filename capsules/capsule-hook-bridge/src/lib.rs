#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![warn(missing_docs)]

//! Hook Bridge capsule — maps lifecycle events to semantic hooks.
//!
//! The kernel dispatches lifecycle events (e.g. `tool_call_started`,
//! `session_created`) to this capsule via interceptors. The Hook Bridge
//! maps each event to a semantic hook name, fans out to subscriber
//! capsules over IPC, and applies merge strategies to the collected
//! responses.
//!
//! # Architecture (post per-domain WIT split)
//!
//! ```text
//! Kernel EventBus → EventDispatcher → Hook Bridge (this capsule)
//!                                        ↓ ipc::publish("hook.v1.event.<hook>", req)
//!                                     Subscriber capsules A, B, C...
//!                                        ↓ publish "hook.v1.response.<hook>.<corr_id>"
//!                                     Hook Bridge collects responses, applies merge
//!                                        ↓ returns merged result via interceptor reply
//! ```
//!
//! Before the per-domain WIT split, fan-out was driven by the
//! `sys::trigger-hook` host fn (removed in `sdk-rust` 0.7). The kernel
//! iterated `CapsuleRegistry` itself and returned a concatenated list
//! of responses. Post-split, fan-out is a capsule-to-capsule IPC
//! convention: the Hook Bridge publishes a request, listens on a
//! correlation-keyed reply topic, and applies the merge.
//!
//! This is a **policy** capsule: it defines which lifecycle events map
//! to which hook names and how responses are merged.

use astrid_sdk::contracts::hook::HookEventRequest;
use astrid_sdk::prelude::*;
use serde::{Deserialize, Serialize};

/// Hard deadline for collecting hook responses, per dispatch.
///
/// The bus capped `request_response` at 60 s; we use a much shorter
/// window because interceptors block the lifecycle event chain and any
/// hook handler that takes >1 s is misbehaving. If no responses arrive
/// in this window, we return the merge of whatever did arrive (which
/// may be the `MergeSemantics::None` result).
const HOOK_COLLECT_DEADLINE_MS: u64 = 5_000;

/// Once one responder has answered, allow a short window for peers in the
/// same fan-out before returning. This avoids paying the full deadline after
/// the common single-responder case.
const HOOK_QUIESCENCE_MS: u64 = 25;

/// Host prompt hooks are latency-sensitive and all expected responders are
/// already-loaded local capsules.
const HOST_HOOK_COLLECT_DEADLINE_MS: u64 = 1_000;

const MAX_HOST_PAYLOAD_BYTES: usize = 1024 * 1024;
const MAX_HOST_CONTEXT_BYTES: usize = 64 * 1024;

/// Merged result from hook fan-out.
///
/// Uses `serde_json::Value` for `data` (not `String`) to preserve the
/// wire format: consumers expect `data` as a nested JSON object, not a
/// JSON-encoded string. The WIT contract describes this as
/// `option<string>` for schema purposes, but the Rust type must match
/// what goes on the wire.
#[derive(Serialize)]
pub struct HookResult {
    #[serde(skip_serializing_if = "Option::is_none")]
    skip: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    data: Option<serde_json::Value>,
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
    event: &'a str,
    correlation_id: &'a str,
    route_id: &'a str,
    delivery_id: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    context: Option<String>,
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

// ── Merge Semantics ────────────────────────────────────────────────────────────────

/// How interceptor responses are merged for a hook.
#[derive(Debug, Clone, PartialEq, Eq)]
enum MergeSemantics {
    /// Fire-and-forget: responses are discarded.
    None,
    /// `before_tool_call` specific: any `skip: true` → skip,
    /// last non-null `modified_params` wins.
    ToolCallBefore,
    /// Last non-null value for the named field wins.
    LastNonNull { field: &'static str },
}

/// A mapping from a lifecycle event to a hook name and merge strategy.
struct HookMapping {
    hook_name: &'static str,
    merge: MergeSemantics,
}

// ── Hook Trigger Protocol ──────────────────────────────────────────────────────────

// The event published on `hook.v1.event.<hook_name>` is the canonical
// `astrid:hook/hook-event-request` shape — `astrid_sdk::contracts::hook::HookEventRequest`,
// shared with sage (the other producer) and the SDK consumer side. Per the
// WIT, `payload` is the lifecycle event serialized as a JSON STRING (not an
// inline value), so a subscriber using the canonical type / the SDK's
// `HookEvent::payload::<T>()` can deserialize it. Subscribers reply on
// `hook.v1.response.<hook_name>.<correlation_id>` when a correlation id is
// present; absent => fire-and-forget.

// ── Event-to-Hook Mapping Table ────────────────────────────────────────────────────

/// Resolve the hook mapping for a given event type string.
///
/// Returns `None` for events that have no corresponding hook.
fn mapping_for_event(event_type: &str) -> Option<HookMapping> {
    match event_type {
        // Session lifecycle
        "astrid.v1.lifecycle.session_created" => Some(HookMapping {
            hook_name: "session_start",
            merge: MergeSemantics::None,
        }),
        "astrid.v1.lifecycle.session_ended" => Some(HookMapping {
            hook_name: "session_end",
            merge: MergeSemantics::None,
        }),

        // Tool hooks
        "astrid.v1.lifecycle.tool_call_started" => Some(HookMapping {
            hook_name: "before_tool_call",
            merge: MergeSemantics::ToolCallBefore,
        }),
        "astrid.v1.lifecycle.tool_call_completed" => Some(HookMapping {
            hook_name: "after_tool_call",
            merge: MergeSemantics::LastNonNull {
                field: "modified_result",
            },
        }),
        "astrid.v1.lifecycle.tool_result_persisting" => Some(HookMapping {
            hook_name: "tool_result_persist",
            merge: MergeSemantics::LastNonNull {
                field: "transformed_result",
            },
        }),

        // Message hooks
        "astrid.v1.lifecycle.message_received" => Some(HookMapping {
            hook_name: "message_received",
            merge: MergeSemantics::None,
        }),
        "astrid.v1.lifecycle.message_sending" => Some(HookMapping {
            hook_name: "message_sending",
            merge: MergeSemantics::LastNonNull {
                field: "modified_content",
            },
        }),
        "astrid.v1.lifecycle.message_sent" => Some(HookMapping {
            hook_name: "message_sent",
            merge: MergeSemantics::None,
        }),

        // Sub-agent hooks
        "astrid.v1.lifecycle.sub_agent_spawned" => Some(HookMapping {
            hook_name: "subagent_start",
            merge: MergeSemantics::None,
        }),
        "astrid.v1.lifecycle.sub_agent_completed"
        | "astrid.v1.lifecycle.sub_agent_failed"
        | "astrid.v1.lifecycle.sub_agent_cancelled" => Some(HookMapping {
            hook_name: "subagent_stop",
            merge: MergeSemantics::None,
        }),

        // Context compaction (broadcast-only observation hooks)
        "astrid.v1.lifecycle.context_compaction_started" => Some(HookMapping {
            hook_name: "on_compaction_started",
            merge: MergeSemantics::None,
        }),
        "astrid.v1.lifecycle.context_compaction_completed" => Some(HookMapping {
            hook_name: "on_compaction_completed",
            merge: MergeSemantics::None,
        }),

        // Kernel lifecycle
        "astrid.v1.lifecycle.kernel_started" => Some(HookMapping {
            hook_name: "kernel_start",
            merge: MergeSemantics::None,
        }),
        "astrid.v1.lifecycle.kernel_shutdown" => Some(HookMapping {
            hook_name: "kernel_stop",
            merge: MergeSemantics::None,
        }),

        _ => Option::None,
    }
}

// ── Merge Logic ────────────────────────────────────────────────────────────────────

/// Apply merge semantics to a list of subscriber responses.
fn apply_merge(merge: &MergeSemantics, responses: &[serde_json::Value]) -> HookResult {
    match merge {
        MergeSemantics::None => HookResult {
            skip: Option::None,
            data: Option::None,
        },

        MergeSemantics::ToolCallBefore => {
            let mut skip = false;
            let mut last_params: Option<serde_json::Value> = Option::None;

            for resp in responses {
                // Any response with skip: true wins
                if resp.get("skip").and_then(|v| v.as_bool()).unwrap_or(false) {
                    skip = true;
                }
                // Last non-null modified_params wins
                if let Some(params) = resp.get("modified_params")
                    && !params.is_null()
                {
                    last_params = Some(params.clone());
                }
            }

            HookResult {
                skip: if skip { Some(true) } else { Option::None },
                data: last_params,
            }
        }

        MergeSemantics::LastNonNull { field } => {
            let mut last_value: Option<serde_json::Value> = Option::None;

            for resp in responses {
                if let Some(val) = resp.get(*field)
                    && !val.is_null()
                {
                    last_value = Some(val.clone());
                }
            }

            HookResult {
                skip: Option::None,
                data: last_value,
            }
        }
    }
}

// ── Correlation IDs ────────────────────────────────────────────────────────────────

/// Generate a 16-byte hex correlation id from the host CSPRNG.
///
/// We can't depend on `uuid` directly (not re-exported from
/// `astrid-sdk`), and using a monotonic timestamp would collide if two
/// dispatches landed in the same nanosecond. `runtime::random_bytes` is
/// the documented CSPRNG path.
fn correlation_id() -> Result<String, SysError> {
    let bytes = runtime::random_bytes(16)?;
    let mut s = String::with_capacity(32);
    for b in bytes {
        s.push(char::from_digit(u32::from(b >> 4), 16).unwrap_or('0'));
        s.push(char::from_digit(u32::from(b & 0x0F), 16).unwrap_or('0'));
    }
    Ok(s)
}

// ── Core Dispatch ──────────────────────────────────────────────────────────────────

/// Dispatch a lifecycle event through the hook system.
///
/// 1. Look up the event-to-hook mapping.
/// 2. For `MergeSemantics::None`: publish on
///    `hook.v1.event.<hook_name>` fire-and-forget.
/// 3. For merge cases: subscribe to a correlation-keyed reply topic,
///    publish the event with the correlation id, collect responses
///    until quiescence or deadline, apply the merge.
fn dispatch_hook(
    event_type: &str,
    payload: &serde_json::Value,
) -> Result<Option<HookResult>, SysError> {
    let Some(mapping) = mapping_for_event(event_type) else {
        // No hook mapping for this event — nothing to do.
        return Ok(Option::None);
    };

    let event_topic = format!("hook.v1.event.{}", mapping.hook_name);

    // Fire-and-forget for None-merge events. No responder is expected,
    // so we don't allocate a subscription handle.
    if matches!(mapping.merge, MergeSemantics::None) {
        let request = HookEventRequest {
            hook: mapping.hook_name.to_string(),
            payload: serde_json::to_string(payload)?,
            correlation_id: Option::None,
        };
        ipc::publish_json(&event_topic, &request)?;
        return Ok(Option::None);
    }

    // Fan-out + collect. Subscribe BEFORE publishing so a fast responder
    // can't beat us to the reply topic.
    let corr_id = correlation_id()?;
    let reply_topic = format!("hook.v1.response.{}.{corr_id}", mapping.hook_name);

    let sub = ipc::subscribe(&reply_topic)?;

    let request = HookEventRequest {
        hook: mapping.hook_name.to_string(),
        payload: serde_json::to_string(payload)?,
        correlation_id: Some(corr_id),
    };
    ipc::publish_json(&event_topic, &request)?;

    // Drain replies until the collection window closes, or `recv`
    // returns an empty batch (quiescence). The Drop on `sub` releases
    // the kernel-side subscription on every return path.
    let mut responses: Vec<serde_json::Value> = Vec::new();
    let start = time::monotonic();
    loop {
        let elapsed_ms = u64::try_from((time::monotonic().saturating_sub(start)).as_millis())
            .unwrap_or(HOOK_COLLECT_DEADLINE_MS);
        if elapsed_ms >= HOOK_COLLECT_DEADLINE_MS {
            break;
        }
        let remaining = if responses.is_empty() {
            HOOK_COLLECT_DEADLINE_MS - elapsed_ms
        } else {
            HOOK_QUIESCENCE_MS.min(HOOK_COLLECT_DEADLINE_MS - elapsed_ms)
        };

        match sub.recv(remaining) {
            Ok(poll) => {
                if poll.messages.is_empty() {
                    // Either the host returned early-empty or we hit the
                    // deadline without new messages. Either way, stop.
                    break;
                }
                for msg in poll.messages {
                    match serde_json::from_str::<serde_json::Value>(&msg.payload) {
                        Ok(v) => responses.push(v),
                        Err(e) => {
                            log::warn(format!(
                                "hook-bridge: dropping malformed reply on {reply_topic}: {e}"
                            ));
                        }
                    }
                }
            }
            Err(SysError::HostError(msg)) if msg.contains("Timeout") => {
                // No more replies inside the window. Done.
                break;
            }
            Err(e) => return Err(e),
        }
    }

    Ok(Some(apply_merge(&mapping.merge, &responses)))
}

fn canonical_hook_for_host_event(event: &str) -> Option<(&'static str, bool)> {
    match event {
        "session_start" => Some(("session_start", false)),
        "user_prompt_submit" => Some(("message_received", true)),
        "pre_tool_use" | "permission_request" => Some(("before_tool_call", false)),
        "post_tool_use" => Some(("after_tool_call", false)),
        "pre_compact" => Some(("on_compaction_started", false)),
        "post_compact" => Some(("on_compaction_completed", false)),
        "subagent_start" => Some(("subagent_start", false)),
        "subagent_stop" => Some(("subagent_stop", false)),
        "stop" | "session_end" => Some(("session_end", false)),
        _ => None,
    }
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

fn validate_oracle_hook(event: &OracleHookEvent) -> Result<(), &'static str> {
    if event.schema_version != 1 {
        return Err("unsupported schema version");
    }
    if !matches!(event.host.as_str(), "codex" | "claude" | "grok") {
        return Err("unsupported host");
    }
    if canonical_hook_for_host_event(&event.event).is_none() {
        return Err("unsupported host event");
    }
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
    Ok(())
}

fn collect_additional_context(
    subscription: &ipc::Subscription,
    reply_topic: &str,
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
                for message in poll.messages {
                    match serde_json::from_str::<serde_json::Value>(&message.payload) {
                        Ok(value) => {
                            if let Some(context) = value
                                .get("additional_context")
                                .and_then(serde_json::Value::as_str)
                                .filter(|context| !context.trim().is_empty())
                                && !push_context(&mut contexts, &mut context_bytes, context)
                            {
                                log::warn(format!(
                                    "hook-bridge: dropping host-hook context that exceeds the {MAX_HOST_CONTEXT_BYTES}-byte response limit"
                                ));
                            }
                        }
                        Err(error) => log::warn(format!(
                            "hook-bridge: dropping malformed host-hook reply on {reply_topic}: {error}"
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

fn dispatch_oracle_hook(event: &OracleHookEvent) -> Result<Option<String>, SysError> {
    let Some((hook, expects_context)) = canonical_hook_for_host_event(&event.event) else {
        return Ok(None);
    };
    let canonical_payload = CanonicalOraclePayload {
        principal_id: &event.principal_id,
        host: &event.host,
        session_id: &event.session_id,
        source_event: &event.event,
        turn_id: event.turn_id.as_deref(),
        workspace_id: event.workspace_id.as_deref(),
        payload: &event.payload,
    };
    let event_topic = format!("hook.v1.event.{hook}");
    if !expects_context {
        let request = HookEventRequest {
            hook: hook.to_owned(),
            payload: serde_json::to_string(&canonical_payload)?,
            correlation_id: None,
        };
        ipc::publish_json(&event_topic, &request)?;
        return Ok(None);
    }

    let reply_topic = format!("hook.v1.response.{hook}.{}", event.correlation_id);
    let subscription = ipc::subscribe(&reply_topic)?;
    let request = HookEventRequest {
        hook: hook.to_owned(),
        payload: serde_json::to_string(&canonical_payload)?,
        correlation_id: Some(event.correlation_id.clone()),
    };
    ipc::publish_json(&event_topic, &request)?;
    collect_additional_context(&subscription, &reply_topic)
}

fn handle_oracle_hook(payload: serde_json::Value) -> Result<(), SysError> {
    let event: OracleHookEvent = match serde_json::from_value(payload) {
        Ok(event) => event,
        Err(error) => {
            log::warn(format!(
                "hook-bridge: dropping malformed oracle hook: {error}"
            ));
            return Ok(());
        }
    };
    if let Err(reason) = validate_oracle_hook(&event) {
        log::warn(format!(
            "hook-bridge: dropping invalid {} hook '{}': {reason}",
            event.host, event.event
        ));
        return Ok(());
    }
    let caller = runtime::caller()?;
    if caller.principal.as_deref() != Some(event.principal_id.as_str()) {
        log::warn(format!(
            "hook-bridge: dropping hook with principal mismatch for host {}",
            event.host
        ));
        return Ok(());
    }

    let context = dispatch_oracle_hook(&event)?;
    let response_topic = format!("oracle.v1.hook.response.{}", event.delivery_id);
    ipc::publish_json(
        &response_topic,
        &OracleHookResponse {
            schema_version: 1,
            principal_id: &event.principal_id,
            host: &event.host,
            session_id: &event.session_id,
            event: &event.event,
            correlation_id: &event.correlation_id,
            route_id: &event.route_id,
            delivery_id: &event.delivery_id,
            context,
        },
    )
}

// ── Capsule Implementation ─────────────────────────────────────────────────────────

/// Hook Bridge capsule.
///
/// Maps lifecycle events to semantic hooks, fans out to subscribers via
/// IPC, and applies merge strategies to the responses.
#[derive(Default)]
pub struct HookBridge;

/// Extract event type and dispatch the hook. Used by all interceptor handlers.
fn handle_lifecycle(
    event_type: &str,
    payload: serde_json::Value,
) -> Result<Option<HookResult>, SysError> {
    dispatch_hook(event_type, &payload)
}

#[capsule]
impl HookBridge {
    /// Normalize a token-validated Codex host hook onto the canonical hook bus.
    #[astrid::interceptor("on_codex_hook")]
    pub fn on_codex_hook(&self, payload: serde_json::Value) -> Result<(), SysError> {
        handle_oracle_hook(payload)
    }

    /// Normalize a token-validated Claude host hook onto the canonical hook bus.
    #[astrid::interceptor("on_claude_hook")]
    pub fn on_claude_hook(&self, payload: serde_json::Value) -> Result<(), SysError> {
        handle_oracle_hook(payload)
    }

    /// Normalize a token-validated Grok host hook onto the canonical hook bus.
    #[astrid::interceptor("on_grok_hook")]
    pub fn on_grok_hook(&self, payload: serde_json::Value) -> Result<(), SysError> {
        handle_oracle_hook(payload)
    }

    // ── Session lifecycle ──

    /// Handle `session_created` lifecycle event.
    #[astrid::interceptor("on_session_created")]
    pub fn on_session_created(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("astrid.v1.lifecycle.session_created", payload)?;
        Ok(())
    }

    /// Handle `session_ended` lifecycle event.
    #[astrid::interceptor("on_session_ended")]
    pub fn on_session_ended(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("astrid.v1.lifecycle.session_ended", payload)?;
        Ok(())
    }

    // ── Tool hooks ──

    /// Handle `tool_call_started` — maps to `before_tool_call` hook.
    ///
    /// Returns merged result with potential skip/modified_params.
    #[astrid::interceptor("on_tool_call_started")]
    pub fn on_tool_call_started(
        &self,
        payload: serde_json::Value,
    ) -> Result<Option<HookResult>, SysError> {
        handle_lifecycle("astrid.v1.lifecycle.tool_call_started", payload)
    }

    /// Handle `tool_call_completed` — maps to `after_tool_call` hook.
    #[astrid::interceptor("on_tool_call_completed")]
    pub fn on_tool_call_completed(
        &self,
        payload: serde_json::Value,
    ) -> Result<Option<HookResult>, SysError> {
        handle_lifecycle("astrid.v1.lifecycle.tool_call_completed", payload)
    }

    /// Handle `tool_result_persisting` — maps to `tool_result_persist` hook.
    #[astrid::interceptor("on_tool_result_persisting")]
    pub fn on_tool_result_persisting(
        &self,
        payload: serde_json::Value,
    ) -> Result<Option<HookResult>, SysError> {
        handle_lifecycle("astrid.v1.lifecycle.tool_result_persisting", payload)
    }

    // ── Message hooks ──

    /// Handle `message_received` lifecycle event.
    #[astrid::interceptor("on_message_received")]
    pub fn on_message_received(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("astrid.v1.lifecycle.message_received", payload)?;
        Ok(())
    }

    /// Handle `message_sending` — maps to `message_sending` hook.
    #[astrid::interceptor("on_message_sending")]
    pub fn on_message_sending(
        &self,
        payload: serde_json::Value,
    ) -> Result<Option<HookResult>, SysError> {
        handle_lifecycle("astrid.v1.lifecycle.message_sending", payload)
    }

    /// Handle `message_sent` lifecycle event.
    #[astrid::interceptor("on_message_sent")]
    pub fn on_message_sent(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("astrid.v1.lifecycle.message_sent", payload)?;
        Ok(())
    }

    // ── Sub-agent hooks ──

    /// Handle `sub_agent_spawned` lifecycle event.
    #[astrid::interceptor("on_subagent_spawned")]
    pub fn on_subagent_spawned(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("astrid.v1.lifecycle.sub_agent_spawned", payload)?;
        Ok(())
    }

    /// Handle `sub_agent_completed` lifecycle event.
    #[astrid::interceptor("on_subagent_completed")]
    pub fn on_subagent_completed(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("astrid.v1.lifecycle.sub_agent_completed", payload)?;
        Ok(())
    }

    /// Handle `sub_agent_failed` lifecycle event.
    #[astrid::interceptor("on_subagent_failed")]
    pub fn on_subagent_failed(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("astrid.v1.lifecycle.sub_agent_failed", payload)?;
        Ok(())
    }

    /// Handle `sub_agent_cancelled` lifecycle event.
    #[astrid::interceptor("on_subagent_cancelled")]
    pub fn on_subagent_cancelled(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("astrid.v1.lifecycle.sub_agent_cancelled", payload)?;
        Ok(())
    }

    // ── Context compaction ──

    /// Handle `context_compaction_started` lifecycle event.
    #[astrid::interceptor("on_compaction_started")]
    pub fn on_compaction_started(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("astrid.v1.lifecycle.context_compaction_started", payload)?;
        Ok(())
    }

    /// Handle `context_compaction_completed` lifecycle event.
    #[astrid::interceptor("on_compaction_completed")]
    pub fn on_compaction_completed(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("astrid.v1.lifecycle.context_compaction_completed", payload)?;
        Ok(())
    }

    // ── Kernel lifecycle ──

    /// Handle `kernel_started` lifecycle event.
    #[astrid::interceptor("on_kernel_started")]
    pub fn on_kernel_started(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("astrid.v1.lifecycle.kernel_started", payload)?;
        Ok(())
    }

    /// Handle `kernel_shutdown` lifecycle event.
    #[astrid::interceptor("on_kernel_shutdown")]
    pub fn on_kernel_shutdown(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let _ = handle_lifecycle("astrid.v1.lifecycle.kernel_shutdown", payload)?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn host_event() -> OracleHookEvent {
        let route_id = "a".repeat(64);
        let correlation_id = "b".repeat(32);
        OracleHookEvent {
            schema_version: 1,
            principal_id: "codex-code".to_owned(),
            host: "codex".to_owned(),
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
    fn prompt_event_maps_to_context_bearing_message_hook() {
        assert_eq!(
            canonical_hook_for_host_event("user_prompt_submit"),
            Some(("message_received", true))
        );
        assert_eq!(
            canonical_hook_for_host_event("stop"),
            Some(("session_end", false))
        );
    }

    #[test]
    fn exact_delivery_binding_is_required() {
        let mut event = host_event();
        assert!(validate_oracle_hook(&event).is_ok());
        event.delivery_id = format!("{}-{}", "c".repeat(64), event.correlation_id);
        assert_eq!(
            validate_oracle_hook(&event),
            Err("delivery identifier does not bind route and correlation")
        );
    }

    #[test]
    fn event_and_session_cannot_add_topic_segments() {
        let mut event = host_event();
        event.session_id = "codex.other".to_owned();
        assert_eq!(validate_oracle_hook(&event), Err("invalid routed segment"));
    }

    #[test]
    fn combined_host_context_stays_inside_relay_limit() {
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
}
