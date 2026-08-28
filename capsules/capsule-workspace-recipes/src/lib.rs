mod backend;
mod documents;
mod policy;
mod service;

// Keep the fixture implementation private while exposing its Rust seam to
// sibling crates and integration harnesses. This deliberately is not an AOS
// runtime entry point: no WIT world, host handler, or authority is implied.
pub use backend::{BackendError, FixtureBackend};
pub use documents::ConflictRecord;
pub use policy::{GrantDirectory, GrantProof, GrantRecord, OwnerContext, PolicyError};
pub use service::{
    ActivityConflictReport, CommitStage, ConflictReport, RecipeService, ServiceError,
    scope_for_owner, transition_activity_owner, transition_recipe_owner,
};

#[cfg(test)]
mod lib_tests;
