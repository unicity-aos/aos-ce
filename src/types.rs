//! Domain types for the users capsule.
//!
//! The on-disk JSON layout shares the key scheme of the legacy kernel
//! `astrid-storage::identity` store. The value shapes are deliberately
//! closer to the WIT contract than to the kernel's Rust serialization:
//!
//! * `public_key` is a `list<u8>` (matches WIT `option<list<u8>>`); the
//!   kernel encodes the same bytes as a base64 string.
//! * `created_at` / `linked_at` are millisecond-precision RFC 3339
//!   strings; the kernel uses chrono's microsecond default.
//! * `AstridUser` carries no `principal` field — the capsule's per-
//!   principal KV scope already encodes it, so the kernel record's
//!   redundant `principal: PrincipalId` is dropped on first re-write.
//!
//! Pre-launch there are no production records to migrate, so these
//! divergences are deliberate. Any future migration tool transforms
//! kernel records into capsule records (base64-decode public keys,
//! reformat timestamps, strip principal) at cutover time.

use crate::time::now_rfc3339;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Canonical Astrid user identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AstridUser {
    /// UUID v4.
    pub id: Uuid,
    /// Optional ed25519 public key (32 bytes).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub public_key: Option<[u8; 32]>,
    /// Operator/canonical Astrid-side display name. Mutable via
    /// `set_display_name`.
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
///
/// Composite key: `(platform, platform_instance?, platform_user_id)`.
/// Exactly one link per triple; relink upserts.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FrontendLink {
    /// Normalized platform name (lowercased, trimmed).
    pub platform: String,
    /// Optional workspace / homeserver / network scope for federated
    /// and multi-instance platforms (Slack, IRC, XMPP, Mattermost).
    /// `None` for globally-scoped platforms (Discord, Telegram, X)
    /// and for federated platforms whose identifier already embeds
    /// the homeserver (Matrix `@alice:server.org`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_instance: Option<String>,
    /// Platform-specific stable opaque user identifier.
    pub platform_user_id: String,
    /// The Astrid user UUID this link maps to.
    pub astrid_user_id: Uuid,
    /// When this link was created (RFC 3339).
    pub linked_at: String,
    /// How the link was established — audit string.
    pub method: String,
    /// Platform's *global* display name at link time. Distinct from
    /// the canonical `AstridUser.display_name` and from any
    /// per-context override (see [`ContextIdentity`]).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
}

/// Per-context display-name overlay on a [`FrontendLink`].
///
/// One record per `(platform, platform_instance?, platform_user_id,
/// context_id)`. `context_id` is opaque to the capsule — uplinks
/// define per-platform schemes (`"guild:123"`, `"room:!abc:server"`,
/// `"channel:C01"`).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextIdentity {
    pub platform: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub platform_instance: Option<String>,
    pub platform_user_id: String,
    pub context_id: String,
    pub display_name: String,
    pub updated_at: String,
}

/// Multi-tenant request envelope. Mirrors the `source` record in
/// `users.wit`. Sits on every inbound `users.v1.*.request` payload.
///
/// Deserializes both kebab-case (WIT-generated bindings) and
/// snake_case (hand-written JSON) wire formats.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Source {
    /// Originating uplink capsule — `"cli"`, `"sphere"`, `"discord"`,
    /// `"telegram"`, etc. Distinct from `FrontendLink.platform`
    /// (which identifies the external service being linked, not the
    /// capsule making the request).
    #[serde(alias = "uplink", alias = "channel")]
    pub uplink: String,
    /// AstridUserId of the requester when known.
    #[serde(default, skip_serializing_if = "Option::is_none", alias = "user_id")]
    pub user_id: Option<String>,
    /// Correlation token — the requester filters response topics by this.
    #[serde(alias = "correlation_id")]
    pub correlation_id: String,
}

/// Operation error.
#[derive(Debug, PartialEq, Eq)]
pub enum StoreError {
    InvalidInput(String),
    UserNotFound(Uuid),
    LinkNotFound,
    Storage(String),
}

impl std::fmt::Display for StoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(s) => write!(f, "invalid input: {s}"),
            Self::UserNotFound(id) => write!(f, "user not found: {id}"),
            Self::LinkNotFound => write!(f, "link not found"),
            Self::Storage(s) => write!(f, "storage error: {s}"),
        }
    }
}

/// Normalize a platform name: trim whitespace, lowercase.
#[must_use]
pub fn normalize_platform(name: &str) -> String {
    name.trim().to_ascii_lowercase()
}

/// Result of the layered display-name resolution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedDisplayName {
    pub name: String,
    /// One of `"context"`, `"link"`, `"canonical"`.
    pub source: &'static str,
}
