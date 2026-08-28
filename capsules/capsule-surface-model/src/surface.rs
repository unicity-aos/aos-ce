//! Ephemeral materialized surface. Never a restore-truth source.

use crate::bindings::BindingSet;
use crate::canonical::valid_blake3_digest;
use crate::components::{SceneError, SemanticNode};
use crate::recipe::{Recipe, RecipeValidationError, SURFACE_SCHEMA};
use serde::{Deserialize, Serialize};

fn valid_surface_id(value: &str) -> bool {
    !value.is_empty() && value.len() <= 256
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Surface {
    pub schema: String,
    pub surface_id: String,
    pub recipe_id: String,
    pub recipe_revision: u64,
    pub recipe_digest: String,
    pub incarnation: u64,
    #[serde(flatten)]
    pub bindings: BindingSet,
    pub root: SemanticNode,
}

impl Surface {
    pub fn from_recipe(
        recipe: &Recipe,
        surface_id: impl Into<String>,
        incarnation: u64,
    ) -> Result<Self, RecipeValidationError> {
        let surface_id = surface_id.into();
        if !valid_surface_id(&surface_id)
            || incarnation == 0
            || !valid_blake3_digest(&recipe.digest)
        {
            return Err(RecipeValidationError::Identity);
        }
        recipe.validate()?;
        Ok(Self {
            schema: SURFACE_SCHEMA.to_owned(),
            surface_id,
            recipe_id: recipe.recipe_id.clone(),
            recipe_revision: recipe.revision,
            recipe_digest: recipe.digest.clone(),
            incarnation,
            bindings: recipe.bindings.clone(),
            root: recipe.root.clone(),
        })
    }

    pub fn validate(&self) -> Result<(), SceneError> {
        if self.schema != SURFACE_SCHEMA
            || !valid_surface_id(&self.surface_id)
            || self.recipe_id.is_empty()
            || self.recipe_revision == 0
            || self.incarnation == 0
            || !valid_blake3_digest(&self.recipe_digest)
            || !self.bindings.validate().is_ok()
        {
            return Err(SceneError::InvalidSurfaceId);
        }
        self.root.validate().map(|_| ())
    }
}
