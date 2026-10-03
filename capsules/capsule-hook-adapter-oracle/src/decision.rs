//! Required, principal-scoped policy replies. Silence is not permission.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub(super) struct Decision {
    pub skip: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub ask: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
}

fn is_false(value: &bool) -> bool {
    !value
}

impl Decision {
    pub(crate) fn unavailable() -> Self {
        Self {
            skip: true,
            ask: false,
            reason: Some(
                "A required AOS policy responder is unavailable; retry when AOS is ready.".into(),
            ),
        }
    }
}

pub(super) struct RequiredReplies {
    pending: BTreeSet<String>,
    required: BTreeSet<String>,
    decision: Decision,
    invalid: bool,
}

impl RequiredReplies {
    pub(crate) fn parse(config: &str) -> Result<Self, ()> {
        let mut policies = BTreeMap::new();
        // The kernel limits each appended value, not the aggregate history.
        // Do not turn that per-request ceiling into an unrecoverable policy
        // failure: later named corrections and tombstones must still apply.
        if !config.trim().is_empty() {
            let registrations: Vec<String> = serde_json::from_str(config).map_err(|_| ())?;
            for registration in &registrations {
                let (name, source) = registration.split_once('=').ok_or(())?;
                if name.is_empty()
                    || name.len() > 128
                    || !name.bytes().all(|byte| {
                        byte.is_ascii_lowercase()
                            || byte.is_ascii_digit()
                            || matches!(byte, b'-' | b'_')
                    })
                {
                    return Err(());
                }
                // The authenticated admin append operation preserves other
                // policies. A later revision replaces only this named policy.
                // Only an explicit tombstone unregisters this policy. Empty or
                // malformed identities still fail closed.
                if source == "-" {
                    policies.remove(name);
                    continue;
                }
                policies.insert(name.to_owned(), source.to_owned());
            }
        }
        // Validate the effective revision, not superseded values. An explicit
        // repair can replace a malformed identity for the same named policy;
        // malformed names above cannot be attributed and remain fail-closed.
        for source in policies.values() {
            if source.len() != 36
                || !source.bytes().enumerate().all(|(index, byte)| {
                    if matches!(index, 8 | 13 | 18 | 23) {
                        byte == b'-'
                    } else {
                        byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)
                    }
                })
            {
                return Err(());
            }
        }
        let required: BTreeSet<String> = policies.into_values().collect();
        Ok(Self {
            pending: required.clone(),
            required,
            decision: Decision::default(),
            invalid: false,
        })
    }

    pub(crate) fn complete(&self) -> bool {
        self.pending.is_empty()
    }

    /// Keep a bounded fan-out window after quorum; a fast verdict must not
    /// suppress optional context arriving in the next IPC poll.
    pub(crate) fn next_wait(&self, elapsed: u64, deadline: u64, quiescence: u64) -> Option<u64> {
        let remaining = deadline.checked_sub(elapsed).filter(|left| *left > 0)?;
        Some(if self.complete() {
            remaining.min(quiescence)
        } else {
            remaining
        })
    }

    pub(crate) fn accept(&mut self, source: &str, payload: &str) {
        if !self.required.contains(source) {
            return;
        }
        let Ok(reply) = serde_json::from_str::<Decision>(payload) else {
            self.invalid = true;
            return;
        };
        if reply
            .reason
            .as_ref()
            .is_some_and(|reason| reason.len() > 4096)
        {
            self.invalid = true;
            return;
        }
        self.pending.remove(source);
        // Never let an allow or ask erase a prior veto, including duplicates.
        if reply.skip || (!self.decision.skip && reply.ask) {
            self.decision = reply;
        }
    }

    pub(crate) fn finish(self) -> Decision {
        if self.invalid || !self.complete() {
            Decision::unavailable()
        } else {
            self.decision
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const FIRST: &str = "11111111-1111-4111-8111-111111111111";
    const SECOND: &str = "22222222-2222-4222-8222-222222222222";

    fn config(sources: &[&str]) -> String {
        serde_json::to_string(
            &sources
                .iter()
                .enumerate()
                .map(|(index, source)| format!("policy-{index}={source}"))
                .collect::<Vec<_>>(),
        )
        .unwrap()
    }

    #[test]
    fn silence_and_partial_quorum_block() {
        let mut replies = RequiredReplies::parse(&config(&[FIRST, SECOND])).unwrap();
        replies.accept(FIRST, r#"{"skip":false}"#);
        assert!(!replies.complete());
        assert!(replies.finish().skip);
        assert!(
            RequiredReplies::parse(&config(&[FIRST]))
                .unwrap()
                .finish()
                .skip
        );
    }

    #[test]
    fn quorum_keeps_a_bounded_poll_for_later_context() {
        let mut replies = RequiredReplies::parse(&config(&[FIRST])).unwrap();
        assert_eq!(replies.next_wait(10, 1000, 25), Some(990));
        replies.accept(FIRST, r#"{"skip":false}"#);
        assert_eq!(replies.next_wait(10, 1000, 25), Some(25));
        assert_eq!(replies.next_wait(990, 1000, 25), Some(10));
        assert_eq!(replies.next_wait(1000, 1000, 25), None);
        assert_eq!(replies.next_wait(1001, 1000, 25), None);
        assert!(!replies.finish().skip);
    }

    #[test]
    fn unrelated_source_cannot_satisfy_policy() {
        let mut replies = RequiredReplies::parse(&config(&[FIRST])).unwrap();
        replies.accept(SECOND, r#"{"skip":false}"#);
        assert!(replies.finish().skip);
    }

    #[test]
    fn deny_wins_over_ask_and_duplicate_allow() {
        let mut replies = RequiredReplies::parse(&config(&[FIRST, SECOND])).unwrap();
        replies.accept(FIRST, r#"{"skip":true,"reason":"rule"}"#);
        replies.accept(SECOND, r#"{"skip":false,"ask":true}"#);
        replies.accept(FIRST, r#"{"skip":false}"#);
        assert!(replies.complete());
        assert!(replies.finish().skip);
    }

    #[test]
    fn malformed_required_response_is_not_permission() {
        let mut replies = RequiredReplies::parse(&config(&[FIRST])).unwrap();
        replies.accept(FIRST, r#"{"additional_context":"hello"}"#);
        replies.accept(FIRST, r#"{"skip":false}"#);
        assert!(replies.finish().skip);
        assert!(RequiredReplies::parse("not-a-source").is_err());
        assert!(RequiredReplies::parse(&format!("{FIRST},")).is_err());
    }

    #[test]
    fn no_required_policy_is_no_objection_not_explicit_allow() {
        assert_eq!(
            RequiredReplies::parse("").unwrap().finish(),
            Decision::default()
        );
    }

    #[test]
    fn revised_policy_does_not_require_retired_identity() {
        let config = serde_json::to_string(&[
            format!("codewall-enforcer={FIRST}"),
            format!("codewall-enforcer={SECOND}"),
            format!("codewall-enforcer={SECOND}"),
        ])
        .unwrap();
        let mut replies = RequiredReplies::parse(&config).unwrap();
        replies.accept(FIRST, r#"{"skip":false}"#);
        assert!(!replies.complete());
        replies.accept(SECOND, r#"{"skip":false}"#);
        assert!(!replies.finish().skip);
    }

    #[test]
    fn updating_one_policy_preserves_another_required_policy() {
        let config = serde_json::to_string(&[
            format!("other-policy={FIRST}"),
            format!("codewall-enforcer={FIRST}"),
            format!("codewall-enforcer={SECOND}"),
        ])
        .unwrap();
        let mut replies = RequiredReplies::parse(&config).unwrap();
        replies.accept(SECOND, r#"{"skip":false}"#);
        assert!(replies.finish().skip);
    }

    #[test]
    fn invalid_registration_cannot_silently_remove_policy() {
        for config in [
            r#"["codewall-enforcer="]"#,
            r#"["=11111111-1111-4111-8111-111111111111"]"#,
            r#"{}"#,
        ] {
            assert!(RequiredReplies::parse(config).is_err());
        }
    }

    #[test]
    fn unregister_preserves_other_policy_and_can_be_reinstated() {
        let mut revisions = vec![
            format!("other={FIRST}"),
            format!("codewall-enforcer={SECOND}"),
            "codewall-enforcer=-".into(),
        ];
        let mut replies =
            RequiredReplies::parse(&serde_json::to_string(&revisions).unwrap()).unwrap();
        replies.accept(FIRST, r#"{"skip":false}"#);
        assert!(replies.complete());
        assert!(!replies.finish().skip);
        revisions.push(format!("codewall-enforcer={SECOND}"));
        let mut replies =
            RequiredReplies::parse(&serde_json::to_string(&revisions).unwrap()).unwrap();
        replies.accept(FIRST, r#"{"skip":false}"#);
        assert!(!replies.complete());
        replies.accept(SECOND, r#"{"skip":false}"#);
        assert!(!replies.finish().skip);
    }

    #[test]
    fn registration_history_is_not_limited_to_a_reply_reason() {
        let revisions = vec![format!("codewall-enforcer={FIRST}"); 100];
        let config = serde_json::to_string(&revisions).unwrap();
        assert!(config.len() > 4096);
        assert!(RequiredReplies::parse(&config).is_ok());
        for value in ["", "--", "null"] {
            assert!(
                RequiredReplies::parse(
                    &serde_json::to_string(&[format!("codewall-enforcer={value}")]).unwrap()
                )
                .is_err()
            );
        }
    }

    #[test]
    fn repair_and_removal_work_after_history_exceeds_one_request() {
        let mut revisions = vec![format!("codewall-enforcer={FIRST}"); 22_000];
        revisions.push(format!("other={SECOND}"));
        revisions.push("codewall-enforcer=-".into());
        let config = serde_json::to_string(&revisions).unwrap();
        assert!(config.len() > 1 << 20);
        let mut replies = RequiredReplies::parse(&config).unwrap();
        replies.accept(SECOND, r#"{"skip":false}"#);
        assert!(replies.complete());
        assert!(!replies.finish().skip);

        revisions.push(format!("codewall-enforcer={FIRST}"));
        let mut replies =
            RequiredReplies::parse(&serde_json::to_string(&revisions).unwrap()).unwrap();
        replies.accept(SECOND, r#"{"skip":false}"#);
        assert!(!replies.complete());
        replies.accept(FIRST, r#"{"skip":true,"reason":"blocked"}"#);
        assert!(replies.finish().skip);
    }

    #[test]
    fn explicit_repair_supersedes_only_its_named_invalid_revision() {
        for repair in [
            format!("codewall-enforcer={FIRST}"),
            "codewall-enforcer=-".into(),
        ] {
            let revisions = serde_json::to_string(&["codewall-enforcer=bad", &repair]).unwrap();
            assert!(RequiredReplies::parse(&revisions).is_ok());
            for unrepairable in ["other=bad", "unidentifiable", "=bad"] {
                let revisions = serde_json::to_string(&[unrepairable, &repair]).unwrap();
                assert!(RequiredReplies::parse(&revisions).is_err());
            }
        }
    }
}
