#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![warn(missing_docs)]

//! Canonical Astrid lifecycle-to-hook router.
//!
//! Frontend-specific protocols are translated by adapter capsules before they
//! reach the canonical `hook.v1.*` bus. This capsule owns only Astrid lifecycle
//! mapping and canonical response merge policy.

use astrid_sdk::contracts::hook::{HookEventRequest, HookResult};
use astrid_sdk::prelude::*;

const HOOK_COLLECT_DEADLINE_MS: u64 = 5_000;
const HOOK_QUIESCENCE_MS: u64 = 25;
const MAX_HOOK_RESPONSE_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq)]
enum MergeSemantics {
    None,
    ToolCallBefore,
    LastNonNull { field: &'static str },
}

struct HookMapping {
    hook_name: &'static str,
    merge: MergeSemantics,
}

#[derive(Default)]
struct ResponseBatch {
    values: Vec<serde_json::Value>,
    complete: bool,
}

fn mapping_for_event(event_type: &str) -> Option<HookMapping> {
    match event_type {
        "astrid.v1.lifecycle.session_created" => Some(HookMapping {
            hook_name: "session_start",
            merge: MergeSemantics::None,
        }),
        "astrid.v1.lifecycle.session_ended" => Some(HookMapping {
            hook_name: "session_end",
            merge: MergeSemantics::None,
        }),
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
        "astrid.v1.lifecycle.context_compaction_started" => Some(HookMapping {
            hook_name: "on_compaction_started",
            merge: MergeSemantics::None,
        }),
        "astrid.v1.lifecycle.context_compaction_completed" => Some(HookMapping {
            hook_name: "on_compaction_completed",
            merge: MergeSemantics::None,
        }),
        "astrid.v1.lifecycle.kernel_started" => Some(HookMapping {
            hook_name: "kernel_start",
            merge: MergeSemantics::None,
        }),
        "astrid.v1.lifecycle.kernel_shutdown" => Some(HookMapping {
            hook_name: "kernel_stop",
            merge: MergeSemantics::None,
        }),
        _ => None,
    }
}

fn apply_merge(merge: &MergeSemantics, responses: &[serde_json::Value]) -> HookResult {
    match merge {
        MergeSemantics::None => HookResult {
            skip: None,
            data: None,
        },
        MergeSemantics::ToolCallBefore => {
            let mut skip = false;
            let mut last_params = None;
            for response in responses {
                skip |= response
                    .get("skip")
                    .and_then(serde_json::Value::as_bool)
                    .unwrap_or(false);
                if let Some(params) = response.get("modified_params")
                    && !params.is_null()
                {
                    last_params = Some(params.clone());
                }
            }
            HookResult {
                skip: skip.then_some(true),
                data: last_params.map(|value| value.to_string()),
            }
        }
        MergeSemantics::LastNonNull { field } => HookResult {
            skip: None,
            data: responses
                .iter()
                .filter_map(|response| response.get(*field))
                .rfind(|value| !value.is_null())
                .map(serde_json::Value::to_string),
        },
    }
}

fn merge_batch(merge: &MergeSemantics, batch: ResponseBatch) -> HookResult {
    if batch.complete {
        return apply_merge(merge, &batch.values);
    }

    // Never apply a partial transform. For the security-sensitive pre-tool
    // hook, a transport integrity failure is a binding denial.
    match merge {
        MergeSemantics::ToolCallBefore => HookResult {
            skip: Some(true),
            data: None,
        },
        MergeSemantics::None | MergeSemantics::LastNonNull { .. } => HookResult {
            skip: None,
            data: None,
        },
    }
}

fn correlation_id() -> Result<String, SysError> {
    let bytes = runtime::random_bytes(16)?;
    let mut output = String::with_capacity(32);
    for byte in bytes {
        output.push(char::from_digit(u32::from(byte >> 4), 16).unwrap_or('0'));
        output.push(char::from_digit(u32::from(byte & 0x0f), 16).unwrap_or('0'));
    }
    Ok(output)
}

fn collect_responses(
    subscription: &ipc::Subscription,
    reply_topic: &str,
    principal: Option<&str>,
) -> Result<ResponseBatch, SysError> {
    let mut batch = ResponseBatch {
        complete: true,
        ..ResponseBatch::default()
    };
    let start = time::monotonic();
    loop {
        let elapsed_ms = u64::try_from((time::monotonic().saturating_sub(start)).as_millis())
            .unwrap_or(HOOK_COLLECT_DEADLINE_MS);
        if elapsed_ms >= HOOK_COLLECT_DEADLINE_MS {
            break;
        }
        let remaining = if batch.values.is_empty() {
            HOOK_COLLECT_DEADLINE_MS - elapsed_ms
        } else {
            HOOK_QUIESCENCE_MS.min(HOOK_COLLECT_DEADLINE_MS - elapsed_ms)
        };
        match subscription.recv(remaining) {
            Ok(poll) if poll.messages.is_empty() => break,
            Ok(poll) => {
                if poll.dropped != 0 || poll.lagged != 0 {
                    batch.complete = false;
                    log::warn(format!(
                        "hook-bridge: response fan-out on {reply_topic} lost messages"
                    ));
                }
                for message in poll.messages {
                    if message.topic != reply_topic || message.principal.verified() != principal {
                        batch.complete = false;
                        log::warn(format!(
                            "hook-bridge: dropping response with mismatched route or principal on {reply_topic}"
                        ));
                        continue;
                    }
                    if message.payload.len() > MAX_HOOK_RESPONSE_BYTES {
                        batch.complete = false;
                        log::warn(format!(
                            "hook-bridge: dropping oversized reply on {reply_topic}"
                        ));
                        continue;
                    }
                    match serde_json::from_str(&message.payload) {
                        Ok(value) => batch.values.push(value),
                        Err(error) => {
                            batch.complete = false;
                            log::warn(format!(
                                "hook-bridge: dropping malformed reply on {reply_topic}: {error}"
                            ));
                        }
                    }
                }
            }
            Err(SysError::HostError(message)) if message.contains("Timeout") => break,
            Err(error) => return Err(error),
        }
    }
    Ok(batch)
}

fn dispatch_hook(
    event_type: &str,
    payload: &serde_json::Value,
) -> Result<Option<HookResult>, SysError> {
    let Some(mapping) = mapping_for_event(event_type) else {
        return Ok(None);
    };
    let event_topic = format!("hook.v1.event.{}", mapping.hook_name);
    if matches!(mapping.merge, MergeSemantics::None) {
        ipc::publish_json(
            &event_topic,
            &HookEventRequest {
                hook: mapping.hook_name.to_owned(),
                payload: serde_json::to_string(payload)?,
                correlation_id: None,
            },
        )?;
        return Ok(None);
    }

    let correlation = correlation_id()?;
    let caller = runtime::caller()?;
    let reply_topic = format!("hook.v1.response.{}.{}", mapping.hook_name, correlation);
    let subscription = ipc::subscribe(&reply_topic)?;
    ipc::publish_json(
        &event_topic,
        &HookEventRequest {
            hook: mapping.hook_name.to_owned(),
            payload: serde_json::to_string(payload)?,
            correlation_id: Some(correlation),
        },
    )?;
    Ok(Some(merge_batch(
        &mapping.merge,
        collect_responses(&subscription, &reply_topic, caller.principal.as_deref())?,
    )))
}

/// Canonical lifecycle hook router.
#[derive(Default)]
pub struct HookBridge;

fn handle_lifecycle(
    event_type: &str,
    payload: serde_json::Value,
) -> Result<Option<HookResult>, SysError> {
    dispatch_hook(event_type, &payload)
}

#[capsule]
impl HookBridge {
    /// Route session creation.
    #[astrid::interceptor("on_session_created")]
    pub fn on_session_created(&self, payload: serde_json::Value) -> Result<(), SysError> {
        handle_lifecycle("astrid.v1.lifecycle.session_created", payload).map(drop)
    }

    /// Route session completion.
    #[astrid::interceptor("on_session_ended")]
    pub fn on_session_ended(&self, payload: serde_json::Value) -> Result<(), SysError> {
        handle_lifecycle("astrid.v1.lifecycle.session_ended", payload).map(drop)
    }

    /// Route a binding before-tool lifecycle hook.
    #[astrid::interceptor("on_tool_call_started")]
    pub fn on_tool_call_started(
        &self,
        payload: serde_json::Value,
    ) -> Result<Option<HookResult>, SysError> {
        handle_lifecycle("astrid.v1.lifecycle.tool_call_started", payload)
    }

    /// Route tool completion.
    #[astrid::interceptor("on_tool_call_completed")]
    pub fn on_tool_call_completed(
        &self,
        payload: serde_json::Value,
    ) -> Result<Option<HookResult>, SysError> {
        handle_lifecycle("astrid.v1.lifecycle.tool_call_completed", payload)
    }

    /// Route tool-result persistence.
    #[astrid::interceptor("on_tool_result_persisting")]
    pub fn on_tool_result_persisting(
        &self,
        payload: serde_json::Value,
    ) -> Result<Option<HookResult>, SysError> {
        handle_lifecycle("astrid.v1.lifecycle.tool_result_persisting", payload)
    }

    /// Route inbound message observation.
    #[astrid::interceptor("on_message_received")]
    pub fn on_message_received(&self, payload: serde_json::Value) -> Result<(), SysError> {
        handle_lifecycle("astrid.v1.lifecycle.message_received", payload).map(drop)
    }

    /// Route a binding message-send hook.
    #[astrid::interceptor("on_message_sending")]
    pub fn on_message_sending(
        &self,
        payload: serde_json::Value,
    ) -> Result<Option<HookResult>, SysError> {
        handle_lifecycle("astrid.v1.lifecycle.message_sending", payload)
    }

    /// Route sent-message observation.
    #[astrid::interceptor("on_message_sent")]
    pub fn on_message_sent(&self, payload: serde_json::Value) -> Result<(), SysError> {
        handle_lifecycle("astrid.v1.lifecycle.message_sent", payload).map(drop)
    }

    /// Route sub-agent creation.
    #[astrid::interceptor("on_subagent_spawned")]
    pub fn on_subagent_spawned(&self, payload: serde_json::Value) -> Result<(), SysError> {
        handle_lifecycle("astrid.v1.lifecycle.sub_agent_spawned", payload).map(drop)
    }

    /// Route successful sub-agent completion.
    #[astrid::interceptor("on_subagent_completed")]
    pub fn on_subagent_completed(&self, payload: serde_json::Value) -> Result<(), SysError> {
        handle_lifecycle("astrid.v1.lifecycle.sub_agent_completed", payload).map(drop)
    }

    /// Route failed sub-agent completion.
    #[astrid::interceptor("on_subagent_failed")]
    pub fn on_subagent_failed(&self, payload: serde_json::Value) -> Result<(), SysError> {
        handle_lifecycle("astrid.v1.lifecycle.sub_agent_failed", payload).map(drop)
    }

    /// Route cancelled sub-agent completion.
    #[astrid::interceptor("on_subagent_cancelled")]
    pub fn on_subagent_cancelled(&self, payload: serde_json::Value) -> Result<(), SysError> {
        handle_lifecycle("astrid.v1.lifecycle.sub_agent_cancelled", payload).map(drop)
    }

    /// Route compaction start.
    #[astrid::interceptor("on_compaction_started")]
    pub fn on_compaction_started(&self, payload: serde_json::Value) -> Result<(), SysError> {
        handle_lifecycle("astrid.v1.lifecycle.context_compaction_started", payload).map(drop)
    }

    /// Route compaction completion.
    #[astrid::interceptor("on_compaction_completed")]
    pub fn on_compaction_completed(&self, payload: serde_json::Value) -> Result<(), SysError> {
        handle_lifecycle("astrid.v1.lifecycle.context_compaction_completed", payload).map(drop)
    }

    /// Route kernel start.
    #[astrid::interceptor("on_kernel_started")]
    pub fn on_kernel_started(&self, payload: serde_json::Value) -> Result<(), SysError> {
        handle_lifecycle("astrid.v1.lifecycle.kernel_started", payload).map(drop)
    }

    /// Route kernel shutdown.
    #[astrid::interceptor("on_kernel_shutdown")]
    pub fn on_kernel_shutdown(&self, payload: serde_json::Value) -> Result<(), SysError> {
        handle_lifecycle("astrid.v1.lifecycle.kernel_shutdown", payload).map(drop)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lifecycle_mappings_keep_binding_and_observation_distinct() {
        let before = mapping_for_event("astrid.v1.lifecycle.tool_call_started").unwrap();
        assert_eq!(before.hook_name, "before_tool_call");
        assert_eq!(before.merge, MergeSemantics::ToolCallBefore);
        let received = mapping_for_event("astrid.v1.lifecycle.message_received").unwrap();
        assert_eq!(received.hook_name, "message_received");
        assert_eq!(received.merge, MergeSemantics::None);
    }

    #[test]
    fn before_tool_call_is_deny_wins_and_last_transform_wins() {
        let responses = vec![
            serde_json::json!({"modified_params": {"value": 1}}),
            serde_json::json!({"skip": true}),
            serde_json::json!({"modified_params": {"value": 2}}),
        ];
        assert_eq!(
            apply_merge(&MergeSemantics::ToolCallBefore, &responses),
            HookResult {
                skip: Some(true),
                data: Some(r#"{"value":2}"#.to_owned()),
            }
        );
    }

    #[test]
    fn last_non_null_ignores_nulls() {
        let responses = vec![
            serde_json::json!({"modified_content": "first"}),
            serde_json::json!({"modified_content": null}),
            serde_json::json!({"modified_content": "last"}),
        ];
        assert_eq!(
            apply_merge(
                &MergeSemantics::LastNonNull {
                    field: "modified_content"
                },
                &responses
            ),
            HookResult {
                skip: None,
                data: Some(r#""last""#.to_owned()),
            }
        );
    }

    #[test]
    fn merged_result_uses_the_canonical_hook_wire_shape() {
        let result = apply_merge(
            &MergeSemantics::LastNonNull {
                field: "modified_content",
            },
            &[serde_json::json!({"modified_content": {"text": "hello"}})],
        );
        assert_eq!(
            serde_json::to_value(result).unwrap(),
            serde_json::json!({
                "data": r#"{"text":"hello"}"#,
            })
        );
    }

    #[test]
    fn incomplete_pretool_fanout_fails_closed_without_partial_transform() {
        let result = merge_batch(
            &MergeSemantics::ToolCallBefore,
            ResponseBatch {
                values: vec![serde_json::json!({"modified_params": {"value": 1}})],
                complete: false,
            },
        );
        assert_eq!(
            result,
            HookResult {
                skip: Some(true),
                data: None,
            }
        );
    }

    #[test]
    fn incomplete_transform_fanout_discards_partial_output() {
        let result = merge_batch(
            &MergeSemantics::LastNonNull {
                field: "modified_content",
            },
            ResponseBatch {
                values: vec![serde_json::json!({"modified_content": "partial"})],
                complete: false,
            },
        );
        assert_eq!(
            result,
            HookResult {
                skip: None,
                data: None,
            }
        );
    }
}
