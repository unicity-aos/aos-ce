//! Bounded compare-and-swap recipe ledger.

use crate::activity::{OpaqueOwnerRef, OpaquePrincipalRef};
use crate::bindings::{Binding, RhaiReference};
use crate::canonical::{digest_parts, valid_blake3_digest};
use crate::components::{MAX_NODES, NodeId, PropValue, SceneError, SemanticNode};
use crate::recipe::{PATCH_SCHEMA, Recipe, RecipeValidationError};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

pub const MAX_PATCH_OPS: usize = 64;
pub const MAX_PATCH_TEXT_BYTES: usize = 4096;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewAcceptance {
    pub reviewer: OpaquePrincipalRef,
    pub receipt: String,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PatchOp {
    SetState {
        node_id: NodeId,
        state: crate::components::StateSet,
    },
    SetProperty {
        node_id: NodeId,
        key: String,
        value: PropValue,
    },
    ReplaceRoot {
        root: SemanticNode,
    },
    ClearRoot,
    SetTheme {
        theme_id: String,
    },
    SetRecipeMetadata {
        key: String,
        value: PropValue,
    },
    SetBinding {
        binding: Binding,
    },
    SetRhai {
        reference: RhaiReference,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "PatchInput")]
pub struct Patch {
    pub schema: String,
    pub owner_ref: OpaqueOwnerRef,
    pub acting_principal: OpaquePrincipalRef,
    pub proposal_id: String,
    pub review: ReviewAcceptance,
    pub patch_id: String,
    pub recipe_id: String,
    pub base_revision: u64,
    pub base_digest: String,
    pub summary: String,
    pub operations: Vec<PatchOp>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct PatchInput {
    schema: String,
    owner_ref: OpaqueOwnerRef,
    acting_principal: OpaquePrincipalRef,
    proposal_id: String,
    review: ReviewAcceptance,
    patch_id: String,
    recipe_id: String,
    base_revision: u64,
    base_digest: String,
    summary: String,
    operations: Vec<PatchOp>,
}

impl TryFrom<PatchInput> for Patch {
    type Error = PatchError;

    fn try_from(input: PatchInput) -> Result<Self, Self::Error> {
        let value = Self {
            schema: input.schema,
            owner_ref: input.owner_ref,
            acting_principal: input.acting_principal,
            proposal_id: input.proposal_id,
            review: input.review,
            patch_id: input.patch_id,
            recipe_id: input.recipe_id,
            base_revision: input.base_revision,
            base_digest: input.base_digest,
            summary: input.summary,
            operations: input.operations,
        };
        value.validate()?;
        Ok(value)
    }
}

impl Patch {
    pub fn new(
        owner_ref: OpaqueOwnerRef,
        acting_principal: OpaquePrincipalRef,
        reviewer: OpaquePrincipalRef,
        receipt: impl Into<String>,
        patch_id: impl Into<String>,
        recipe_id: impl Into<String>,
        base_revision: u64,
        base_digest: impl Into<String>,
        summary: impl Into<String>,
        operations: Vec<PatchOp>,
    ) -> Result<Self, PatchError> {
        let patch_id = patch_id.into();
        let value = Self {
            schema: PATCH_SCHEMA.to_owned(),
            owner_ref,
            acting_principal,
            proposal_id: format!("proposal:{patch_id}"),
            review: ReviewAcceptance {
                reviewer,
                receipt: receipt.into(),
            },
            patch_id,
            recipe_id: recipe_id.into(),
            base_revision,
            base_digest: base_digest.into(),
            summary: summary.into(),
            operations,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> Result<(), PatchError> {
        if self.schema != PATCH_SCHEMA {
            return Err(PatchError::InvalidSchema);
        }
        if !self.owner_ref.is_valid() {
            return Err(PatchError::OwnerMismatch);
        }
        self.acting_principal.validate()?;
        self.review.reviewer.validate()?;
        if self.acting_principal == self.review.reviewer {
            return Err(PatchError::SelfApproved);
        }
        if self.base_revision == 0 || !valid_blake3_digest(&self.base_digest) {
            return Err(PatchError::MetadataTooLarge);
        }
        let fields = [
            self.patch_id.as_str(),
            self.recipe_id.as_str(),
            self.proposal_id.as_str(),
            self.review.receipt.as_str(),
            self.base_digest.as_str(),
            self.summary.as_str(),
        ];
        let metadata_bytes = fields.iter().map(|value| value.len()).sum::<usize>();
        if fields
            .iter()
            .any(|value| value.is_empty() || value.len() > 256)
            || metadata_bytes > MAX_PATCH_TEXT_BYTES
        {
            return Err(PatchError::MetadataTooLarge);
        }
        if self.operations.is_empty() || self.operations.len() > MAX_PATCH_OPS {
            return Err(PatchError::TooManyOperations);
        }
        for operation in &self.operations {
            match operation {
                PatchOp::SetProperty { key, value, .. } => {
                    if key.is_empty() || key.len() > 128 || value.byte_len() == usize::MAX {
                        return Err(PatchError::InvalidProperty);
                    }
                }
                PatchOp::SetTheme { theme_id } => {
                    if theme_id.is_empty() || theme_id.len() > 128 {
                        return Err(PatchError::InvalidTheme);
                    }
                }
                PatchOp::SetRecipeMetadata { key, value } => {
                    if key.is_empty() || key.len() > 128 || value.byte_len() > 1024 {
                        return Err(PatchError::InvalidRecipeMetadata);
                    }
                }
                PatchOp::SetBinding { binding } => {
                    binding.validate().map_err(|_| PatchError::InvalidBinding)?;
                }
                PatchOp::SetRhai { reference } => {
                    reference.validate().map_err(|_| PatchError::InvalidRhai)?;
                }
                PatchOp::SetState { state, .. } => {
                    crate::components::StateSet::from_bits(state.bits())
                        .map_err(|_| PatchError::InvalidState)?;
                }
                PatchOp::ReplaceRoot { root } => {
                    root.validate().map_err(PatchError::Scene)?;
                }
                PatchOp::ClearRoot => {}
            }
        }
        Ok(())
    }

    pub fn intent_digest(&self) -> Result<String, PatchError> {
        digest_parts(&(
            &self.owner_ref,
            &self.recipe_id,
            self.base_revision,
            &self.base_digest,
            &self.operations,
        ))
        .map_err(|_| PatchError::Serialization)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PatchOutcome {
    Applied {
        recipe: Recipe,
        visual_changed: bool,
    },
    AlreadyApplied(Recipe),
}

#[derive(Clone, Debug, PartialEq)]
pub struct ConflictCandidate {
    pub current: Recipe,
    pub incoming: Patch,
    pub merged: Option<Patch>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct RollbackRequest {
    pub recipe_id: String,
    pub target_revision: u64,
    pub acting_principal: OpaquePrincipalRef,
    pub reviewer: OpaquePrincipalRef,
    pub receipt: String,
}

#[derive(Clone, Debug, Default)]
pub struct RecipeStore {
    current: BTreeMap<String, Recipe>,
    history: BTreeMap<(String, u64), Recipe>,
    applied_patches: BTreeMap<String, String>,
    applied_patch_by_revision: BTreeMap<(String, u64), Patch>,
}

impl RecipeStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn insert(&mut self, recipe: Recipe) -> Result<(), PatchError> {
        if recipe.schema != crate::recipe::RECIPE_SCHEMA || recipe.revision != 1 {
            return Err(PatchError::Recipe(RecipeValidationError::Identity));
        }
        recipe.validate().map_err(PatchError::Recipe)?;
        if recipe.digest != recipe.content_digest().map_err(PatchError::Recipe)? {
            return Err(PatchError::Recipe(RecipeValidationError::Digest));
        }
        if self.current.contains_key(&recipe.recipe_id) {
            return Err(PatchError::DuplicateRecipe {
                recipe_id: recipe.recipe_id.clone(),
            });
        }
        self.history
            .insert((recipe.recipe_id.clone(), recipe.revision), recipe.clone());
        self.current.insert(recipe.recipe_id.clone(), recipe);
        Ok(())
    }

    pub fn get(&self, recipe_id: &str) -> Option<&Recipe> {
        self.current.get(recipe_id)
    }

    pub fn revision(&self, recipe_id: &str, revision: u64) -> Option<&Recipe> {
        self.history.get(&(recipe_id.to_owned(), revision))
    }

    pub fn apply_patch(&mut self, patch: &Patch) -> Result<PatchOutcome, PatchError> {
        match self.apply_patch_once(patch)? {
            PatchOutcome::Applied {
                recipe,
                visual_changed,
            } => Ok(PatchOutcome::Applied {
                recipe,
                visual_changed,
            }),
            PatchOutcome::AlreadyApplied(recipe) => Ok(PatchOutcome::AlreadyApplied(recipe)),
        }
    }

    fn apply_patch_once(&mut self, patch: &Patch) -> Result<PatchOutcome, PatchError> {
        patch.validate()?;
        let intent = patch.intent_digest()?;
        if let Some(previous_intent) = self.applied_patches.get(&patch.patch_id) {
            if previous_intent != &intent {
                return Err(PatchError::PatchIdConflict {
                    patch_id: patch.patch_id.clone(),
                });
            }
            let recipe = self
                .current
                .get(&patch.recipe_id)
                .ok_or(PatchError::UnknownRecipe)?
                .clone();
            return Ok(PatchOutcome::AlreadyApplied(recipe));
        }

        let recipe = self
            .current
            .get(&patch.recipe_id)
            .ok_or(PatchError::UnknownRecipe)?;
        if recipe.owner_ref != patch.owner_ref {
            return Err(PatchError::OwnerMismatch);
        }
        if recipe.revision != patch.base_revision || recipe.digest != patch.base_digest {
            return Err(PatchError::Conflict {
                current_revision: recipe.revision,
                current_digest: recipe.digest.clone(),
            });
        }

        let mut candidate = recipe.clone();
        let mut visual_changed = false;
        for operation in &patch.operations {
            apply_operation(&mut candidate, operation, &mut visual_changed)?;
        }
        let parent_revision = recipe.revision;
        let parent_digest = recipe.digest.clone();
        candidate
            .finish_revision(parent_revision, parent_digest)
            .map_err(PatchError::Recipe)?;
        self.history.insert(
            (candidate.recipe_id.clone(), candidate.revision),
            candidate.clone(),
        );
        self.applied_patch_by_revision.insert(
            (candidate.recipe_id.clone(), candidate.revision),
            patch.clone(),
        );
        self.current
            .insert(candidate.recipe_id.clone(), candidate.clone());
        self.applied_patches
            .insert(patch.patch_id.clone(), patch.intent_digest()?);
        Ok(PatchOutcome::Applied {
            recipe: candidate,
            visual_changed,
        })
    }

    pub fn apply_patch_with_narrow_merge(
        &mut self,
        incoming: &Patch,
        merged_patch_id: &str,
    ) -> Result<PatchOutcome, PatchError> {
        match self.apply_patch_once(incoming) {
            Ok(outcome) => Ok(outcome),
            Err(PatchError::Conflict { .. }) => {
                let candidate = self.conflict_candidate(incoming, merged_patch_id)?;
                let merged = candidate
                    .merged
                    .ok_or(PatchError::DisjointFieldMergeRejected)?;
                self.apply_patch(&merged)
            }
            Err(error) => Err(error),
        }
    }

    pub fn conflict_candidate(
        &self,
        incoming: &Patch,
        merged_patch_id: &str,
    ) -> Result<ConflictCandidate, PatchError> {
        incoming.validate()?;
        let current = self
            .current
            .get(&incoming.recipe_id)
            .ok_or(PatchError::UnknownRecipe)?;
        if current.owner_ref != incoming.owner_ref {
            return Err(PatchError::OwnerMismatch);
        }
        if current.revision == incoming.base_revision && current.digest == incoming.base_digest {
            return Err(PatchError::NotConflicted);
        }
        let previous = self
            .applied_patch_by_revision
            .get(&(incoming.recipe_id.clone(), current.revision));
        let merged = match previous {
            Some(previous) => match merge_disjoint_fields(previous, incoming, merged_patch_id) {
                Ok(mut merged) => {
                    merged.base_revision = current.revision;
                    merged.base_digest = current.digest.clone();
                    Some(merged)
                }
                Err(error) => return Err(error),
            },
            None => None,
        };
        Ok(ConflictCandidate {
            current: current.clone(),
            incoming: incoming.clone(),
            merged,
        })
    }

    pub fn rollback(
        &mut self,
        request: RollbackRequest,
        patch_id: impl Into<String>,
    ) -> Result<Recipe, PatchError> {
        if request.acting_principal == request.reviewer {
            return Err(PatchError::SelfApproved);
        }
        request.acting_principal.validate()?;
        request.reviewer.validate()?;
        let current = self
            .current
            .get(&request.recipe_id)
            .ok_or(PatchError::UnknownRecipe)?
            .clone();
        if request.receipt.is_empty() || request.receipt.len() > 256 {
            return Err(PatchError::MetadataTooLarge);
        }
        let target = self
            .history
            .get(&(request.recipe_id.clone(), request.target_revision))
            .ok_or(PatchError::UnknownRecipeRevision)?
            .clone();
        let mut candidate = target;
        candidate.bindings = current.bindings.clone();
        candidate.revision = current.revision;
        candidate
            .finish_revision(current.revision, current.digest.clone())
            .map_err(PatchError::Recipe)?;
        self.history.insert(
            (candidate.recipe_id.clone(), candidate.revision),
            candidate.clone(),
        );
        self.current
            .insert(candidate.recipe_id.clone(), candidate.clone());
        let intent = digest_parts(&(
            &candidate.owner_ref,
            &candidate.recipe_id,
            current.revision,
            &current.digest,
            &candidate.parent_revision,
        ))
        .map_err(|_| PatchError::Serialization)?;
        self.applied_patches.insert(patch_id.into(), intent);
        Ok(candidate)
    }

    pub fn replay(recipe_json: &[u8], patches: &[Patch]) -> Result<Recipe, PatchError> {
        let recipe: Recipe = crate::canonical::parse_canonical(recipe_json)
            .map_err(|_| PatchError::InvalidCanonicalDocument)?;
        let mut store = Self::new();
        store.insert(recipe)?;
        for patch in patches {
            if let PatchOutcome::AlreadyApplied(_) = store.apply_patch(patch)? {
                return Err(PatchError::ReplayUnexpectedlyIdempotent);
            }
        }
        store
            .current
            .values()
            .next()
            .cloned()
            .ok_or(PatchError::UnknownRecipe)
    }
}

fn apply_operation(
    recipe: &mut Recipe,
    operation: &PatchOp,
    visual_changed: &mut bool,
) -> Result<(), PatchError> {
    match operation {
        PatchOp::SetState { node_id, state } => {
            let node = recipe
                .root
                .find_mut(*node_id)
                .ok_or(PatchError::UnknownNode(*node_id))?;
            node.state = *state;
            *visual_changed = true;
        }
        PatchOp::SetProperty {
            node_id,
            key,
            value,
        } => {
            let node = recipe
                .root
                .find_mut(*node_id)
                .ok_or(PatchError::UnknownNode(*node_id))?;
            node.props = std::mem::take(&mut node.props)
                .with(key.clone(), value.clone())
                .map_err(PatchError::Props)?;
            *visual_changed = true;
        }
        PatchOp::ReplaceRoot { root } => {
            if root.validate().map_err(PatchError::Scene)? > MAX_NODES {
                return Err(PatchError::Scene(SceneError::TooManyNodes));
            }
            recipe.root = root.clone();
            *visual_changed = true;
        }
        PatchOp::ClearRoot => {
            recipe.root = SemanticNode::new(
                "surface-model/cleared-root",
                crate::components::ComponentKind::Region,
                "Cleared surface",
            );
            *visual_changed = true;
        }
        PatchOp::SetTheme { theme_id } => recipe.theme_id = theme_id.clone(),
        PatchOp::SetRecipeMetadata { key, value } => {
            recipe.metadata.insert(key.clone(), value.clone());
        }
        PatchOp::SetBinding { binding } => recipe.bindings.binding = Some(binding.clone()),
        PatchOp::SetRhai { reference } => recipe.bindings.rhai = Some(reference.clone()),
    }
    Ok(())
}

pub fn merge_disjoint_fields(
    base: &Patch,
    incoming: &Patch,
    merged_patch_id: &str,
) -> Result<Patch, PatchError> {
    base.validate()?;
    incoming.validate()?;
    if base.recipe_id != incoming.recipe_id
        || base.owner_ref != incoming.owner_ref
        || merged_patch_id.is_empty()
        || merged_patch_id.len() > 256
        || merged_patch_id == base.patch_id
        || merged_patch_id == incoming.patch_id
    {
        return Err(PatchError::DisjointFieldMergeRejected);
    }

    let mut metadata_keys = BTreeSet::new();
    let mut property_fields = BTreeSet::new();
    for patch in [base, incoming] {
        for operation in &patch.operations {
            match operation {
                PatchOp::SetRecipeMetadata { key, .. } => {
                    if !metadata_keys.insert(key.clone()) {
                        return Err(PatchError::DisjointFieldMergeRejected);
                    }
                }
                PatchOp::SetProperty { node_id, key, .. } => {
                    if !property_fields.insert((*node_id, key.clone())) {
                        return Err(PatchError::DisjointFieldMergeRejected);
                    }
                }
                _ => return Err(PatchError::DisjointFieldMergeRejected),
            }
        }
    }
    if base.operations.len() + incoming.operations.len() > MAX_PATCH_OPS {
        return Err(PatchError::TooManyOperations);
    }
    let mut operations = base.operations.clone();
    operations.extend(incoming.operations.clone());
    let mut merged = base.clone();
    merged.patch_id = merged_patch_id.to_owned();
    merged.proposal_id = format!("proposal:{merged_patch_id}");
    merged.operations = operations;
    merged.validate()?;
    Ok(merged)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PatchError {
    InvalidSchema,
    TooManyOperations,
    MetadataTooLarge,
    InvalidProperty,
    InvalidTheme,
    InvalidRecipeMetadata,
    InvalidBinding,
    InvalidRhai,
    InvalidState,
    UnknownRecipe,
    UnknownRecipeRevision,
    UnknownNode(NodeId),
    Conflict {
        current_revision: u64,
        current_digest: String,
    },
    Scene(SceneError),
    Props(crate::components::PropsError),
    Recipe(RecipeValidationError),
    InvalidPrincipal,
    SelfApproved,
    OwnerMismatch,
    Serialization,
    RevisionOverflow,
    PatchIdConflict {
        patch_id: String,
    },
    DuplicateRecipe {
        recipe_id: String,
    },
    InvalidCanonicalDocument,
    ReplayUnexpectedlyIdempotent,
    DisjointFieldMergeRejected,
    NotConflicted,
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSchema => f.write_str("unsupported patch schema"),
            Self::TooManyOperations => f.write_str("patch operation bound exceeded"),
            Self::MetadataTooLarge => f.write_str("patch metadata bound exceeded"),
            Self::InvalidProperty => f.write_str("invalid property operation"),
            Self::InvalidTheme => f.write_str("invalid theme id"),
            Self::InvalidRecipeMetadata => f.write_str("invalid recipe metadata operation"),
            Self::InvalidBinding => f.write_str("binding description is invalid"),
            Self::InvalidRhai => f.write_str("Rhai reference is invalid"),
            Self::InvalidState => f.write_str("invalid semantic state"),
            Self::UnknownRecipe => f.write_str("unknown recipe"),
            Self::UnknownRecipeRevision => f.write_str("unknown recipe revision"),
            Self::UnknownNode(id) => write!(f, "unknown semantic node {id}"),
            Self::Conflict {
                current_revision,
                current_digest,
            } => write!(
                f,
                "recipe conflict at revision {current_revision} ({current_digest})"
            ),
            Self::Scene(error) => error.fmt(f),
            Self::Props(error) => error.fmt(f),
            Self::Recipe(error) => error.fmt(f),
            Self::InvalidPrincipal => f.write_str("invalid principal reference"),
            Self::SelfApproved => f.write_str("acting principal may not review its own patch"),
            Self::OwnerMismatch => f.write_str("patch owner does not match recipe owner"),
            Self::Serialization => f.write_str("canonical serialization failed"),
            Self::RevisionOverflow => f.write_str("recipe revision overflow"),
            Self::PatchIdConflict { patch_id } => {
                write!(f, "patch id {patch_id} was reused with different intent")
            }
            Self::DuplicateRecipe { recipe_id } => write!(f, "recipe {recipe_id} already exists"),
            Self::InvalidCanonicalDocument => f.write_str("canonical replay document is invalid"),
            Self::ReplayUnexpectedlyIdempotent => {
                f.write_str("replay patch was unexpectedly duplicate")
            }
            Self::DisjointFieldMergeRejected => {
                f.write_str("conflicting fields are not narrowly disjoint")
            }
            Self::NotConflicted => f.write_str("input is not in conflict"),
        }
    }
}

impl std::error::Error for PatchError {}
