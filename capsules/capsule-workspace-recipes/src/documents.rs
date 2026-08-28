use capsule_surface_model::canonical::{canonical_bytes, digest_parts};
use capsule_surface_model::recipe::Recipe;
use capsule_surface_model::store::Patch;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub(crate) struct RevisionEnvelope {
    pub recipe: Recipe,
    pub patch: Option<Patch>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct CurrentPointer {
    pub revision: u64,
    pub recipe_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct ActivityPointer {
    pub activity_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub(crate) struct JournalRecord {
    pub patch_id: String,
    pub recipe_id: String,
    pub revision: u64,
    pub recipe_digest: String,
    pub intent_digest: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConflictRecord {
    pub current_revision: u64,
    pub current_recipe_digest: String,
    pub incoming_patch_id: String,
    pub incoming_base_revision: u64,
    pub incoming_base_digest: String,
}

pub(crate) fn canonical<T: Serialize>(value: &T) -> Result<Vec<u8>, serde_json::Error> {
    canonical_bytes(value)
}

pub(crate) fn digest<T: Serialize>(value: &T) -> Result<String, serde_json::Error> {
    digest_parts(value)
}

pub(crate) fn parse_canonical<T: serde::de::DeserializeOwned>(
    bytes: &[u8],
) -> Result<T, serde_json::Error> {
    capsule_surface_model::canonical::parse_canonical(bytes)
}
