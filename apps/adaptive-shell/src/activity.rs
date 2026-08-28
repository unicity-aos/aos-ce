//! Activity, recipe, surface, and reviewed semantic patch models.

use crate::components::{NodeId, PropValue, SceneError, SemanticNode, StateSet};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fmt;

/// Maximum number of patch operations accepted by the fixture store.
pub const MAX_PATCH_OPS: usize = 64;
/// Maximum bytes in patch metadata strings.
pub const MAX_PATCH_TEXT_BYTES: usize = 4096;
/// Maximum recipe metadata entries.
pub const MAX_RECIPE_METADATA: usize = 16;
/// Maximum UTF-8 bytes in recipe metadata keys and text values.
pub const MAX_RECIPE_METADATA_BYTES: usize = 2048;
/// Maximum UTF-8 bytes in an opaque owner identifier.
pub const MAX_OWNER_REF_BYTES: usize = 256;
/// Maximum UTF-8 bytes in an activity identifier.
pub const MAX_ACTIVITY_ID_BYTES: usize = 256;
/// Maximum UTF-8 bytes in an activity title.
pub const MAX_ACTIVITY_TITLE_BYTES: usize = crate::components::MAX_SEMANTIC_TEXT_BYTES;

fn bounded_nonempty(value: &str, maximum: usize) -> bool {
    !value.is_empty() && value.len() <= maximum
}

/// An opaque reference to an Astrid-owned workspace owner.
///
/// The shell stores the identity it was given, but never derives ownership
/// from a path, HOME, XDG, shell label, or process identity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "id")]
pub enum OpaqueOwnerRef {
    /// Human-owned personal state.
    User(String),
    /// Agent-private state.
    Principal(String),
    /// Deliberately shared team/fleet state.
    Fleet(String),
}

impl OpaqueOwnerRef {
    /// Return the opaque identifier without interpreting it as a path.
    pub fn id(&self) -> &str {
        match self {
            Self::User(id) | Self::Principal(id) | Self::Fleet(id) => id,
        }
    }
}

/// Stable, typed ephemeral surface target.
#[derive(Clone, Debug, Eq, Hash, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(transparent)]
pub struct SurfaceId(String);

impl SurfaceId {
    /// Construct and bound a stable surface identifier.
    pub fn new(value: impl Into<String>) -> Result<Self, SceneError> {
        let value = value.into();
        if value.is_empty() || value.len() > 256 {
            return Err(SceneError::InvalidSurfaceId);
        }
        Ok(Self(value))
    }

    /// Stable string form.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for SurfaceId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl<'de> Deserialize<'de> for SurfaceId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::new(value).map_err(serde::de::Error::custom)
    }
}

fn value_text_len(value: &PropValue) -> usize {
    match value {
        PropValue::Text(value) | PropValue::Token(value) => value.len(),
        PropValue::Number(value) if value.is_finite() => 8,
        PropValue::Number(_) => usize::MAX,
        PropValue::Bool(_) => 1,
    }
}

impl<'de> Deserialize<'de> for OpaqueOwnerRef {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        #[serde(tag = "kind", content = "id")]
        enum Raw {
            User(String),
            Principal(String),
            Fleet(String),
        }
        match Raw::deserialize(deserializer)? {
            Raw::User(value) if bounded_nonempty(&value, MAX_OWNER_REF_BYTES) => {
                Ok(Self::User(value))
            }
            Raw::Principal(value) if bounded_nonempty(&value, MAX_OWNER_REF_BYTES) => {
                Ok(Self::Principal(value))
            }
            Raw::Fleet(value) if bounded_nonempty(&value, MAX_OWNER_REF_BYTES) => {
                Ok(Self::Fleet(value))
            }
            _ => Err(serde::de::Error::custom("invalid opaque owner reference")),
        }
    }
}

/// Durable identity and owner reference for an activity.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "ActivityInput")]
pub struct Activity {
    /// Schema identifier.
    pub schema: String,
    /// Stable activity identity.
    pub activity_id: String,
    /// Astrid-supplied opaque owner reference.
    pub owner_ref: OpaqueOwnerRef,
    /// Human-facing title.
    pub title: String,
    /// Durable recipe identity.
    pub recipe_id: String,
    /// Current ephemeral surface, when materialized.
    pub current_surface: Option<SurfaceId>,
}

impl Activity {
    /// Construct a fixture activity.
    pub fn new(
        activity_id: impl Into<String>,
        owner_ref: OpaqueOwnerRef,
        title: impl Into<String>,
        recipe_id: impl Into<String>,
    ) -> Self {
        Self {
            schema: "aos.activity@1".to_owned(),
            activity_id: activity_id.into(),
            owner_ref,
            title: title.into(),
            recipe_id: recipe_id.into(),
            current_surface: None,
        }
    }

    fn validate(&self) -> Result<(), PatchError> {
        if self.schema != "aos.activity@1"
            || !bounded_nonempty(self.activity_id.as_str(), MAX_ACTIVITY_ID_BYTES)
            || !bounded_nonempty(self.owner_ref.id(), MAX_OWNER_REF_BYTES)
            || !bounded_nonempty(self.title.as_str(), MAX_ACTIVITY_TITLE_BYTES)
            || !bounded_nonempty(self.recipe_id.as_str(), MAX_ACTIVITY_ID_BYTES)
        {
            return Err(PatchError::InvalidActivity);
        }
        Ok(())
    }
}

#[derive(Deserialize)]
struct ActivityInput {
    schema: String,
    activity_id: String,
    owner_ref: OpaqueOwnerRef,
    title: String,
    recipe_id: String,
    current_surface: Option<SurfaceId>,
}

impl TryFrom<ActivityInput> for Activity {
    type Error = PatchError;

    fn try_from(input: ActivityInput) -> Result<Self, Self::Error> {
        let activity = Self {
            schema: input.schema,
            activity_id: input.activity_id,
            owner_ref: input.owner_ref,
            title: input.title,
            recipe_id: input.recipe_id,
            current_surface: input.current_surface,
        };
        activity.validate()?;
        Ok(activity)
    }
}

/// Durable semantic intent and restore truth.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(try_from = "RecipeInput")]
pub struct Recipe {
    /// Schema identifier.
    pub schema: String,
    /// Astrid-supplied opaque owner. Ownership is never inferred locally.
    pub owner_ref: OpaqueOwnerRef,
    /// Stable recipe identity.
    pub recipe_id: String,
    /// Monotonic accepted revision.
    pub revision: u64,
    /// Digest of the canonical semantic content.
    pub digest: String,
    /// Semantic theme identifier.
    pub theme_id: String,
    /// Explicit nonvisual recipe facts. They never become scene draw commands.
    #[serde(default)]
    pub metadata: BTreeMap<String, PropValue>,
    /// Root semantic node.
    pub root: SemanticNode,
}

impl Recipe {
    /// Construct a recipe and calculate its deterministic digest.
    pub fn new(
        owner_ref: OpaqueOwnerRef,
        recipe_id: impl Into<String>,
        theme_id: impl Into<String>,
        root: SemanticNode,
    ) -> Result<Self, RecipeValidationError> {
        let mut recipe = Self {
            schema: "aos.recipe@1".to_owned(),
            owner_ref,
            recipe_id: recipe_id.into(),
            revision: 1,
            digest: String::new(),
            theme_id: theme_id.into(),
            metadata: BTreeMap::new(),
            root,
        };
        recipe.validate()?;
        recipe.refresh_digest();
        Ok(recipe)
    }

    /// Compute the digest over semantic content, excluding the digest itself.
    pub fn computed_digest(&self) -> String {
        let canonical = serde_json::to_vec(&(
            &self.schema,
            &self.recipe_id,
            self.revision,
            &self.theme_id,
            &self.root,
            &self.metadata,
        ))
        .expect("semantic recipe is serializable");
        blake3::hash(&canonical).to_hex().to_string()
    }

    fn refresh_digest(&mut self) {
        self.digest = self.computed_digest();
    }

    /// Validate recipe invariants, semantic bounds, and metadata bounds.
    pub fn validate(&self) -> Result<(), RecipeValidationError> {
        if self.schema != "aos.recipe@1" {
            return Err(RecipeValidationError::Schema);
        }
        if !bounded_nonempty(self.owner_ref.id(), MAX_OWNER_REF_BYTES)
            || !bounded_nonempty(self.recipe_id.as_str(), 256)
            || self.theme_id.is_empty()
            || self.theme_id.len() > 128
            || self.revision == 0
        {
            return Err(RecipeValidationError::Identity);
        }
        if self.metadata.len() > MAX_RECIPE_METADATA {
            return Err(RecipeValidationError::MetadataTooLarge);
        }
        let metadata_bytes = self
            .metadata
            .iter()
            .map(|(key, value)| key.len() + value_text_len(value))
            .sum::<usize>();
        if metadata_bytes > MAX_RECIPE_METADATA_BYTES {
            return Err(RecipeValidationError::MetadataTooLarge);
        }
        self.root
            .validate()
            .map(|_| ())
            .map_err(RecipeValidationError::Scene)
    }

    /// Materialize a new ephemeral incarnation from this recipe.
    pub fn surface(&self, surface_id: SurfaceId, incarnation: u64) -> Surface {
        Surface {
            schema: "aos.surface@1".to_owned(),
            surface_id,
            recipe_id: self.recipe_id.clone(),
            recipe_revision: self.revision,
            incarnation,
            root: self.root.clone(),
        }
    }
}

/// Ephemeral materialized surface.  It is not restore truth.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Surface {
    /// Schema identifier.
    pub schema: String,
    /// Ephemeral surface identity.
    pub surface_id: SurfaceId,
    /// Source recipe identity.
    pub recipe_id: String,
    /// Recipe revision used for materialization.
    pub recipe_revision: u64,
    /// Monotonic incarnation number.
    pub incarnation: u64,
    /// Materialized semantic root.
    pub root: SemanticNode,
}

#[derive(Deserialize)]
struct RecipeInput {
    schema: String,
    owner_ref: OpaqueOwnerRef,
    recipe_id: String,
    revision: u64,
    #[serde(default)]
    digest: String,
    theme_id: String,
    #[serde(default)]
    metadata: BTreeMap<String, PropValue>,
    root: SemanticNode,
}

impl TryFrom<RecipeInput> for Recipe {
    type Error = RecipeValidationError;

    fn try_from(input: RecipeInput) -> Result<Self, Self::Error> {
        let mut recipe = Self {
            schema: input.schema,
            owner_ref: input.owner_ref,
            recipe_id: input.recipe_id,
            revision: input.revision,
            digest: input.digest,
            theme_id: input.theme_id,
            metadata: input.metadata,
            root: input.root,
        };
        recipe.validate()?;
        recipe.refresh_digest();
        Ok(recipe)
    }
}

/// Recipe invariant validation failure.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum RecipeValidationError {
    /// Unsupported recipe schema.
    Schema,
    /// Empty or oversized identity, or nonmonotonic revision.
    Identity,
    /// Recipe metadata exceeds bounded semantic storage.
    MetadataTooLarge,
    /// Semantic root failed validation.
    Scene(SceneError),
}

impl fmt::Display for RecipeValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Schema => f.write_str("unsupported recipe schema"),
            Self::Identity => f.write_str("invalid recipe identity or revision"),
            Self::MetadataTooLarge => f.write_str("recipe metadata exceeds bound"),
            Self::Scene(error) => error.fmt(f),
        }
    }
}

impl std::error::Error for RecipeValidationError {}

impl From<RecipeValidationError> for PatchError {
    fn from(error: RecipeValidationError) -> Self {
        match error {
            RecipeValidationError::Scene(error) => Self::Scene(error),
            _ => Self::InvalidRecipe,
        }
    }
}

/// Bounded semantic operations in a reviewed patch.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub enum PatchOp {
    /// Replace one node's interaction/status flags.
    SetState {
        /// Target node identity.
        node_id: NodeId,
        /// Replacement state flags.
        state: StateSet,
    },
    /// Set one bounded semantic property.
    SetProperty {
        /// Target node identity.
        node_id: NodeId,
        /// Property key.
        key: String,
        /// Property value.
        value: PropValue,
    },
    /// Change the recipe's semantic theme reference.
    SetTheme {
        /// Semantic theme identifier.
        theme_id: String,
    },
    /// Set explicitly nonvisual recipe metadata.
    SetRecipeMetadata {
        /// Stable metadata key.
        key: String,
        /// Bounded metadata value.
        value: PropValue,
    },
}

/// Astrid-supplied acting principal, separate from the workspace owner.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
#[serde(tag = "kind", content = "id")]
pub enum OpaquePrincipalRef {
    /// Human principal.
    User(String),
    /// Agent principal.
    Agent(String),
    /// Automation or service principal.
    Service(String),
}

impl OpaquePrincipalRef {
    fn validate(&self) -> Result<(), PatchError> {
        let id = match self {
            Self::User(id) | Self::Agent(id) | Self::Service(id) => id,
        };
        if id.is_empty() || id.len() > 256 {
            Err(PatchError::InvalidPrincipal)
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
        #[serde(tag = "kind", content = "id")]
        enum Raw {
            User(String),
            Agent(String),
            Service(String),
        }
        fn bounded(value: &str) -> bool {
            !value.is_empty() && value.len() <= 256
        }
        match Raw::deserialize(deserializer)? {
            Raw::User(value) if bounded(&value) => Ok(Self::User(value)),
            Raw::Agent(value) if bounded(&value) => Ok(Self::Agent(value)),
            Raw::Service(value) if bounded(&value) => Ok(Self::Service(value)),
            _ => Err(serde::de::Error::custom("invalid opaque principal")),
        }
    }
}

/// Explicit accepted review attached before a semantic CAS.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ReviewAcceptance {
    /// Reviewing principal.
    pub reviewer: OpaquePrincipalRef,
    /// Opaque review receipt supplied by the review boundary.
    pub receipt: String,
}

/// A bounded, idempotent compare-and-swap recipe patch.
#[derive(Clone, Debug, PartialEq, Serialize)]
#[serde(try_from = "PatchInput")]
pub struct Patch {
    /// Schema identifier.
    pub schema: String,
    /// Owner of the target recipe.
    pub owner_ref: OpaqueOwnerRef,
    /// Principal requesting CAS, distinct from the owner and reviewer.
    pub acting_principal: OpaquePrincipalRef,
    /// Stable proposal identity.
    pub proposal_id: String,
    /// Explicit review acceptance.
    pub review: ReviewAcceptance,
    /// Stable idempotency key.
    pub patch_id: String,
    /// Recipe targeted by this patch.
    pub recipe_id: String,
    /// Expected base revision.
    pub base_revision: u64,
    /// Expected base digest.
    pub base_digest: String,
    /// Human sentence shown before acceptance.
    pub summary: String,
    /// Ordered semantic operations.
    pub operations: Vec<PatchOp>,
}

#[derive(Deserialize)]
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
        let patch = Self {
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
        patch.validate()?;
        Ok(patch)
    }
}

impl Patch {
    /// Validate patch bounds and metadata before attempting CAS.
    pub fn validate(&self) -> Result<(), PatchError> {
        if self.schema != "aos.patch@1" {
            return Err(PatchError::InvalidSchema);
        }
        self.acting_principal.validate()?;
        self.review.reviewer.validate()?;
        if self.acting_principal == self.review.reviewer {
            return Err(PatchError::SelfApproved);
        }
        for text in [
            self.patch_id.as_str(),
            self.recipe_id.as_str(),
            self.proposal_id.as_str(),
            self.review.receipt.as_str(),
            self.base_digest.as_str(),
        ] {
            if text.is_empty() || text.len() > 256 {
                return Err(PatchError::MetadataTooLarge);
            }
        }
        if self.operations.is_empty() || self.operations.len() > MAX_PATCH_OPS {
            return Err(PatchError::TooManyOperations);
        }
        let metadata_bytes = self.patch_id.len()
            + self.recipe_id.len()
            + self.proposal_id.len()
            + self.review.receipt.len()
            + self.base_digest.len()
            + self.summary.len();
        if metadata_bytes > MAX_PATCH_TEXT_BYTES {
            return Err(PatchError::MetadataTooLarge);
        }
        for operation in &self.operations {
            if let PatchOp::SetProperty { key, .. } = operation
                && (key.is_empty() || key.len() > 128)
            {
                return Err(PatchError::InvalidProperty);
            }
            if let PatchOp::SetTheme { theme_id } = operation
                && (theme_id.is_empty() || theme_id.len() > 128)
            {
                return Err(PatchError::InvalidTheme);
            }
            if let PatchOp::SetRecipeMetadata { key, value } = operation
                && (key.is_empty() || key.len() > 128 || value_text_len(value) > 1024)
            {
                return Err(PatchError::InvalidRecipeMetadata);
            }
        }
        Ok(())
    }

    /// Identity of target plus exact operation/content intent, excluding idempotency key.
    pub fn intent_digest(&self) -> String {
        let canonical = serde_json::to_vec(&(
            &self.owner_ref,
            &self.recipe_id,
            self.base_revision,
            &self.base_digest,
            &self.operations,
        ))
        .expect("patch operations are serializable");
        blake3::hash(&canonical).to_hex().to_string()
    }
}

/// Result of applying a reviewed patch.
#[derive(Clone, Debug, PartialEq)]
pub enum PatchOutcome {
    /// Patch accepted and a new recipe is available.
    Applied {
        /// Canonical accepted recipe.
        recipe: Recipe,
        /// Whether any accepted operation changed visual semantic surface.
        visual_changed: bool,
    },
    /// Same idempotency key was already accepted; no second mutation occurred.
    AlreadyApplied(Recipe),
}

/// In-memory recipe store used by the deterministic fixture runner.
#[derive(Clone, Debug, Default)]
pub struct RecipeStore {
    recipes: BTreeMap<String, Recipe>,
    applied_patches: BTreeMap<String, String>,
}

impl RecipeStore {
    /// Create an empty store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Insert a recipe fixture.
    pub fn insert(&mut self, recipe: Recipe) -> Result<(), PatchError> {
        if recipe.schema != "aos.recipe@1" || recipe.recipe_id.is_empty() {
            return Err(PatchError::InvalidRecipe);
        }
        recipe.validate()?;
        let mut recipe = recipe;
        recipe.refresh_digest();
        self.recipes.insert(recipe.recipe_id.clone(), recipe);
        Ok(())
    }

    /// Read a recipe by stable identity.
    pub fn get(&self, recipe_id: &str) -> Option<&Recipe> {
        self.recipes.get(recipe_id)
    }

    /// Apply a reviewed patch with revision and digest CAS.
    pub fn apply_patch(&mut self, patch: &Patch) -> Result<PatchOutcome, PatchError> {
        patch.validate()?;
        let intent = patch.intent_digest();
        if let Some(previous_intent) = self.applied_patches.get(&patch.patch_id) {
            if previous_intent != &intent {
                return Err(PatchError::PatchIdConflict {
                    patch_id: patch.patch_id.clone(),
                });
            }
            let recipe = self
                .recipes
                .get(&patch.recipe_id)
                .ok_or(PatchError::UnknownRecipe)?
                .clone();
            return Ok(PatchOutcome::AlreadyApplied(recipe));
        }
        let recipe = self
            .recipes
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
            match operation {
                PatchOp::SetState { node_id, state } => {
                    let node = find_node_mut(&mut candidate.root, *node_id)
                        .ok_or(PatchError::UnknownNode(*node_id))?;
                    node.state = *state;
                    visual_changed = true;
                }
                PatchOp::SetProperty {
                    node_id,
                    key,
                    value,
                } => {
                    let node = find_node_mut(&mut candidate.root, *node_id)
                        .ok_or(PatchError::UnknownNode(*node_id))?;
                    node.props = node
                        .props
                        .clone()
                        .with(key.clone(), value.clone())
                        .map_err(PatchError::Props)?;
                    visual_changed = true;
                }
                PatchOp::SetTheme { theme_id } => candidate.theme_id = theme_id.clone(),
                PatchOp::SetRecipeMetadata { key, value } => {
                    candidate.metadata.insert(key.clone(), value.clone());
                }
            }
        }
        candidate.revision = candidate
            .revision
            .checked_add(1)
            .ok_or(PatchError::RevisionOverflow)?;
        candidate.refresh_digest();
        candidate.validate()?;
        self.recipes
            .insert(candidate.recipe_id.clone(), candidate.clone());
        self.applied_patches
            .insert(patch.patch_id.clone(), patch.intent_digest());
        Ok(PatchOutcome::Applied {
            recipe: candidate,
            visual_changed,
        })
    }
}

/// Bounded registry of activities and their durable recipe.
#[derive(Clone, Debug, Default)]
pub struct ActivityRegistry {
    entries: BTreeMap<String, (Activity, String)>,
}

impl ActivityRegistry {
    /// Create an empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register an activity to a recipe owned by the same opaque owner.
    pub fn register(&mut self, activity: Activity, recipe: &Recipe) -> Result<(), PatchError> {
        activity.validate()?;
        if activity.owner_ref != recipe.owner_ref || activity.recipe_id != recipe.recipe_id {
            return Err(PatchError::OwnerMismatch);
        }
        if self.entries.contains_key(&activity.activity_id) {
            return Err(PatchError::DuplicateActivity {
                activity_id: activity.activity_id.clone(),
            });
        }
        if self.entries.len() >= 64 {
            return Err(PatchError::MetadataTooLarge);
        }
        self.entries.insert(
            activity.activity_id.clone(),
            (activity, recipe.recipe_id.clone()),
        );
        Ok(())
    }

    /// Rematerialize the selected activity's current recipe incarnation.
    pub fn rematerialize(
        &self,
        activity_id: &str,
        recipes: &RecipeStore,
        next_incarnation: u64,
    ) -> Result<Surface, PatchError> {
        let (_, recipe_id) = self
            .entries
            .get(activity_id)
            .ok_or(PatchError::UnknownRecipe)?;
        let recipe = recipes.get(recipe_id).ok_or(PatchError::UnknownRecipe)?;
        let incarnation = next_incarnation
            .checked_add(1)
            .ok_or(PatchError::IncarnationOverflow)?;
        let surface_id = SurfaceId::new(format!("{activity_id}:{incarnation}"))
            .map_err(|_| PatchError::InvalidRecipe)?;
        Ok(recipe.surface(surface_id, incarnation))
    }
}

fn find_node_mut(node: &mut SemanticNode, target: NodeId) -> Option<&mut SemanticNode> {
    if node.id == target {
        return Some(node);
    }
    for child in &mut node.children {
        if let Some(found) = find_node_mut(child, target) {
            return Some(found);
        }
    }
    None
}

/// Recipe/patch failure.
#[derive(Clone, Debug, PartialEq)]
pub enum PatchError {
    /// Patch schema was not `aos.patch@1`.
    InvalidSchema,
    /// Patch operation count is outside bounds.
    TooManyOperations,
    /// Patch metadata exceeds bounds.
    MetadataTooLarge,
    /// Property key is invalid.
    InvalidProperty,
    /// Theme identifier is invalid.
    InvalidTheme,
    /// Target recipe does not exist.
    UnknownRecipe,
    /// Target node does not exist.
    UnknownNode(NodeId),
    /// Base revision or digest does not match.
    Conflict {
        /// Current revision.
        current_revision: u64,
        /// Current digest.
        current_digest: String,
    },
    /// Scene validation failed.
    Scene(SceneError),
    /// Property validation failed.
    Props(crate::components::PropsError),
    /// Recipe invariant failed.
    InvalidRecipe,
    /// Acting principal is invalid.
    InvalidPrincipal,
    /// Activity identity, owner, or title violated its bounds.
    InvalidActivity,
    /// A patch may not review itself.
    SelfApproved,
    /// Nonvisual metadata operation is invalid.
    InvalidRecipeMetadata,
    /// Same patch id was reused for different intent.
    PatchIdConflict {
        /// Reused idempotency key.
        patch_id: String,
    },
    /// Patch owner does not target recipe owner.
    OwnerMismatch,
    /// Monotonic revision exhausted u64.
    RevisionOverflow,
    /// Ephemeral surface incarnation exhausted u64.
    IncarnationOverflow,
    /// Activity identity was already registered.
    DuplicateActivity {
        /// Reused activity identity.
        activity_id: String,
    },
}

impl fmt::Display for PatchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidSchema => write!(f, "unsupported patch schema"),
            Self::TooManyOperations => write!(f, "patch operation bound exceeded"),
            Self::MetadataTooLarge => write!(f, "patch metadata bound exceeded"),
            Self::InvalidProperty => write!(f, "invalid property key"),
            Self::InvalidTheme => write!(f, "invalid theme id"),
            Self::UnknownRecipe => write!(f, "unknown recipe"),
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
            Self::InvalidRecipe => write!(f, "invalid recipe invariants"),
            Self::InvalidPrincipal => write!(f, "invalid acting principal"),
            Self::InvalidActivity => write!(f, "invalid activity identity or title"),
            Self::SelfApproved => write!(f, "acting principal may not review its own patch"),
            Self::InvalidRecipeMetadata => write!(f, "invalid nonvisual recipe metadata"),
            Self::PatchIdConflict { patch_id } => {
                write!(f, "patch id {patch_id} was reused with different intent")
            }
            Self::OwnerMismatch => write!(f, "patch owner does not match recipe owner"),
            Self::RevisionOverflow => write!(f, "recipe revision overflow"),
            Self::IncarnationOverflow => write!(f, "surface incarnation overflow"),
            Self::DuplicateActivity { activity_id } => {
                write!(f, "activity id {activity_id} is already registered")
            }
        }
    }
}

impl std::error::Error for PatchError {}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::{ComponentKind, SemanticNode};

    fn recipe() -> Recipe {
        Recipe::new(
            OpaqueOwnerRef::Principal("fixture-owner".to_owned()),
            "cat-break",
            "fieldglass-dark",
            SemanticNode::new("root", ComponentKind::Region, "Cat break"),
        )
        .expect("valid fixture")
    }

    #[test]
    fn patch_is_cas_and_idempotent() {
        let recipe = recipe();
        let root_id = recipe.root.id;
        let mut store = RecipeStore::new();
        store.insert(recipe.clone()).expect("insert");
        let patch = Patch {
            schema: "aos.patch@1".to_owned(),
            owner_ref: OpaqueOwnerRef::Principal("fixture-owner".to_owned()),
            acting_principal: OpaquePrincipalRef::Agent("fixture-agent".to_owned()),
            proposal_id: "proposal-1".to_owned(),
            review: ReviewAcceptance {
                reviewer: OpaquePrincipalRef::User("fixture-reviewer".to_owned()),
                receipt: "review-1".to_owned(),
            },
            patch_id: "patch-1".to_owned(),
            recipe_id: recipe.recipe_id.clone(),
            base_revision: recipe.revision,
            base_digest: recipe.digest.clone(),
            summary: "focus the activity".to_owned(),
            operations: vec![PatchOp::SetState {
                node_id: root_id,
                state: StateSet::empty().with(StateSet::FOCUS, true),
            }],
        };
        let applied = store.apply_patch(&patch).expect("accepted");
        let second = store.apply_patch(&patch).expect("idempotent");
        assert!(matches!(
            applied,
            PatchOutcome::Applied {
                visual_changed: true,
                ..
            }
        ));
        assert!(matches!(second, PatchOutcome::AlreadyApplied(_)));
        let stale = Patch {
            base_revision: recipe.revision,
            base_digest: recipe.digest,
            patch_id: "patch-2".to_owned(),
            ..patch
        };
        assert!(matches!(
            store.apply_patch(&stale),
            Err(PatchError::Conflict { .. })
        ));
    }

    #[test]
    fn patch_ids_reject_changed_intent_and_self_review() {
        let recipe = recipe();
        let root_id = recipe.root.id;
        let owner = OpaqueOwnerRef::Principal("fixture-owner".to_owned());
        let make_patch = |id: &str, state: u16| Patch {
            schema: "aos.patch@1".to_owned(),
            owner_ref: owner.clone(),
            acting_principal: OpaquePrincipalRef::Agent("agent".to_owned()),
            proposal_id: "proposal".to_owned(),
            review: ReviewAcceptance {
                reviewer: OpaquePrincipalRef::User("reviewer".to_owned()),
                receipt: "receipt".to_owned(),
            },
            patch_id: id.to_owned(),
            recipe_id: recipe.recipe_id.clone(),
            base_revision: recipe.revision,
            base_digest: recipe.digest.clone(),
            summary: "reviewed".to_owned(),
            operations: vec![PatchOp::SetState {
                node_id: root_id,
                state: StateSet::from_bits(state).expect("state"),
            }],
        };
        let mut store = RecipeStore::new();
        store.insert(recipe.clone()).expect("insert");
        store
            .apply_patch(&make_patch("same-id", StateSet::FOCUS))
            .expect("accepted");
        assert!(matches!(
            store.apply_patch(&make_patch("same-id", StateSet::PRESSED)),
            Err(PatchError::PatchIdConflict { .. })
        ));
        let mut self_review = make_patch("self", StateSet::FOCUS);
        self_review.review = ReviewAcceptance {
            reviewer: self_review.acting_principal.clone(),
            receipt: "self".to_owned(),
        };
        assert_eq!(self_review.validate(), Err(PatchError::SelfApproved));
    }

    #[test]
    fn activity_registry_rematerializes_typed_surface() {
        let recipe = recipe();
        let mut activity = Activity::new(
            "cats",
            OpaqueOwnerRef::Principal("fixture-owner".to_owned()),
            "Cat break",
            recipe.recipe_id.clone(),
        );
        activity.owner_ref = recipe.owner_ref.clone();
        let mut registry = ActivityRegistry::new();
        registry.register(activity, &recipe).expect("register");
        let mut store = RecipeStore::new();
        store.insert(recipe).expect("insert");
        let surface = registry.rematerialize("cats", &store, 1).expect("surface");
        assert_eq!(surface.incarnation, 2);
        assert_eq!(surface.surface_id.as_str(), "cats:2");
    }

    #[test]
    fn recipe_validation_bounds_owner_reference() {
        let mut recipe = recipe();
        recipe.owner_ref = OpaqueOwnerRef::Principal("o".repeat(MAX_OWNER_REF_BYTES + 1));
        assert_eq!(recipe.validate(), Err(RecipeValidationError::Identity));
    }

    #[test]
    fn activity_registry_rejects_oversized_identity_and_title() {
        let recipe = recipe();
        let oversized = [
            Activity::new(
                "cats",
                OpaqueOwnerRef::Principal("o".repeat(MAX_OWNER_REF_BYTES + 1)),
                "Cat break",
                recipe.recipe_id.clone(),
            ),
            Activity::new(
                "a".repeat(MAX_ACTIVITY_ID_BYTES + 1),
                recipe.owner_ref.clone(),
                "Cat break",
                recipe.recipe_id.clone(),
            ),
            Activity::new(
                "cats",
                recipe.owner_ref.clone(),
                "t".repeat(MAX_ACTIVITY_TITLE_BYTES + 1),
                recipe.recipe_id.clone(),
            ),
        ];
        for activity in oversized {
            let mut registry = ActivityRegistry::new();
            assert_eq!(
                registry.register(activity, &recipe),
                Err(PatchError::InvalidActivity)
            );
        }
    }

    #[test]
    fn activity_registry_rejects_duplicate_identity() {
        let recipe = recipe();
        let mut registry = ActivityRegistry::new();
        let first = Activity::new(
            "cats",
            recipe.owner_ref.clone(),
            "Cat break",
            recipe.recipe_id.clone(),
        );
        registry
            .register(first, &recipe)
            .expect("first registration");
        let replacement = Activity::new(
            "cats",
            recipe.owner_ref.clone(),
            "Replacement",
            recipe.recipe_id.clone(),
        );
        assert_eq!(
            registry.register(replacement, &recipe),
            Err(PatchError::DuplicateActivity {
                activity_id: "cats".to_owned(),
            })
        );
    }

    #[test]
    fn recipe_revision_overflow_is_reported() {
        let mut recipe = recipe();
        recipe.revision = u64::MAX;
        let mut store = RecipeStore::new();
        store
            .insert(recipe.clone())
            .expect("max revision remains valid");
        let stored = store.get(&recipe.recipe_id).expect("stored recipe").clone();
        let patch = Patch {
            schema: "aos.patch@1".to_owned(),
            owner_ref: stored.owner_ref.clone(),
            acting_principal: OpaquePrincipalRef::Agent("fixture-agent".to_owned()),
            proposal_id: "overflow-proposal".to_owned(),
            review: ReviewAcceptance {
                reviewer: OpaquePrincipalRef::User("fixture-reviewer".to_owned()),
                receipt: "overflow-review".to_owned(),
            },
            patch_id: "overflow-patch".to_owned(),
            recipe_id: stored.recipe_id.clone(),
            base_revision: stored.revision,
            base_digest: stored.digest.clone(),
            summary: "exercise revision overflow".to_owned(),
            operations: vec![PatchOp::SetState {
                node_id: stored.root.id,
                state: StateSet::empty().with(StateSet::FOCUS, true),
            }],
        };
        assert_eq!(store.apply_patch(&patch), Err(PatchError::RevisionOverflow));
    }

    #[test]
    fn activity_incarnation_overflow_is_reported() {
        let recipe = recipe();
        let mut registry = ActivityRegistry::new();
        let activity = Activity::new(
            "cats",
            recipe.owner_ref.clone(),
            "Cat break",
            recipe.recipe_id.clone(),
        );
        registry.register(activity, &recipe).expect("registration");
        let mut store = RecipeStore::new();
        store.insert(recipe).expect("recipe");
        assert_eq!(
            registry.rematerialize("cats", &store, u64::MAX),
            Err(PatchError::IncarnationOverflow)
        );
    }
}
