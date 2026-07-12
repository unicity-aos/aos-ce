//! JSON projection helpers for outbound responses.
//!
//! Emit `serde_json::Value` directly so the wire shape matches
//! `users.wit` (kebab-case fields) without maintaining a parallel
//! set of Rust structs only the publish path would use.

use serde_json::{Map, Value};

use crate::types::{AstridUser, ContextIdentity, FrontendLink};

/// Project an [`AstridUser`] into kebab-case JSON.
#[must_use]
pub fn user_value(u: &AstridUser) -> Value {
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
}

#[must_use]
pub fn user_to_json(user: Option<&AstridUser>) -> Option<Value> {
    user.map(user_value)
}

/// Project a [`FrontendLink`] into kebab-case JSON.
#[must_use]
pub fn link_to_json(link: &FrontendLink) -> Value {
    let mut obj = Map::new();
    obj.insert("platform".into(), Value::String(link.platform.clone()));
    if let Some(inst) = &link.platform_instance {
        obj.insert("platform-instance".into(), Value::String(inst.clone()));
    }
    obj.insert(
        "platform-user-id".into(),
        Value::String(link.platform_user_id.clone()),
    );
    obj.insert(
        "astrid-user-id".into(),
        Value::String(link.astrid_user_id.to_string()),
    );
    obj.insert("linked-at".into(), Value::String(link.linked_at.clone()));
    obj.insert("method".into(), Value::String(link.method.clone()));
    if let Some(dn) = &link.display_name {
        obj.insert("display-name".into(), Value::String(dn.clone()));
    }
    Value::Object(obj)
}

/// Project a [`ContextIdentity`] into kebab-case JSON.
#[must_use]
pub fn context_to_json(c: &ContextIdentity) -> Value {
    let mut obj = Map::new();
    obj.insert("platform".into(), Value::String(c.platform.clone()));
    if let Some(inst) = &c.platform_instance {
        obj.insert("platform-instance".into(), Value::String(inst.clone()));
    }
    obj.insert(
        "platform-user-id".into(),
        Value::String(c.platform_user_id.clone()),
    );
    obj.insert("context-id".into(), Value::String(c.context_id.clone()));
    obj.insert("display-name".into(), Value::String(c.display_name.clone()));
    obj.insert("updated-at".into(), Value::String(c.updated_at.clone()));
    Value::Object(obj)
}
