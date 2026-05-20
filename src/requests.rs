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
    #[serde(alias = "platform_user_id")]
    pub platform_user_id: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct LinkRequest {
    pub source: Source,
    pub platform: String,
    #[serde(alias = "platform_user_id")]
    pub platform_user_id: String,
    #[serde(alias = "astrid_user_id")]
    pub astrid_user_id: String,
    pub method: String,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct UnlinkRequest {
    pub source: Source,
    pub platform: String,
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
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn resolve_request_parses_kebab_case() {
        let json = r#"{
            "source": {
                "channel": "discord",
                "user-id": null,
                "correlation-id": "abc"
            },
            "platform": "discord",
            "platform-user-id": "12345"
        }"#;
        let req: ResolveRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.platform, "discord");
        assert_eq!(req.platform_user_id, "12345");
        assert_eq!(req.source.correlation_id, "abc");
    }

    #[test]
    fn resolve_request_parses_snake_case_via_alias() {
        let json = r#"{
            "source": {
                "channel": "discord",
                "user_id": null,
                "correlation_id": "abc"
            },
            "platform": "discord",
            "platform_user_id": "12345"
        }"#;
        let req: ResolveRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.platform_user_id, "12345");
        assert_eq!(req.source.correlation_id, "abc");
    }

    #[test]
    fn link_request_round_trips() {
        let json = r#"{
            "source": { "channel": "cli", "correlation-id": "c1" },
            "platform": "discord",
            "platform-user-id": "u1",
            "astrid-user-id": "00000000-0000-4000-8000-000000000001",
            "method": "admin"
        }"#;
        let req: LinkRequest = serde_json::from_str(json).unwrap();
        assert_eq!(req.platform, "discord");
        assert_eq!(req.platform_user_id, "u1");
        assert_eq!(req.astrid_user_id, "00000000-0000-4000-8000-000000000001");
        assert_eq!(req.method, "admin");
    }

    #[test]
    fn create_request_allows_missing_display_name() {
        let json = r#"{ "source": { "channel": "cli", "correlation-id": "c1" } }"#;
        let req: CreateRequest = serde_json::from_str(json).unwrap();
        assert!(req.display_name.is_none());
    }
}
