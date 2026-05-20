//! Domain types for the users capsule.
//!
//! The on-disk JSON layout matches the legacy kernel
//! `astrid-storage::identity` store (`user/{uuid}`, `link/{platform}/{id}`,
//! `name/{name}`) so a future kernel-side cutover can read existing
//! records without migration.

use crate::time::now_rfc3339;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Canonical Astrid user record stored in the capsule's KV.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AstridUser {
    /// UUID v4.
    pub id: Uuid,
    /// Optional ed25519 public key (32 bytes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<[u8; 32]>,
    /// Optional human-readable display name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Creation timestamp, RFC 3339.
    pub created_at: String,
}

impl AstridUser {
    /// Build a fresh user record with a random UUID and the current
    /// host wallclock.
    #[must_use]
    pub fn new(display_name: Option<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            public_key: None,
            display_name,
            created_at: now_rfc3339(),
        }
    }
}

/// A platform identity linked to an [`AstridUser`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontendLink {
    /// Normalized platform name (lowercased, trimmed).
    pub platform: String,
    /// Platform-specific user identifier.
    pub platform_user_id: String,
    /// The Astrid user UUID this link maps to.
    pub astrid_user_id: Uuid,
    /// When this link was created (RFC 3339).
    pub linked_at: String,
    /// How the link was established — audit string.
    pub method: String,
}

/// Multi-tenant request envelope. Mirrors the `source` record in
/// `users.wit`. Sits on every inbound `users.v1.*.request` payload.
///
/// Deserializes both kebab-case (WIT-generated bindings) and snake_case
/// (hand-written JSON) wire formats — callers from either side of the
/// bus just work.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Source {
    /// Originating channel — `"cli"`, `"sphere"`, `"discord"`, etc.
    pub channel: String,
    /// AstridUserId of the requester when known.
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "user_id")]
    pub user_id: Option<String>,
    /// Correlation token — the requester filters response topics by this.
    #[serde(alias = "correlation_id")]
    pub correlation_id: String,
}

/// Operation error surfaced through the `error` field on each response.
#[derive(Debug, PartialEq, Eq)]
pub enum StoreError {
    /// A required field was empty or malformed.
    InvalidInput(String),
    /// The target user UUID does not exist.
    UserNotFound(Uuid),
    /// Underlying KV operation failed.
    Storage(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(s) => write!(f, "invalid input: {s}"),
            Self::UserNotFound(id) => write!(f, "user not found: {id}"),
            Self::Storage(s) => write!(f, "storage error: {s}"),
        }
    }
}

/// Normalize a platform name: trim whitespace, lowercase. Mirrors the
/// kernel's `normalize_platform` so the resolve semantics carry over
/// after the cutover.
#[must_use]
pub fn normalize_platform(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}
