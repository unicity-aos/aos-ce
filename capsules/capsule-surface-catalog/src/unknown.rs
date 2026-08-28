//! Safe namespaced-extension boundary and neutral fallback.

use crate::catalog::{CatalogPrimitive, Primitive};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use std::fmt;

const MAX_CHILDREN: usize = 64;
const MAX_IDENTIFIER_BYTES: usize = 128;

/// Untrusted unknown-component document with only inert JSON children.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnknownComponentDocument {
    /// Declared namespaced identifier, preserved only when legal.
    pub identifier: String,
    /// References already accepted by the interaction contract.
    pub accepted_actions: BTreeSet<String>,
    /// Inert child documents; they are never interpreted as executable nodes.
    pub children: Vec<serde_json::Value>,
}

/// Validated unknown component boundary.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnknownComponent {
    /// Preserved identifier.
    pub identifier: String,
    /// Preserved legal action references.
    pub accepted_actions: BTreeSet<String>,
    /// Count of inert children omitted by the fallback.
    pub omitted_children: usize,
}

/// Neutral rendering decision.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct UnknownFallback {
    /// Declared identifier.
    pub identifier: String,
    /// Accepted action references.
    pub accepted_actions: BTreeSet<String>,
    /// Stable neutral rendering marker.
    pub rendering: String,
    /// Unknown children are never executed.
    pub children_executed: bool,
    /// Fallback never creates authority.
    pub authority_minted: bool,
}

/// Unknown-component rejection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum UnknownComponentError {
    /// Identifier was not a legal namespaced extension.
    Identifier,
    /// Identifier was legal but named a known v1 primitive.
    KnownPrimitive,
    /// An action reference was not legally accepted.
    ActionReference,
    /// Child count exceeded the safe bound.
    Children,
}

impl fmt::Display for UnknownComponentError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let message = match self {
            Self::Identifier => "unknown component identifier is invalid",
            Self::KnownPrimitive => "known primitives do not use the unknown fallback",
            Self::ActionReference => "unknown component action reference is invalid",
            Self::Children => "unknown component has too many children",
        };
        formatter.write_str(message)
    }
}

impl std::error::Error for UnknownComponentError {}

/// Validate and reduce an unknown document to a neutral fallback decision.
pub fn validate_unknown_component(
    document: &UnknownComponentDocument,
) -> Result<UnknownFallback, UnknownComponentError> {
    if !valid_identifier(&document.identifier) {
        return Err(UnknownComponentError::Identifier);
    }
    if Primitive::from_id(&document.identifier).is_some() {
        return Err(UnknownComponentError::KnownPrimitive);
    }
    if !document
        .accepted_actions
        .iter()
        .all(|action| valid_action(action))
    {
        return Err(UnknownComponentError::ActionReference);
    }
    if document.children.len() > MAX_CHILDREN {
        return Err(UnknownComponentError::Children);
    }
    Ok(UnknownFallback {
        identifier: document.identifier.clone(),
        accepted_actions: document.accepted_actions.clone(),
        rendering: "neutral-unsupported-component".to_owned(),
        children_executed: false,
        authority_minted: false,
    })
}

fn valid_identifier(value: &str) -> bool {
    if value.len() > MAX_IDENTIFIER_BYTES || value.matches(':').count() != 1 {
        return false;
    }
    let Some((namespace, component)) = value.split_once(':') else {
        return false;
    };
    valid_namespace(namespace) && valid_component(component)
}

fn valid_namespace(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_lowercase())
        && value.chars().all(|character| {
            character.is_ascii_lowercase() || character.is_ascii_digit() || character == '-'
        })
        && !value.ends_with('-')
}

fn valid_component(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 64
        && value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_uppercase())
        && value
            .chars()
            .all(|character| character.is_ascii_alphanumeric())
}

fn valid_action(value: &str) -> bool {
    if value.is_empty()
        || value.len() > 96
        || !value
            .chars()
            .next()
            .is_some_and(|character| character.is_ascii_lowercase())
    {
        return false;
    }
    let legal = value.chars().all(|character| {
        character.is_ascii_lowercase()
            || character.is_ascii_digit()
            || matches!(character, '-' | '.')
    });
    let forbidden = [
        "grant",
        "capability",
        "secret",
        "token",
        "credential",
        "exec",
        "shell",
    ];
    legal && !forbidden.iter().any(|word| value.contains(word))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn document(identifier: &str) -> UnknownComponentDocument {
        UnknownComponentDocument {
            identifier: identifier.to_owned(),
            accepted_actions: ["action.open-details".to_owned()].into_iter().collect(),
            children: vec![serde_json::json!({ "opaque": true })],
        }
    }

    #[test]
    fn preserves_identifier_and_actions_but_omits_children() {
        let fallback = validate_unknown_component(&document("example:UnsupportedWidget"))
            .expect("unknown component is safe");
        assert_eq!(fallback.identifier, "example:UnsupportedWidget");
        assert!(fallback.accepted_actions.contains("action.open-details"));
        assert_eq!(fallback.rendering, "neutral-unsupported-component");
        assert!(!fallback.children_executed);
        assert!(!fallback.authority_minted);
    }

    #[test]
    fn rejects_hostile_identity_actions_and_execution() {
        assert_eq!(
            validate_unknown_component(&document("evil")),
            Err(UnknownComponentError::Identifier)
        );
        let namespaced_known_word = validate_unknown_component(&document("example:Button"))
            .expect("a namespaced extension is not the unnamespaced catalog primitive");
        assert_eq!(namespaced_known_word.identifier, "example:Button");
        let hostile_action = UnknownComponentDocument {
            accepted_actions: ["capability.grant".to_owned()].into_iter().collect(),
            ..document("example:Widget")
        };
        assert_eq!(
            validate_unknown_component(&hostile_action),
            Err(UnknownComponentError::ActionReference)
        );
        let hostile_children = UnknownComponentDocument {
            children: Vec::new(),
            ..document("example:Widget")
        };
        assert!(validate_unknown_component(&hostile_children).is_ok());
    }
}
