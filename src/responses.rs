//! JSON projection helpers for outbound responses.
//!
//! We emit `serde_json::Value` directly instead of round-tripping
//! through typed structs so the wire shape matches `users.wit`
//! (kebab-case fields) without hand-maintaining a parallel set of
//! Rust structs only the publish path would use.

use serde_json::{Map, Value};

use crate::types::{AstridUser, FrontendLink};

/// Project an [`AstridUser`] into the kebab-case JSON shape from
/// `users.wit`. Returns `None` when the input is `None`, so the
/// `user: option<astrid-user>` field can be passed through unchanged.
#[must_use]
pub fn user_to_json(user: Option<&AstridUser>) -> Option<Value> {
    user.map(|u| {
        let mut obj = Map::new();
        obj.insert("id".into(), Value::String(u.id.to_string()));
        if let Some(name) = &u.display_name {
            obj.insert("display-name".into(), Value::String(name.clone()));
        }
        if let Some(key) = &u.public_key {
            obj.insert(
                "public-key".into(),
                Value::Array(key.iter().map(|b| Value::Number((*b).into())).collect()),
            );
        }
        obj.insert("created-at".into(), Value::String(u.created_at.clone()));
        Value::Object(obj)
    })
}

/// Project a [`FrontendLink`] into the kebab-case JSON shape from
/// `users.wit`.
#[must_use]
pub fn link_to_json(link: &FrontendLink) -> Value {
    serde_json::json!({
        "platform": link.platform,
        "platform-user-id": link.platform_user_id,
        "astrid-user-id": link.astrid_user_id.to_string(),
        "linked-at": link.linked_at,
        "method": link.method,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use uuid::Uuid;

    #[test]
    fn user_to_json_uses_kebab_case() {
        let user = AstridUser {
            id: Uuid::parse_str("00000000-0000-4000-8000-000000000001").unwrap(),
            public_key: None,
            display_name: Some("Alice".into()),
            created_at: "2026-01-15T12:00:00.000Z".into(),
        };
        let json = user_to_json(Some(&user)).unwrap();
        let obj = json.as_object().unwrap();
        assert_eq!(obj["id"], "00000000-0000-4000-8000-000000000001");
        assert_eq!(obj["display-name"], "Alice");
        assert_eq!(obj["created-at"], "2026-01-15T12:00:00.000Z");
        assert!(!obj.contains_key("display_name"));
    }

    #[test]
    fn user_to_json_omits_none_fields() {
        let user = AstridUser::new(None);
        let json = user_to_json(Some(&user)).unwrap();
        let obj = json.as_object().unwrap();
        assert!(!obj.contains_key("display-name"));
        assert!(!obj.contains_key("public-key"));
    }

    #[test]
    fn user_to_json_returns_none_for_absent_user() {
        assert!(user_to_json(None).is_none());
    }

    #[test]
    fn link_to_json_uses_kebab_case() {
        let link = FrontendLink {
            platform: "discord".into(),
            platform_user_id: "12345".into(),
            astrid_user_id: Uuid::parse_str("00000000-0000-4000-8000-000000000002").unwrap(),
            linked_at: "2026-01-15T12:00:00.000Z".into(),
            method: "admin".into(),
        };
        let json = link_to_json(&link);
        let obj = json.as_object().unwrap();
        assert_eq!(obj["platform-user-id"], "12345");
        assert_eq!(
            obj["astrid-user-id"],
            "00000000-0000-4000-8000-000000000002"
        );
        assert_eq!(obj["linked-at"], "2026-01-15T12:00:00.000Z");
    }
}
