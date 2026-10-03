//! Host-neutral policy output. Native host schemas belong to Oracle.

use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Decision {
    skip: bool,
    #[serde(default)]
    ask: bool,
    #[serde(default)]
    reason: Option<String>,
}

#[derive(Serialize)]
struct Reply<'a> {
    schema_version: u8,
    event: &'a str,
    decision: Option<&'a Decision>,
    context: Option<&'a str>,
}

pub(super) fn reply(
    event: &str,
    decision: Option<&Decision>,
    context: Option<&str>,
) -> Result<Option<String>, String> {
    serde_json::to_string(&Reply {
        schema_version: 1,
        event,
        decision,
        context,
    })
    .map(Some)
    .map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decision_and_context_are_preserved_without_host_translation()
    -> Result<(), Box<dyn std::error::Error>> {
        let decision = Decision {
            skip: false,
            ask: true,
            reason: Some("approval".into()),
        };
        let encoded =
            reply("custom_event", Some(&decision), Some("context"))?.ok_or("missing JSON reply")?;
        let value: serde_json::Value = serde_json::from_str(&encoded)?;
        assert_eq!(
            value,
            serde_json::json!({
                "schema_version":1, "event":"custom_event",
                "decision":{"skip":false,"ask":true,"reason":"approval"},
                "context":"context"
            })
        );
        Ok(())
    }
}
