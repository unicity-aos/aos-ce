//! Canonical, host-independent document model for AOS surfaces.
//!
//! Durable activity and recipe documents are restore truth. Surface documents
//! are ephemeral incarnations. Patches are bounded compare-and-swap requests.
//! Identity in payloads is a label until a kernel-stamped caller supplies it.

pub mod a2ui;
pub mod activity;
pub mod bindings;
pub mod canonical;
pub mod components;
pub mod error;
pub mod fixtures;
pub mod recipe;
pub mod store;
pub mod surface;

pub use bindings::{
    Binding, CachePolicy, CapsuleBinding, DataBinding, FreshnessPolicy, NativeBinding,
    RestorePolicy, RhaiReference,
};
pub use canonical::{CanonicalJson, canonical_bytes, canonical_string, parse_canonical};
pub use components::{
    ComponentKind, MAX_NODES, NodeId, PropValue, SceneError, SemanticNode, StateSet,
};
pub use error::{DocumentError, ExtensionError};
pub use recipe::Activity;
pub use recipe::{
    ACTIVITY_SCHEMA, PATCH_SCHEMA, RECIPE_SCHEMA, Recipe, RecipeValidationError, SURFACE_SCHEMA,
};
pub use store::{ConflictCandidate, PatchError, PatchOutcome, RecipeStore, RollbackRequest};
pub use surface::Surface;
