//! Host-neutral policy output. Native host schemas belong to Oracle.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Decision {
    pub(super) skip: bool,
    #[serde(default)]
    pub(super) ask: bool,
    #[serde(default)]
    pub(super) reason: Option<String>,
}

/// Keep each policy's veto while retaining Oracle's context separately.
pub(super) fn combine(existing: Option<Decision>, codewall: Decision) -> Decision {
    let Some(existing) = existing else {
        return codewall;
    };
    if existing.skip {
        existing
    } else if codewall.skip {
        codewall
    } else if existing.ask {
        existing
    } else if codewall.ask {
        codewall
    } else {
        existing
    }
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
    fn independent_policy_denial_survives_codewall_approval() {
        let existing = Decision {
            skip: true,
            ask: false,
            reason: Some("other policy".into()),
        };
        let codewall = Decision {
            skip: false,
            ask: false,
            reason: None,
        };
        let combined = combine(Some(existing), codewall);
        assert!(combined.skip);
        assert_eq!(combined.reason.as_deref(), Some("other policy"));
    }

    #[test]
    fn codewall_denial_wins_over_existing_ask() {
        let existing = Decision {
            skip: false,
            ask: true,
            reason: Some("review".into()),
        };
        let codewall = Decision {
            skip: true,
            ask: false,
            reason: Some("blocked".into()),
        };
        let combined = combine(Some(existing), codewall);
        assert!(combined.skip);
        assert_eq!(combined.reason.as_deref(), Some("blocked"));
    }

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
