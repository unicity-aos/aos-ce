//! Durable activity and recipe restore-truth documents.

use crate::activity::OpaqueOwnerRef;
use crate::bindings::BindingSet;
use crate::canonical::digest_parts;
use crate::components::{MAX_SEMANTIC_TEXT_BYTES, PropValue, SceneError, SemanticNode};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

pub const ACTIVITY_SCHEMA: &str = "aos.activity@1";
pub const RECIPE_SCHEMA: &str = "aos.recipe@1";
pub const SURFACE_SCHEMA: &str = "aos.surface@1";
pub const PATCH_SCHEMA: &str = "aos.patch@1";
pub const MAX_ID_BYTES: usize = 256;
pub const MAX_RECIPE_METADATA: usize = 16;
pub const MAX_RECIPE_METADATA_BYTES: usize = 2048;

fn bounded_nonempty(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum
}

pub(crate) fn value_text_len(value: &PropValue) -> usize {
    value.byte_len()
}

/// Durable identity and owner reference for an activity.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "ActivityInput")]
pub struct Activity {
    pub schema: String,
    pub activity_id: String,
    pub owner_ref: OpaqueOwnerRef,
    pub title: String,
    pub recipe_id: String,
    #[serde(flatten)]
    pub bindings: BindingSet,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub current_surface: Option<SurfacePointer>,
}

/// Non-authoritative pointer to an ephemeral surface.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SurfacePointer {
    pub surface_id: String,
    pub incarnation: u64,
}

impl Activity {
    pub fn new(
        activity_id: impl Into<String>,
        owner_ref: OpaqueOwnerRef,
        title: impl Into<String>,
        recipe_id: impl Into<String>,
    ) -> Result<Self, RecipeValidationError> {
        let value = Self {
            schema: ACTIVITY_SCHEMA.to_owned(),
            activity_id: activity_id.into(),
            owner_ref,
            title: title.into(),
            recipe_id: recipe_id.into(),
            bindings: BindingSet::default(),
            current_surface: None,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), RecipeValidationError> {
        if self.schema != ACTIVITY_SCHEMA
            || !bounded_nonempty(&self.activity_id, MAX_ID_BYTES)
            || self.owner_ref.id().len() > MAX_ID_BYTES
            || self.owner_ref.id().is_empty()
            || !bounded_nonempty(&self.title, MAX_SEMANTIC_TEXT_BYTES)
            || !bounded_nonempty(&self.recipe_id, MAX_ID_BYTES)
        {
            return Err(RecipeValidationError::Identity);
        }
        self.bindings
            .validate()
            .map_err(|_| RecipeValidationError::Identity)?;
        if let Some(surface) = &self.current_surface {
            if surface.incarnation == 0 || !bounded_nonempty(&surface.surface_id, MAX_ID_BYTES) {
                return Err(RecipeValidationError::Identity);
            }
        }
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ActivityInput {
    schema: String,
    activity_id: String,
    owner_ref: OpaqueOwnerRef,
    title: String,
    recipe_id: String,
    #[serde(flatten)]
    bindings: BindingSet,
    current_surface: Option<SurfacePointer>,
}

impl TryFrom<ActivityInput> for Activity {
    type Error = RecipeValidationError;

    fn try_from(input: ActivityInput) -> Result<Self, Self::Error> {
        let value = Self {
            schema: input.schema,
            activity_id: input.activity_id,
            owner_ref: input.owner_ref,
            title: input.title,
            recipe_id: input.recipe_id,
            bindings: input.bindings,
            current_surface: input.current_surface,
        };
        value.validate()?;
        Ok(value)
    }
}

/// Durable semantic intent and immutable revision record.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "RecipeInput")]
pub struct Recipe {
    pub schema: String,
    pub owner_ref: OpaqueOwnerRef,
    pub recipe_id: String,
    pub revision: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_revision: Option<u64>,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub parent_digest: String,
    pub digest: String,
    pub theme_id: String,
    #[serde(default)]
    pub metadata: BTreeMap<String, PropValue>,
    pub root: SemanticNode,
    #[serde(flatten)]
    pub bindings: BindingSet,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RecipeInput {
    schema: String,
    owner_ref: OpaqueOwnerRef,
    recipe_id: String,
    revision: u64,
    parent_revision: Option<u64>,
    #[serde(default)]
    parent_digest: String,
    digest: String,
    theme_id: String,
    #[serde(default)]
    metadata: BTreeMap<String, PropValue>,
    root: SemanticNode,
    #[serde(flatten)]
    bindings: BindingSet,
}

impl TryFrom<RecipeInput> for Recipe {
    type Error = RecipeValidationError;

    fn try_from(input: RecipeInput) -> Result<Self, Self::Error> {
        let value = Self {
            schema: input.schema,
            owner_ref: input.owner_ref,
            recipe_id: input.recipe_id,
            revision: input.revision,
            parent_revision: input.parent_revision,
            parent_digest: input.parent_digest,
            digest: input.digest,
            theme_id: input.theme_id,
            metadata: input.metadata,
            root: input.root,
            bindings: input.bindings,
        };
        value.validate()?;
        if value.digest != value.content_digest()? {
            return Err(RecipeValidationError::Digest);
        }
        Ok(value)
    }
}

impl Recipe {
    pub fn new(
        owner_ref: OpaqueOwnerRef,
        recipe_id: impl Into<String>,
        theme_id: impl Into<String>,
        root: SemanticNode,
    ) -> Result<Self, RecipeValidationError> {
        let mut value = Self {
            schema: RECIPE_SCHEMA.to_owned(),
            owner_ref,
            recipe_id: recipe_id.into(),
            revision: 1,
            parent_revision: None,
            parent_digest: String::new(),
            digest: String::new(),
            theme_id: theme_id.into(),
            metadata: BTreeMap::new(),
            root,
            bindings: BindingSet::default(),
        };
        value.validate()?;
        value.refresh_digest()?;
        Ok(value)
    }

    pub fn content_digest(&self) -> Result<String, RecipeValidationError> {
        digest_parts(&self.content_tuple()).map_err(|_| RecipeValidationError::Serialization)
    }

    fn content_tuple(&self) -> impl Serialize + '_ {
        (
            &self.schema,
            &self.owner_ref,
            &self.recipe_id,
            self.revision,
            &self.parent_revision,
            &self.parent_digest,
            &self.theme_id,
            &self.metadata,
            &self.root,
            &self.bindings,
        )
    }

    fn refresh_digest(&mut self) -> Result<(), RecipeValidationError> {
        self.digest = self.content_digest()?;
        Ok(())
    }

    pub fn validate(&self) -> Result<(), RecipeValidationError> {
        if self.schema != RECIPE_SCHEMA {
            return Err(RecipeValidationError::Schema);
        }
        if !bounded_nonempty(self.owner_ref.id(), MAX_ID_BYTES)
            || !self.owner_ref.is_valid()
            || !bounded_nonempty(&self.recipe_id, MAX_ID_BYTES)
            || !bounded_nonempty(&self.theme_id, 128)
            || self.revision == 0
        {
            return Err(RecipeValidationError::Identity);
        }
        let has_parent = !self.parent_digest.is_empty() || self.parent_revision.is_some();
        if has_parent != (self.revision > 1) {
            return Err(RecipeValidationError::ParentLink);
        }
        if self.parent_revision.is_some() != has_parent
            || (has_parent && self.parent_digest.is_empty())
            || (has_parent && self.parent_revision.unwrap() == 0)
        {
            return Err(RecipeValidationError::ParentLink);
        }
        if self.metadata.len() > MAX_RECIPE_METADATA {
            return Err(RecipeValidationError::MetadataTooLarge);
        }
        let metadata_bytes = self
            .metadata
            .iter()
            .map(|(key, value)| key.len().saturating_add(value_text_len(value)))
            .sum::<usize>();
        if metadata_bytes > MAX_RECIPE_METADATA_BYTES {
            return Err(RecipeValidationError::MetadataTooLarge);
        }
        self.bindings
            .validate()
            .map_err(|_| RecipeValidationError::Identity)?;
        self.root.validate()?;
        Ok(())
    }

    pub(crate) fn finish_revision(
        &mut self,
        parent_revision: u64,
        parent_digest: String,
    ) -> Result<(), RecipeValidationError> {
        self.parent_revision = Some(parent_revision);
        self.parent_digest = parent_digest;
        self.revision = self
            .revision
            .checked_add(1)
            .ok_or(RecipeValidationError::Identity)?;
        self.validate()?;
        self.refresh_digest()?;
        Ok(())
    }

    pub fn refresh_after_declared_change(&mut self) -> Result<(), RecipeValidationError> {
        self.validate()?;
        self.refresh_digest()
    }

    pub fn downgrade_boundary(&self, target_schema: &str) -> Result<(), RecipeValidationError> {
        if target_schema != "aos.recipe@0.9" {
            return Err(RecipeValidationError::Schema);
        }
        if self.bindings.binding.is_some()
            || self.bindings.rhai.is_some()
            || !self.bindings.extensions.0.is_empty()
        {
            return Err(RecipeValidationError::Identity);
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecipeValidationError {
    Schema,
    Identity,
    ParentLink,
    MetadataTooLarge,
    Digest,
    Serialization,
    Scene(SceneError),
}

impl fmt::Display for RecipeValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Schema => f.write_str("unsupported recipe schema"),
            Self::Identity => f.write_str("invalid recipe identity or revision"),
            Self::ParentLink => f.write_str("recipe revision is not parent-linked"),
            Self::MetadataTooLarge => f.write_str("recipe metadata exceeds bound"),
            Self::Digest => f.write_str("recipe digest does not match canonical content"),
            Self::Serialization => f.write_str("recipe could not be canonically serialized"),
            Self::Scene(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for RecipeValidationError {}

impl From<SceneError> for RecipeValidationError {
    fn from(value: SceneError) -> Self {
        Self::Scene(value)
    }
}

impl From<crate::error::ExtensionError> for RecipeValidationError {
    fn from(_: crate::error::ExtensionError) -> Self {
        Self::Identity
    }
}
