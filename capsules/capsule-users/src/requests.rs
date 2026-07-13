//! Inbound IPC request payload shapes.
//!
//! Each struct sets `rename_all = "kebab-case"` to match `users.wit`
//! and individually `alias`es the snake_case form so callers from
//! either side of the bus (WIT-generated kebab vs hand-written
//! snake_case Rust JSON) deserialize cleanly without the publisher
//! needing to know which convention this capsule speaks.

use serde::Deserialize;

use crate::types::Source;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ResolveRequest {
    pub source: Source,
    pub platform: String,
    #[serde(default, alias = "platform_instance")]
    pub platform_instance: Option<String>,
    #[serde(alias = "platform_user_id")]
    pub platform_user_id: String,
    #[serde(default, alias = "context_id")]
    pub context_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct LinkRequest {
    pub source: Source,
    pub platform: String,
    #[serde(default, alias = "platform_instance")]
    pub platform_instance: Option<String>,
    #[serde(alias = "platform_user_id")]
    pub platform_user_id: String,
    #[serde(alias = "astrid_user_id")]
    pub astrid_user_id: String,
    pub method: String,
    #[serde(default, alias = "display_name")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct UnlinkRequest {
    pub source: Source,
    pub platform: String,
    #[serde(default, alias = "platform_instance")]
    pub platform_instance: Option<String>,
    #[serde(alias = "platform_user_id")]
    pub platform_user_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct CreateRequest {
    pub source: Source,
    #[serde(default, alias = "display_name")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SetDisplayNameRequest {
    pub source: Source,
    #[serde(alias = "astrid_user_id")]
    pub astrid_user_id: String,
    #[serde(default, alias = "display_name")]
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct SetPublicKeyRequest {
    pub source: Source,
    #[serde(alias = "astrid_user_id")]
    pub astrid_user_id: String,
    /// Raw bytes (`option<list<u8>>` in WIT). `None` clears the key.
    #[serde(default, alias = "public_key")]
    pub public_key: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct LinksRequest {
    pub source: Source,
    #[serde(alias = "astrid_user_id")]
    pub astrid_user_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct GetRequest {
    pub source: Source,
    #[serde(alias = "astrid_user_id")]
    pub astrid_user_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct DeleteRequest {
    pub source: Source,
    #[serde(alias = "astrid_user_id")]
    pub astrid_user_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ListRequest {
    pub source: Source,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ContextSetRequest {
    pub source: Source,
    pub platform: String,
    #[serde(default, alias = "platform_instance")]
    pub platform_instance: Option<String>,
    #[serde(alias = "platform_user_id")]
    pub platform_user_id: String,
    #[serde(alias = "context_id")]
    pub context_id: String,
    #[serde(alias = "display_name")]
    pub display_name: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ContextClearRequest {
    pub source: Source,
    pub platform: String,
    #[serde(default, alias = "platform_instance")]
    pub platform_instance: Option<String>,
    #[serde(alias = "platform_user_id")]
    pub platform_user_id: String,
    #[serde(alias = "context_id")]
    pub context_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ContextGetRequest {
    pub source: Source,
    pub platform: String,
    #[serde(default, alias = "platform_instance")]
    pub platform_instance: Option<String>,
    #[serde(alias = "platform_user_id")]
    pub platform_user_id: String,
    #[serde(alias = "context_id")]
    pub context_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ContextListForUserRequest {
    pub source: Source,
    #[serde(alias = "astrid_user_id")]
    pub astrid_user_id: String,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct ContextListInContextRequest {
    pub source: Source,
    pub platform: String,
    #[serde(default, alias = "platform_instance")]
    pub platform_instance: Option<String>,
    #[serde(alias = "context_id")]
    pub context_id: String,
    #[serde(default)]
    pub cursor: Option<String>,
    #[serde(default)]
    pub limit: Option<u32>,
}
