//! Duplicate-safe canonical JSON and deterministic BLAKE3 helpers.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use serde_json::Number;
use std::collections::BTreeMap;
use std::fmt;

/// Maximum bytes accepted by [`parse_canonical`].
pub const MAX_CANONICAL_DOCUMENT_BYTES: usize = 1 << 20;

/// A JSON value whose object representation rejects duplicate keys and sorts them.
#[derive(Clone, Debug, PartialEq)]
pub enum CanonicalJson {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Array(Vec<Self>),
    Object(BTreeMap<String, Self>),
}

impl Eq for CanonicalJson {}

impl CanonicalJson {
    /// Canonical encoded size, used for bounded extension storage.
    pub fn len_bytes(&self) -> usize {
        canonical_bytes(self).map_or(usize::MAX, |bytes| bytes.len())
    }
}

impl Serialize for CanonicalJson {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        match self {
            Self::Null => serializer.serialize_none(),
            Self::Bool(value) => serializer.serialize_bool(*value),
            Self::Number(value) => value.serialize(serializer),
            Self::String(value) => serializer.serialize_str(value),
            Self::Array(values) => values.serialize(serializer),
            Self::Object(values) => serializer.collect_map(values.iter()),
        }
    }
}

impl<'de> Deserialize<'de> for CanonicalJson {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_any(CanonicalJsonVisitor)
    }
}

struct CanonicalJsonVisitor;

impl<'de> de::Visitor<'de> for CanonicalJsonVisitor {
    type Value = CanonicalJson;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a finite JSON value")
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(CanonicalJson::Null)
    }

    fn visit_none<E>(self) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(CanonicalJson::Null)
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: Deserializer<'de>,
    {
        CanonicalJson::deserialize(deserializer)
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(CanonicalJson::Bool(value))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(CanonicalJson::Number(value.into()))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(CanonicalJson::Number(value.into()))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Number::from_f64(value)
            .map(CanonicalJson::Number)
            .ok_or_else(|| de::Error::custom("JSON number must be finite"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(CanonicalJson::String(value.to_owned()))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Ok(CanonicalJson::String(value))
    }

    fn visit_seq<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: de::SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = access.next_element::<CanonicalJson>()? {
            values.push(value);
        }
        Ok(CanonicalJson::Array(values))
    }

    fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
    where
        A: de::MapAccess<'de>,
    {
        let mut values = BTreeMap::new();
        while let Some((key, value)) = access.next_entry::<String, CanonicalJson>()? {
            if values.insert(key.clone(), value).is_some() {
                return Err(de::Error::custom(format!("duplicate JSON key `{key}`")));
            }
        }
        Ok(CanonicalJson::Object(values))
    }
}

/// Serialize to the deterministic JSON form used by all content digests.
pub fn canonical_bytes<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    let raw = serde_json::to_vec(value)?;
    let parsed = serde_json::from_slice::<CanonicalJson>(&raw).map_err(|error| {
        serde_json::Error::io(std::io::Error::new(std::io::ErrorKind::InvalidData, error))
    })?;
    serde_json::to_vec(&parsed)
}

/// Serialize to deterministic canonical JSON text.
pub fn canonical_string<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    canonical_bytes(value).map(|bytes| String::from_utf8(bytes).expect("JSON is UTF-8"))
}

/// Parse once for duplicate keys, canonicalize, then deserialize the typed value.
pub fn parse_canonical<T: serde::de::DeserializeOwned>(
    input: &[u8],
) -> Result<T, serde_json::Error> {
    if input.len() > MAX_CANONICAL_DOCUMENT_BYTES {
        return Err(serde_json::Error::io(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "canonical document exceeds bound",
        )));
    }
    let parsed = serde_json::from_slice::<CanonicalJson>(input)?;
    let canonical = serde_json::to_vec(&parsed)?;
    serde_json::from_slice(&canonical)
}

/// Canonical digest for one or more ordered semantic values.
pub fn digest_parts<T: Serialize>(parts: &T) -> Result<String, serde_json::Error> {
    Ok(blake3::hash(&canonical_bytes(parts)?).to_hex().to_string())
}

/// Validate a BLAKE3 digest in its lowercase hexadecimal form.
pub fn valid_blake3_digest(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    #[test]
    fn duplicate_keys_are_rejected_and_objects_are_stable() {
        let reversed = br#"{"z":1,"a":2}"#;
        let forward = br#"{"a":2,"z":1}"#;
        let reversed = parse_canonical::<CanonicalJson>(reversed).expect("reverse input");
        let forward = parse_canonical::<CanonicalJson>(forward).expect("forward input");
        assert_eq!(reversed, forward);
        assert_eq!(
            canonical_string(&forward).expect("canonical"),
            r#"{"a":2,"z":1}"#
        );
        assert!(parse_canonical::<CanonicalJson>(br#"{"a":1,"a":2}"#).is_err());
    }

    #[test]
    fn nonfinite_numbers_are_rejected() {
        assert!(Number::from_f64(f64::NAN).is_none());
        assert!(canonical_string(&CanonicalJson::Object(BTreeMap::new())).is_ok());
    }
}
