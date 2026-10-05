//! Test-only responder; the production adapter authenticates its source.
use astrid_sdk::prelude::*;

#[derive(Default)]
pub struct Policy;

#[capsule]
impl Policy {
    #[astrid::run]
    fn run(&self) -> Result<(), SysError> {
        let subscription = ipc::subscribe("hook.v1.event.*")?;
        runtime::signal_ready()?;
        loop {
            for message in subscription.recv(1_000)?.messages {
                let request: serde_json::Value = serde_json::from_str(&message.payload)
                    .map_err(|error| SysError::ApiError(error.to_string()))?;
                let hook = request["hook"].as_str().unwrap_or_default();
                let correlation = request["correlation_id"].as_str().unwrap_or_default();
                if correlation.is_empty() {
                    continue; // Observations deliberately do not solicit replies.
                }
                if hook.is_empty()
                    || !hook
                        .bytes()
                        .chain(correlation.bytes())
                        .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'-'))
                {
                    return Err(SysError::ApiError("invalid test correlation".into()));
                }
                let mode = env::var("QA_MODE")?;
                let reply = match mode.as_str() {
                    "silent" => continue,
                    "deny" => serde_json::json!({"skip":true,"reason":"test-policy-deny"}),
                    "malformed" => serde_json::json!({"skip":"invalid"}),
                    "context" => {
                        serde_json::json!({"additional_context":format!("observed:{hook}")})
                    }
                    "allow" => serde_json::json!({"skip":false}),
                    _ => return Err(SysError::ApiError("invalid test response mode".into())),
                };
                ipc::publish_json(&format!("hook.v1.response.{hook}.{correlation}"), &reply)?;
            }
        }
    }
}
