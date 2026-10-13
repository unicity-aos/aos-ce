//! Correlated, host-attributed failure receipts for the broker's own publication.

use astrid_sdk::ipc::{Message, PrincipalAttribution, Subscription};
use serde_json::Value;

pub(super) const NAMESPACE: &str = "astrid.v1.delivery.failure";

/// The host scopes this alias to the calling publisher and effective principal.
/// Subscribing before publication closes the fast-trap race. Older hosts simply
/// receive no receipts and retain the existing bounded reply timeout.
pub(crate) fn subscribe(delivery_id: &str) -> Result<Subscription, astrid_sdk::SysError> {
    if !is_valid_id(delivery_id) {
        return Err(astrid_sdk::SysError::ApiError(
            "invalid delivery correlation".into(),
        ));
    }
    astrid_sdk::ipc::subscribe(&format!("{NAMESPACE}.{delivery_id}"))
}

pub(crate) fn poll(
    sub: &Subscription,
    delivery_id: &str,
    request_topic: &str,
    principal: &str,
) -> Option<String> {
    sub.poll()
        .ok()?
        .messages
        .iter()
        .find_map(|message| match_failure(message, delivery_id, request_topic, principal))
}

pub(crate) fn is_valid_id(value: &str) -> bool {
    uuid::Uuid::parse_str(value).is_ok_and(|id| !id.is_nil() && id.simple().to_string() == value)
}

fn match_failure(
    message: &Message,
    delivery_id: &str,
    request_topic: &str,
    principal: &str,
) -> Option<String> {
    // Only the host can mint nil-source attribution. The resource subscription
    // additionally binds the publisher; a peer cannot induce our private receipt.
    if message.source_id != "00000000-0000-0000-0000-000000000000"
        || message.principal != PrincipalAttribution::Verified(principal.to_owned())
    {
        return None;
    }
    let suffix = message.topic.strip_prefix(&format!("{NAMESPACE}."))?;
    let (publisher, topic_id) = suffix.split_once('.')?;
    if !is_valid_id(publisher) || topic_id != delivery_id {
        return None;
    }
    let value: Value = serde_json::from_str(&message.payload).ok()?;
    if value.get("delivery_id")?.as_str()? != delivery_id
        || value.get("request_topic")?.as_str()? != request_topic
    {
        return None;
    }
    Some(value.get("reason")?.as_str()?.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_failure_is_correlated_and_guest_forgeries_are_rejected() {
        let id = "1234567890abcdef1234567890abcdef";
        let topic = "tool.v1.execute.example";
        let mut message = Message {
            topic: format!("{NAMESPACE}.2234567890abcdef1234567890abcdef.{id}"),
            payload: serde_json::json!({"delivery_id":id,"request_topic":topic,"reason":"interceptor execution failed"}).to_string(),
            source_id: "00000000-0000-0000-0000-000000000000".into(),
            principal: PrincipalAttribution::Verified("alice".into()),
        };
        assert_eq!(
            match_failure(&message, id, topic, "alice").as_deref(),
            Some("interceptor execution failed")
        );
        assert!(match_failure(&message, "other", topic, "alice").is_none());
        assert!(match_failure(&message, id, "other.topic", "alice").is_none());
        assert!(match_failure(&message, id, topic, "bob").is_none());
        let original_topic = message.topic.clone();
        message.topic = "other.topic".into();
        assert!(match_failure(&message, id, topic, "alice").is_none());
        message.topic = original_topic;
        message.principal = PrincipalAttribution::Claimed("alice".into());
        assert!(match_failure(&message, id, topic, "alice").is_none());
        message.principal = PrincipalAttribution::Verified("alice".into());
        message.source_id = "12345678-90ab-cdef-1234-567890abcdef".into();
        assert!(match_failure(&message, id, topic, "alice").is_none());
    }
}

#[cfg(test)]
mod id_tests {
    use super::*;
    #[test]
    fn invalid_delivery_ids_fail_before_host_subscription() {
        for value in [
            "",
            "../other",
            "00000000000000000000000000000000",
            "12345678-90ab-cdef-1234-567890abcdef",
        ] {
            assert!(!is_valid_id(value));
            assert!(subscribe(value).is_err());
        }
    }
}
