//! Display metadata is advisory. Response values and broker targets are unchanged.
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{InteractionError, MAX_MESSAGE_BYTES, OptionValue};

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Kind {
    CapsuleAccess,
    ActionApproval,
    Ingress,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
enum Lifetime {
    None,
    Session,
    UntilRuntimeRestart,
    Durable,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(super) struct ConsentPresentation {
    version: u32,
    kind: Kind,
    #[serde(skip_serializing_if = "Option::is_none")]
    action: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    resource: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reason: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    principal: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    capsule: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool: Option<String>,
    lifetimes: Vec<Lifetime>,
}

/// Bind the explicit extension to the form values, never to guessed field names.
pub(super) fn parse(
    params: &Map<String, Value>,
    field: &str,
    options: &[OptionValue],
) -> Result<Option<ConsentPresentation>, InteractionError> {
    let Some(raw) = params
        .get("_meta")
        .and_then(|m| m.get("org.astrid/consent"))
    else {
        return Ok(None);
    };
    // A future extension is not interpreted as version 1.
    if raw
        .get("version")
        .and_then(Value::as_u64)
        .is_some_and(|v| v != 1)
    {
        return Ok(None);
    }
    let invalid = || InteractionError::Invalid("invalid Astrid consent display metadata");
    let mut object = raw.as_object().cloned().ok_or_else(invalid)?;
    let choices = object.remove("choices").ok_or_else(invalid)?;
    let choices = choices.as_array().ok_or_else(invalid)?;
    if choices.len() != options.len() {
        return Err(invalid());
    }
    let mut lifetimes = Vec::with_capacity(options.len());
    for option in options {
        let matching: Vec<_> = choices
            .iter()
            .filter(|c| c.get("value") == Some(&option.value))
            .collect();
        let [choice] = matching.as_slice() else {
            return Err(invalid());
        };
        let lifetime = choice.get("lifetime").cloned().ok_or_else(invalid)?;
        if !option.affirmative && lifetime != Value::String("none".to_owned()) {
            return Err(invalid());
        }
        lifetimes.push(lifetime);
    }
    object.insert("lifetimes".to_owned(), Value::Array(lifetimes));
    let display: ConsentPresentation =
        serde_json::from_value(Value::Object(object)).map_err(|_| invalid())?;
    if display.version != 1
        || [
            &display.action,
            &display.resource,
            &display.reason,
            &display.principal,
            &display.capsule,
            &display.tool,
        ]
        .into_iter()
        .flatten()
        .any(|text| text.is_empty() || text.len() > MAX_MESSAGE_BYTES)
        || !kind_matches_field(&display.kind, field)
    {
        return Err(invalid());
    }
    Ok(Some(display))
}

fn kind_matches_field(kind: &Kind, field: &str) -> bool {
    matches!(
        (kind, field),
        (Kind::ActionApproval, "choice")
            | (Kind::CapsuleAccess, "grant")
            | (Kind::Ingress, "allow")
    )
}

/// Rewrite only v1 `until_runtime_restart` affirmatives. Tokens stay unchanged.
pub(super) fn apply_runtime_restart_labels(
    options: &mut [OptionValue],
    consent: &ConsentPresentation,
) {
    for (option, lifetime) in options.iter_mut().zip(&consent.lifetimes) {
        if option.affirmative && *lifetime == Lifetime::UntilRuntimeRestart {
            option.label = "Until runtime restart".to_owned();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn request() -> Value {
        json!({"params":{"message":"Approve?","requestedSchema":{
            "type":"object","properties":{"grant":{"type":"boolean"}}
        }}})
    }

    fn action_request(meta: Option<Value>) -> Value {
        let mut value = json!({"params":{"message":"Approve?","requestedSchema":{
            "type":"object","properties":{"choice":{"type":"string",
                "enum":["approve_once","approve_session","approve_always","deny"]}}
        }}});
        if let Some(meta) = meta {
            value["params"]["_meta"] = meta;
        }
        value
    }

    fn action_meta(choices: Value) -> Value {
        json!({"org.astrid/consent":{"version":1,"kind":"action_approval","choices":choices}})
    }

    fn canonical_action_choices() -> Value {
        json!([
            {"value":"approve_once","lifetime":"none"},
            {"value":"approve_session","lifetime":"session"},
            {"value":"approve_always","lifetime":"until_runtime_restart"},
            {"value":"deny","lifetime":"none"}
        ])
    }

    #[test]
    fn generic_field_name_does_not_invent_consent() {
        assert!(
            super::super::parse_request(&request())
                .unwrap()
                .consent
                .is_none()
        );
    }

    #[test]
    fn lifetimes_bind_to_values_not_metadata_order() {
        let mut value = request();
        value["params"]["_meta"] = json!({"org.astrid/consent":{
            "version":1,"kind":"capsule_access","capsule":"notes",
            "choices":[{"value":false,"lifetime":"none"},{"value":true,"lifetime":"durable"}]
        }});
        let parsed = super::super::parse_request(&value).unwrap();
        assert_eq!(
            parsed.consent.unwrap().lifetimes,
            [Lifetime::Durable, Lifetime::None]
        );
        assert_eq!(parsed.options[0].value, true);
        value["params"]["_meta"]["org.astrid/consent"]["choices"][0]["value"] = json!(true);
        assert!(super::super::parse_request(&value).is_err());
    }

    #[test]
    fn action_approval_rewrites_always_label_to_runtime_restart() {
        let parsed = super::super::parse_request(&action_request(Some(action_meta(
            canonical_action_choices(),
        ))))
        .unwrap();
        let always = parsed
            .options
            .iter()
            .find(|option| option.value == "approve_always")
            .unwrap();
        assert_eq!(always.label, "Until runtime restart");
        assert_eq!(always.value, "approve_always");
        assert_eq!(
            parsed
                .options
                .iter()
                .find(|option| option.value == "deny")
                .unwrap()
                .label,
            "Deny"
        );
        assert_eq!(
            parsed
                .options
                .iter()
                .find(|option| option.value == "approve_session")
                .unwrap()
                .label,
            "Approve for Session"
        );
    }

    #[test]
    fn absent_metadata_keeps_generic_always_approve_label() {
        let parsed = super::super::parse_request(&action_request(None)).unwrap();
        assert!(parsed.consent.is_none());
        assert_eq!(
            parsed
                .options
                .iter()
                .find(|option| option.value == "approve_always")
                .unwrap()
                .label,
            "Always Approve"
        );
    }

    #[test]
    fn explicit_kind_must_match_form_field() {
        let mut grant = request();
        grant["params"]["_meta"] = json!({"org.astrid/consent":{
            "version":1,"kind":"ingress",
            "choices":[{"value":true,"lifetime":"session"},{"value":false,"lifetime":"none"}]
        }});
        assert!(matches!(
            super::super::parse_request(&grant),
            Err(InteractionError::Invalid(_))
        ));

        let mut allow = json!({"params":{"message":"Approve?","requestedSchema":{
            "type":"object","properties":{"allow":{"type":"boolean"}}
        }}});
        allow["params"]["_meta"] = json!({"org.astrid/consent":{
            "version":1,"kind":"capsule_access",
            "choices":[{"value":true,"lifetime":"durable"},{"value":false,"lifetime":"none"}]
        }});
        assert!(matches!(
            super::super::parse_request(&allow),
            Err(InteractionError::Invalid(_))
        ));
    }

    #[test]
    fn reordered_choices_relabel_only_matching_affirmative() {
        let parsed = super::super::parse_request(&action_request(Some(action_meta(json!([
            {"value":"deny","lifetime":"none"},
            {"value":"approve_always","lifetime":"until_runtime_restart"},
            {"value":"approve_once","lifetime":"none"},
            {"value":"approve_session","lifetime":"session"}
        ])))))
        .unwrap();
        assert_eq!(
            parsed
                .options
                .iter()
                .map(|option| option.label.as_str())
                .collect::<Vec<_>>(),
            [
                "Approve Once",
                "Approve for Session",
                "Until runtime restart",
                "Deny"
            ]
        );
        assert_eq!(
            parsed.consent.unwrap().lifetimes,
            [
                Lifetime::None,
                Lifetime::Session,
                Lifetime::UntilRuntimeRestart,
                Lifetime::None
            ]
        );
        assert_eq!(parsed.options[2].value, "approve_always");
    }
}
