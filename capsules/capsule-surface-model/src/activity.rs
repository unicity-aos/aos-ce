//! Opaque owner and actor identity references.

use crate::recipe::MAX_ID_BYTES;
use serde::{Deserialize, Serialize};

/// Opaque durable owner. Payload identity is never authority by itself.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "kebab-case")]
pub enum OpaqueOwnerRef {
    User(String),
    Principal(String),
    Fleet(String),
}

impl OpaqueOwnerRef {
    pub fn id(&self) -> &str {
        match self {
            Self::User(id) | Self::Principal(id) | Self::Fleet(id) => id,
        }
    }

    pub fn is_valid(&self) -> bool {
        !self.id().is_empty()
            && self.id().len() <= MAX_ID_BYTES
            && !self.id().contains("..")
            && self
                .id()
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'))
    }
}

impl<'de> Deserialize<'de> for OpaqueOwnerRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "kind", content = "id", rename_all = "kebab-case")]
        enum Raw {
            User(String),
            Principal(String),
            Fleet(String),
        }
        let valid = |value: &str| !value.is_empty() && value.len() <= MAX_ID_BYTES;
        match Raw::deserialize(deserializer)? {
            Raw::User(value) if valid(&value) => Ok(Self::User(value)),
            Raw::Principal(value) if valid(&value) => Ok(Self::Principal(value)),
            Raw::Fleet(value) if valid(&value) => Ok(Self::Fleet(value)),
            _ => Err(serde::de::Error::custom("invalid opaque owner reference")),
        }
    }
}

/// Opaque kernel-stamped actor reference as represented in an API document.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(tag = "kind", content = "id", rename_all = "kebab-case")]
pub enum OpaquePrincipalRef {
    User(String),
    Agent(String),
    Service(String),
}

impl OpaquePrincipalRef {
    pub fn id(&self) -> &str {
        match self {
            Self::User(id) | Self::Agent(id) | Self::Service(id) => id,
        }
    }

    pub fn is_valid(&self) -> bool {
        !self.id().is_empty()
            && self.id().len() <= MAX_ID_BYTES
            && !self.id().contains("..")
            && self
                .id()
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | ':'))
    }

    pub fn validate(&self) -> Result<(), crate::store::PatchError> {
        if !self.is_valid() {
            Err(crate::store::PatchError::InvalidPrincipal)
        } else {
            Ok(())
        }
    }
}

impl<'de> Deserialize<'de> for OpaquePrincipalRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "kind", content = "id", rename_all = "kebab-case")]
        enum Raw {
            User(String),
            Agent(String),
            Service(String),
        }
        let valid = |value: &str| !value.is_empty() && value.len() <= MAX_ID_BYTES;
        match Raw::deserialize(deserializer)? {
            Raw::User(value) if valid(&value) => Ok(Self::User(value)),
            Raw::Agent(value) if valid(&value) => Ok(Self::Agent(value)),
            Raw::Service(value) if valid(&value) => Ok(Self::Service(value)),
            _ => Err(serde::de::Error::custom("invalid opaque principal")),
        }
    }
}
