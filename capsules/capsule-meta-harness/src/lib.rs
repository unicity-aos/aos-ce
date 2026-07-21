#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![warn(missing_docs)]

//! Meta Harness activation for host-agent prompt turns.

use astrid_sdk::prelude::*;
use serde::Deserialize;

const ADAPTIVE_CONTEXT: &str = "Work on the user's request normally. Privately, within this same turn, notice whether the work reveals a missing ability, recurring friction, or reusable leverage in your Unicity AOS user-space world. If useful, proactively use the Meta Harness skill and decide whether to reuse, remember, compose, improve the harness, or Forge-build within the user's intent and your existing authority. Do not add a second response, announce this check, or report that nothing was found.";

const PROPOSE_CONTEXT: &str = "Work on the user's request normally. Privately, within this same turn, notice whether the work reveals a missing ability, recurring friction, or reusable leverage in your Unicity AOS user-space world. If useful, use the Meta Harness skill to form a concrete capability proposal and surface it only when it materially helps the user. Do not make durable changes without the approval or authority the work normally requires. Do not add a second response, announce this check, or report that nothing was found.";

const AUTOMATIC_CONTEXT: &str = "Work on the user's request normally. Privately, within this same turn, notice whether the work reveals a missing ability, recurring friction, or reusable leverage in your Unicity AOS user-space world. If useful, proactively use the Meta Harness skill and directly reuse, remember, compose, improve the harness, or Forge-build when that action is already inside the user's intent and your existing authority. Ask only when the action normally needs new authority. Do not add a second response, announce this check, or report that nothing was found.";

#[derive(Debug, Deserialize)]
struct HookRequest {
    payload: String,
    #[serde(default)]
    correlation_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct CanonicalHostPayload {
    host: String,
    session_id: String,
    source_event: String,
}

/// Proactive reflection responder for host prompt turns.
#[derive(Default)]
pub struct MetaHarness;

fn is_correlation(value: &str) -> bool {
    value.len() == 32
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn activation_context() -> Result<Option<&'static str>, SysError> {
    match env::var("activation")?.trim().to_ascii_lowercase().as_str() {
        "" | "adaptive" => Ok(Some(ADAPTIVE_CONTEXT)),
        "propose" => Ok(Some(PROPOSE_CONTEXT)),
        "automatic" => Ok(Some(AUTOMATIC_CONTEXT)),
        "off" => Ok(None),
        other => {
            log::warn(format!(
                "meta-harness: unknown activation mode '{other}', using adaptive"
            ));
            Ok(Some(ADAPTIVE_CONTEXT))
        }
    }
}

#[capsule]
impl MetaHarness {
    /// Add private same-turn reflection context to an exact host prompt turn.
    #[astrid::interceptor("on_message_received")]
    pub fn on_message_received(&self, payload: serde_json::Value) -> Result<(), SysError> {
        let request: HookRequest = match serde_json::from_value(payload) {
            Ok(request) => request,
            Err(error) => {
                log::warn(format!("meta-harness: malformed hook request: {error}"));
                return Ok(());
            }
        };
        let Some(correlation_id) = request.correlation_id else {
            return Ok(());
        };
        if !is_correlation(&correlation_id) {
            log::warn("meta-harness: ignored unroutable hook correlation");
            return Ok(());
        }
        let canonical: CanonicalHostPayload = match serde_json::from_str(&request.payload) {
            Ok(canonical) => canonical,
            Err(error) => {
                log::warn(format!(
                    "meta-harness: malformed canonical payload: {error}"
                ));
                return Ok(());
            }
        };
        if canonical.source_event != "user_prompt_submit"
            || !matches!(canonical.host.as_str(), "codex" | "claude" | "grok")
            || canonical.session_id.is_empty()
        {
            return Ok(());
        }
        let Some(context) = activation_context()? else {
            return Ok(());
        };
        ipc::publish_json(
            &format!("hook.v1.response.message_received.{correlation_id}"),
            &serde_json::json!({ "additional_context": context }),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn correlation_is_one_lower_hex_segment() {
        assert!(is_correlation(&"a".repeat(32)));
        assert!(!is_correlation("a.b"));
        assert!(!is_correlation(&"A".repeat(32)));
        assert!(!is_correlation(&"a".repeat(31)));
    }

    #[test]
    fn context_is_same_turn_and_non_reporting() {
        assert!(ADAPTIVE_CONTEXT.contains("within this same turn"));
        assert!(ADAPTIVE_CONTEXT.contains("Do not add a second response"));
        assert!(ADAPTIVE_CONTEXT.contains("report that nothing was found"));
    }
}
