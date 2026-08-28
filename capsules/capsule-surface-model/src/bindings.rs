//! Binding references. Bindings describe targets; they never grant authority.

use crate::canonical::{CanonicalJson, valid_blake3_digest};
use crate::error::{DocumentError, ExtensionError, Extensions};
use serde::{Deserialize, Serialize};

fn bounded(value: &str, max: usize) -> bool {
    !value.is_empty() && value.len() <= max
}

fn valid_identifier(value: &str, max: usize) -> bool {
    bounded(value, max)
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':' | '*'))
        && !value.contains("..")
}

fn valid_interface(value: &str) -> bool {
    value.contains(':') && valid_identifier(value, 256)
}

fn valid_mime(value: &str) -> bool {
    let (kind, parameters) = value.split_once(';').unwrap_or((value, ""));
    if !bounded(value, 128)
        || parameters.contains(char::is_control)
        || parameters.contains("path=")
        || parameters.contains("home=")
    {
        return false;
    }
    let mime = kind.trim();
    let Some((type_name, subtype)) = mime.split_once('/') else {
        return false;
    };
    !type_name.is_empty()
        && !subtype.is_empty()
        && value.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '/' | ';' | '=' | '-' | '+' | '.' | '_' | ' ')
        })
}

/// Requested route to a capsule contract.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CapsuleBinding {
    pub capsule: String,
    pub interface: String,
    pub contract_version: String,
    pub requested_route: String,
}

impl CapsuleBinding {
    fn validate(&self) -> Result<(), DocumentError> {
        if valid_identifier(&self.capsule, 128)
            && valid_interface(&self.interface)
            && valid_identifier(&self.contract_version, 32)
            && valid_identifier(&self.requested_route, 256)
        {
            Ok(())
        } else {
            Err(DocumentError::Invalid("capsule binding"))
        }
    }
}

/// Durable descriptor for a native portal. No process identity or command line.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NativeBinding {
    pub app_identity: String,
    pub portal_contract: String,
    pub descriptor_schema: String,
    pub descriptor_id: String,
    pub restore_policy: RestorePolicy,
}

impl NativeBinding {
    fn validate(&self) -> Result<(), DocumentError> {
        if valid_identifier(&self.app_identity, 256)
            && valid_interface(&self.portal_contract)
            && valid_identifier(&self.descriptor_schema, 256)
            && valid_identifier(&self.descriptor_id, 256)
        {
            Ok(())
        } else {
            Err(DocumentError::Invalid("native binding"))
        }
    }
}

/// Restore behavior requested for a future incarnation.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RestorePolicy {
    RestoreRecipe,
    CloseOnRestart,
    PreserveFocusOnly,
}

/// Durable reference to a kernel-owned object.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct DataBinding {
    pub owner: crate::activity::OpaqueOwnerRef,
    pub kernel_object_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub grant_id: Option<String>,
    pub content_hash: String,
    pub mime: String,
    pub freshness: FreshnessPolicy,
    pub cache: CachePolicy,
}

impl DataBinding {
    fn validate(&self) -> Result<(), DocumentError> {
        if !valid_identifier(&self.kernel_object_id, 256) {
            return Err(DocumentError::Invalid("data object id"));
        }
        if !self.owner.is_valid() {
            return Err(DocumentError::Invalid("data owner"));
        }
        if self
            .grant_id
            .as_ref()
            .is_none_or(|id| valid_identifier(id, 256))
        {
        } else {
            return Err(DocumentError::Invalid("data grant id"));
        }
        if !valid_blake3_digest(&self.content_hash) || !valid_mime(&self.mime) {
            return Err(DocumentError::Invalid("data content reference"));
        }
        Ok(())
    }
}

/// Freshness requested from the data boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum FreshnessPolicy {
    Always,
    Immutable,
    OlderThanMs(u64),
}

/// Cache scope requested from the data boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum CachePolicy {
    NoStore,
    Private,
    Shared,
}

/// A target binding. Presence in a document is descriptive, not a grant.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum Binding {
    Capsule(CapsuleBinding),
    Native(NativeBinding),
    Data(DataBinding),
}

impl Binding {
    pub fn validate(&self) -> Result<(), DocumentError> {
        match self {
            Self::Capsule(value) => value.validate(),
            Self::Native(value) => value.validate(),
            Self::Data(value) => value.validate(),
        }
    }
}

/// Named Rhai profile or a request stricter than a named ceiling. Never code.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "kebab-case")]
pub enum RhaiReference {
    NamedProfile {
        profile: String,
    },
    StricterThanCeiling {
        ceiling_profile: String,
        requested_profile: String,
    },
}

impl RhaiReference {
    pub fn validate(&self) -> Result<(), DocumentError> {
        let valid = match self {
            Self::NamedProfile { profile } => valid_identifier(profile, 128),
            Self::StricterThanCeiling {
                ceiling_profile,
                requested_profile,
            } => {
                valid_identifier(ceiling_profile, 128)
                    && valid_identifier(requested_profile, 128)
                    && ceiling_profile != requested_profile
            }
        };
        if valid {
            Ok(())
        } else {
            Err(DocumentError::Invalid("Rhai reference"))
        }
    }
}

/// Validated binding and reference block shared by durable documents.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize, Deserialize)]
pub struct BindingSet {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub binding: Option<Binding>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rhai: Option<RhaiReference>,
    #[serde(default, skip_serializing_if = "Extensions::is_empty")]
    pub extensions: Extensions,
}

impl Extensions {
    fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl BindingSet {
    pub fn validate(&self) -> Result<(), DocumentError> {
        self.binding.as_ref().map_or(Ok(()), Binding::validate)?;
        self.rhai.as_ref().map_or(Ok(()), RhaiReference::validate)?;
        Ok(())
    }

    pub fn extension(&self, key: &str) -> Result<&CanonicalJson, ExtensionError> {
        self.extensions.get(key)
    }
}
