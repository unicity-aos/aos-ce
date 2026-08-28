//! Document validation and extension fallback errors.

use serde::{Deserialize, Deserializer, Serialize};
use std::fmt;

/// An extension is namespaced, bounded, and stored verbatim.
#[derive(Clone, Debug, Default, Eq, PartialEq, Serialize)]
pub struct Extensions(pub Vec<(String, crate::canonical::CanonicalJson)>);

impl<'de> Deserialize<'de> for Extensions {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let values = Vec::<(String, crate::canonical::CanonicalJson)>::deserialize(deserializer)?;
        Self::new(values).map_err(serde::de::Error::custom)
    }
}

impl Extensions {
    /// Store extensions after checking that every key has a declared namespace.
    pub fn new(
        values: Vec<(String, crate::canonical::CanonicalJson)>,
    ) -> Result<Self, ExtensionError> {
        let mut seen = std::collections::BTreeSet::new();
        for (key, value) in &values {
            Self::validate_key(key)?;
            let canonical = value.len_bytes();
            if canonical > MAX_EXTENSION_VALUE_BYTES {
                return Err(ExtensionError::TooLarge(key.to_string()));
            }
            if !seen.insert(key.clone()) {
                return Err(ExtensionError::DuplicateKey(key.to_string()));
            }
        }
        Ok(Self(values.clone()))
    }

    fn validate_key(key: &str) -> Result<(), ExtensionError> {
        let separator = key
            .find(['/', ':'])
            .ok_or(ExtensionError::NotNamespaced(key.to_owned()))?;
        if separator == 0
            || key.len() > MAX_EXTENSION_KEY_BYTES
            || !key
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_' | '/' | ':'))
            || key.contains("..")
        {
            return Err(ExtensionError::InvalidKey(key.to_owned()));
        }
        Ok(())
    }

    pub fn validate(&self) -> Result<(), ExtensionError> {
        Self::new(self.0.clone()).map(|_| ())
    }

    /// Read an extension or return the explicit fallback error.
    pub fn get(&self, key: &str) -> Result<&crate::canonical::CanonicalJson, ExtensionError> {
        Self::validate_key(key)?;
        self.0
            .iter()
            .find(|(stored, _)| stored == key)
            .map(|(_, value)| value)
            .ok_or_else(|| ExtensionError::Unsupported(key.to_owned()))
    }
}

/// Maximum canonical bytes retained for one extension value.
pub const MAX_EXTENSION_VALUE_BYTES: usize = 4096;
/// Maximum UTF-8 bytes in an extension key.
pub const MAX_EXTENSION_KEY_BYTES: usize = 256;

/// Extension validation failures.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExtensionError {
    /// A key has no namespace separator.
    NotNamespaced(String),
    /// A key contains unsupported or traversal-like text.
    InvalidKey(String),
    /// A key was supplied more than once.
    DuplicateKey(String),
    /// A canonical value exceeds the retained-extension bound.
    TooLarge(String),
    /// No adapter understands the namespaced extension.
    Unsupported(String),
}

impl fmt::Display for ExtensionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotNamespaced(key) => write!(f, "extension {key} is not namespaced"),
            Self::InvalidKey(key) => write!(f, "extension key {key} is invalid"),
            Self::DuplicateKey(key) => write!(f, "extension key {key} is duplicated"),
            Self::TooLarge(key) => write!(f, "extension {key} exceeds the value bound"),
            Self::Unsupported(key) => write!(f, "extension {key} requires its declared adapter"),
        }
    }
}

impl std::error::Error for ExtensionError {}

impl From<ExtensionError> for DocumentError {
    fn from(value: ExtensionError) -> Self {
        Self::Extensions(value)
    }
}

/// Generic document parsing and validation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum DocumentError {
    /// JSON was malformed, duplicate-keyed, oversized, or noncanonical.
    Canonical(String),
    /// Document schema was not the requested schema.
    Schema,
    /// Document failed its structural validation.
    Invalid(&'static str),
    /// Namespaced extension failed validation or lacked a fallback adapter.
    Extensions(ExtensionError),
}

impl fmt::Display for DocumentError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Canonical(message) => write!(f, "canonical JSON error: {message}"),
            Self::Schema => f.write_str("unsupported document schema"),
            Self::Invalid(document) => write!(f, "invalid {document} document"),
            Self::Extensions(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for DocumentError {}
