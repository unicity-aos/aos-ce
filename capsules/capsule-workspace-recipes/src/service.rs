use crate::backend::{BackendError, FixtureBackend, RecipeLocation, scope_key};
use crate::documents::{
    ActivityPointer, ConflictRecord, CurrentPointer, JournalRecord, RevisionEnvelope, canonical,
    digest, parse_canonical,
};
use crate::policy::{
    GrantDirectory, OwnerContext, PolicyError, scan_for_forbidden_content, valid_key_id,
};
use capsule_surface_model::recipe::{ACTIVITY_SCHEMA, Activity, RECIPE_SCHEMA, Recipe};
use capsule_surface_model::store::{
    ConflictCandidate, Patch, PatchError, PatchOp, PatchOutcome, RecipeStore, RollbackRequest,
};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CommitStage {
    Object,
    Revision,
    Pointer,
    Journal,
}

#[derive(Debug)]
pub enum ServiceError {
    Model(PatchError),
    Recipe(capsule_surface_model::recipe::RecipeValidationError),
    Json(serde_json::Error),
    Backend(BackendError),
    Policy(PolicyError),
    Crash(CommitStage),
    CorruptState,
    DuplicateRecipe(String),
    Conflict(Box<ConflictReport>),
    ActivityConflict(Box<ActivityConflictReport>),
}

impl From<PatchError> for ServiceError {
    fn from(value: PatchError) -> Self {
        Self::Model(value)
    }
}

impl From<capsule_surface_model::recipe::RecipeValidationError> for ServiceError {
    fn from(value: capsule_surface_model::recipe::RecipeValidationError) -> Self {
        Self::Recipe(value)
    }
}

impl From<serde_json::Error> for ServiceError {
    fn from(value: serde_json::Error) -> Self {
        Self::Json(value)
    }
}

impl From<BackendError> for ServiceError {
    fn from(value: BackendError) -> Self {
        Self::Backend(value)
    }
}

impl From<PolicyError> for ServiceError {
    fn from(value: PolicyError) -> Self {
        Self::Policy(value)
    }
}

#[derive(Debug)]
pub struct ConflictReport {
    current: Recipe,
    incoming: Patch,
    merged: Option<Patch>,
}

impl ConflictReport {
    pub fn current(&self) -> &Recipe {
        &self.current
    }

    pub fn incoming(&self) -> &Patch {
        &self.incoming
    }

    pub fn merged(&self) -> Option<&Patch> {
        self.merged.as_ref()
    }
}

#[derive(Debug)]
pub struct ActivityConflictReport {
    current: Option<Activity>,
    incoming: Activity,
}

impl ActivityConflictReport {
    pub fn current(&self) -> Option<&Activity> {
        self.current.as_ref()
    }

    pub fn incoming(&self) -> &Activity {
        &self.incoming
    }
}

pub struct RecipeService {
    pub(crate) backend: FixtureBackend,
    pub(crate) grants: GrantDirectory,
    pub(crate) stores: BTreeMap<String, RecipeStore>,
}

impl RecipeService {
    pub fn open(mut backend: FixtureBackend, grants: GrantDirectory) -> Result<Self, ServiceError> {
        backend.clear_fault();
        let mut stores: BTreeMap<String, RecipeStore> = BTreeMap::new();
        let mut referenced_objects = BTreeSet::new();
        let mut referenced_revisions = BTreeSet::new();

        for committed in backend.committed_pointers() {
            let location = committed.location.clone();
            let mut final_recipe = None;
            for revision in 1..=committed.pointer.revision {
                let envelope =
                    backend.revision_envelope(&location.scope, &location.recipe_id, revision)?;
                let recipe = envelope.recipe;
                if recipe.schema != RECIPE_SCHEMA
                    || recipe.digest != recipe.content_digest()?
                    || scope_for_owner(&recipe.owner_ref)? != location.scope
                    || recipe.recipe_id != location.recipe_id
                {
                    return Err(ServiceError::CorruptState);
                }
                let recipe_bytes = canonical(&recipe)?;
                if !backend.object_matches(&recipe.digest, &recipe_bytes) {
                    return Err(ServiceError::CorruptState);
                }
                referenced_objects.insert(format!("object/{}", recipe.digest));
                referenced_revisions.insert((
                    location.scope.clone(),
                    location.recipe_id.clone(),
                    revision,
                ));

                if revision == 1 {
                    if envelope.patch.is_some() {
                        return Err(ServiceError::CorruptState);
                    }
                    stores
                        .entry(location.scope.clone())
                        .or_default()
                        .insert(recipe.clone())?;
                } else {
                    let Some(patch) = envelope.patch.as_ref() else {
                        return Err(ServiceError::CorruptState);
                    };
                    if patch.recipe_id != location.recipe_id
                        || scope_for_owner(&patch.owner_ref)? != location.scope
                        || !valid_key_id(&patch.patch_id)
                    {
                        return Err(ServiceError::CorruptState);
                    }
                    let expected = JournalRecord {
                        patch_id: patch.patch_id.clone(),
                        recipe_id: patch.recipe_id.clone(),
                        revision,
                        recipe_digest: recipe.digest.clone(),
                        intent_digest: patch.intent_digest()?,
                    };
                    match backend.journal_for_recipe(
                        &location.scope,
                        &location.recipe_id,
                        &patch.patch_id,
                    ) {
                        Some(existing) if existing != &expected => {
                            return Err(ServiceError::CorruptState);
                        }
                        Some(_) => {}
                        None => backend.insert_recovered_journal(
                            &location.scope,
                            &location.recipe_id,
                            expected,
                        ),
                    }
                    let outcome = stores
                        .entry(location.scope.clone())
                        .or_default()
                        .apply_patch(patch)?;
                    match outcome {
                        PatchOutcome::Applied {
                            recipe: applied, ..
                        } if applied == recipe => {}
                        PatchOutcome::AlreadyApplied(applied) if applied == recipe => {}
                        _ => return Err(ServiceError::CorruptState),
                    }
                }
                final_recipe = Some(recipe);
            }

            let final_recipe = final_recipe.ok_or(ServiceError::CorruptState)?;
            if final_recipe.revision != committed.pointer.revision
                || final_recipe.digest != committed.pointer.recipe_digest
            {
                return Err(ServiceError::CorruptState);
            }
        }

        backend.quarantine_orphans(&referenced_objects, &referenced_revisions);
        Ok(Self {
            backend,
            grants,
            stores,
        })
    }

    pub fn create_recipe(
        &mut self,
        context: &OwnerContext,
        recipe: Recipe,
    ) -> Result<Recipe, ServiceError> {
        self.authorize(context, &recipe.owner_ref, &context.acting_principal)?;
        if !valid_key_id(&recipe.recipe_id) {
            return Err(ServiceError::Policy(PolicyError::ForbiddenContent));
        }
        scan_for_forbidden_content(&recipe)?;
        recipe.validate()?;
        if recipe.schema != RECIPE_SCHEMA || recipe.revision != 1 {
            return Err(ServiceError::CorruptState);
        }
        let scope = scope_for_owner(&recipe.owner_ref)?;
        let location = RecipeLocation {
            scope: scope.clone(),
            recipe_id: recipe.recipe_id.clone(),
        };
        if self.backend.current(&location)?.is_some() {
            return Err(ServiceError::DuplicateRecipe(recipe.recipe_id));
        }
        let expected = self.backend.current(&location)?;
        let mut staged_stores = self.stores.clone();
        staged_stores
            .entry(scope.clone())
            .or_default()
            .insert(recipe.clone())?;
        commit_recipe(
            &mut self.backend,
            &location,
            &recipe,
            None,
            expected,
            CurrentPointer {
                revision: 1,
                recipe_digest: recipe.digest.clone(),
            },
        )?;
        self.stores = staged_stores;
        Ok(recipe)
    }

    pub fn apply_patch(
        &mut self,
        context: &OwnerContext,
        patch: Patch,
    ) -> Result<PatchOutcome, ServiceError> {
        self.authorize(context, &patch.owner_ref, &patch.acting_principal)?;
        if !valid_key_id(&patch.patch_id) || !valid_key_id(&patch.recipe_id) {
            return Err(ServiceError::Policy(PolicyError::ForbiddenContent));
        }
        scan_for_forbidden_content(&patch)?;
        patch.validate()?;
        let scope = scope_for_owner(&patch.owner_ref)?;
        let location = RecipeLocation {
            scope: scope.clone(),
            recipe_id: patch.recipe_id.clone(),
        };
        let expected = self.backend.current(&location)?;
        let mut staged_stores = self.stores.clone();
        let outcome = staged_stores
            .entry(scope.clone())
            .or_default()
            .apply_patch(&patch);
        match outcome {
            Ok(PatchOutcome::Applied {
                recipe,
                visual_changed,
            }) => {
                commit_recipe(
                    &mut self.backend,
                    &location,
                    &recipe,
                    Some(&patch),
                    expected,
                    CurrentPointer {
                        revision: recipe.revision,
                        recipe_digest: recipe.digest.clone(),
                    },
                )?;
                self.stores = staged_stores;
                Ok(PatchOutcome::Applied {
                    recipe,
                    visual_changed,
                })
            }
            Ok(already @ PatchOutcome::AlreadyApplied(_)) => Ok(already),
            Err(PatchError::Conflict { .. }) => {
                let store = self
                    .stores
                    .get(&scope)
                    .ok_or(ServiceError::Model(PatchError::UnknownRecipe))?;
                let candidate = conflict_candidate(store, &patch, "unused")?;
                self.backend.retain_conflict(
                    &location,
                    ConflictRecord {
                        current_revision: candidate.current.revision,
                        current_recipe_digest: candidate.current.digest.clone(),
                        incoming_patch_id: patch.patch_id.clone(),
                        incoming_base_revision: patch.base_revision,
                        incoming_base_digest: patch.base_digest.clone(),
                    },
                );
                Err(ServiceError::Conflict(Box::new(report(candidate, patch))))
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn apply_patch_with_narrow_merge(
        &mut self,
        context: &OwnerContext,
        patch: Patch,
        merged_patch_id: &str,
    ) -> Result<PatchOutcome, ServiceError> {
        if !valid_key_id(merged_patch_id) {
            return Err(ServiceError::Policy(PolicyError::ForbiddenContent));
        }
        self.authorize(context, &patch.owner_ref, &patch.acting_principal)?;
        scan_for_forbidden_content(&patch)?;
        patch.validate()?;
        let scope = scope_for_owner(&patch.owner_ref)?;
        let location = RecipeLocation {
            scope: scope.clone(),
            recipe_id: patch.recipe_id.clone(),
        };
        let expected = self.backend.current(&location)?;
        let store = self
            .stores
            .get(&scope)
            .ok_or(ServiceError::Model(PatchError::UnknownRecipe))?;
        let commit_patch = match store.conflict_candidate(&patch, merged_patch_id) {
            Ok(candidate) => candidate
                .merged
                .map(|mut merged| {
                    // The caller's actor remains the actor recorded for the
                    // merged intent; merge_disjoint_fields supplies the
                    // canonical operation set and fresh base below.
                    merged.acting_principal = patch.acting_principal.clone();
                    merged.review = patch.review.clone();
                    merged
                })
                .unwrap_or_else(|| patch.clone()),
            Err(PatchError::NotConflicted) => patch.clone(),
            Err(_) => patch.clone(),
        };
        let mut staged_stores = self.stores.clone();
        let outcome = staged_stores
            .entry(scope.clone())
            .or_default()
            .apply_patch_with_narrow_merge(&patch, merged_patch_id);
        match outcome {
            Ok(PatchOutcome::Applied {
                recipe,
                visual_changed,
            }) => {
                commit_recipe(
                    &mut self.backend,
                    &location,
                    &recipe,
                    Some(&commit_patch),
                    expected,
                    CurrentPointer {
                        revision: recipe.revision,
                        recipe_digest: recipe.digest.clone(),
                    },
                )?;
                self.stores = staged_stores;
                Ok(PatchOutcome::Applied {
                    recipe,
                    visual_changed,
                })
            }
            Ok(already @ PatchOutcome::AlreadyApplied(_)) => Ok(already),
            Err(PatchError::Conflict { .. }) => {
                let store = self
                    .stores
                    .get(&scope)
                    .ok_or(ServiceError::Model(PatchError::UnknownRecipe))?;
                let candidate = conflict_candidate(store, &patch, merged_patch_id)?;
                self.backend.retain_conflict(
                    &location,
                    ConflictRecord {
                        current_revision: candidate.current.revision,
                        current_recipe_digest: candidate.current.digest.clone(),
                        incoming_patch_id: patch.patch_id.clone(),
                        incoming_base_revision: patch.base_revision,
                        incoming_base_digest: patch.base_digest.clone(),
                    },
                );
                Err(ServiceError::Conflict(Box::new(report(candidate, patch))))
            }
            Err(error) => Err(error.into()),
        }
    }

    pub fn retained_conflict(
        &self,
        owner_scope: &str,
        recipe_id: &str,
        incoming_patch_id: &str,
    ) -> Option<ConflictRecord> {
        self.backend
            .conflict(
                &RecipeLocation {
                    scope: owner_scope.to_owned(),
                    recipe_id: recipe_id.to_owned(),
                },
                incoming_patch_id,
            )
            .cloned()
    }

    pub fn rollback(
        &mut self,
        context: &OwnerContext,
        request: RollbackRequest,
        patch_id: &str,
    ) -> Result<Recipe, ServiceError> {
        self.authorize(context, &request.owner_ref, &request.acting_principal)?;
        if !valid_key_id(patch_id) || !valid_key_id(&request.recipe_id) {
            return Err(ServiceError::Policy(PolicyError::ForbiddenContent));
        }
        let scope = scope_for_owner(&request.owner_ref)?;
        let location = RecipeLocation {
            scope: scope.clone(),
            recipe_id: request.recipe_id.clone(),
        };
        let expected = self.backend.current(&location)?;
        let store = self
            .stores
            .get(&scope)
            .ok_or(ServiceError::Model(PatchError::UnknownRecipe))?;
        let current = store
            .get(&request.recipe_id)
            .cloned()
            .ok_or(ServiceError::Model(PatchError::UnknownRecipe))?;
        let target = store
            .revision(&request.recipe_id, request.target_revision)
            .cloned()
            .ok_or(ServiceError::Model(PatchError::UnknownRecipeRevision))?;
        let restore_patch = Patch::new(
            request.owner_ref.clone(),
            request.acting_principal.clone(),
            request.reviewer.clone(),
            request.receipt.clone(),
            patch_id,
            request.recipe_id.clone(),
            current.revision,
            current.digest.clone(),
            format!("rollback to revision {}", request.target_revision),
            vec![PatchOp::RestoreRevision {
                target_revision: target.revision,
                target_digest: target.digest.clone(),
            }],
        )?;
        let mut staged_stores = self.stores.clone();
        let recipe = staged_stores
            .entry(scope.clone())
            .or_default()
            .rollback(request, patch_id)?;
        if recipe.revision == current.revision && recipe.digest == current.digest {
            return Ok(recipe);
        }
        commit_recipe(
            &mut self.backend,
            &location,
            &recipe,
            Some(&restore_patch),
            expected,
            CurrentPointer {
                revision: recipe.revision,
                recipe_digest: recipe.digest.clone(),
            },
        )?;
        self.stores = staged_stores;
        Ok(recipe)
    }

    pub fn current_recipe(
        &self,
        context: &OwnerContext,
        recipe_id: &str,
    ) -> Result<Option<Recipe>, ServiceError> {
        let scope = self.require_recipe_scope(context, recipe_id)?;
        Ok(self
            .stores
            .get(&scope)
            .and_then(|store| store.get(recipe_id).cloned()))
    }

    pub fn revision_recipe(
        &self,
        context: &OwnerContext,
        recipe_id: &str,
        revision: u64,
    ) -> Result<Option<Recipe>, ServiceError> {
        let scope = self.require_recipe_scope(context, recipe_id)?;
        Ok(self
            .stores
            .get(&scope)
            .and_then(|store| store.revision(recipe_id, revision).cloned()))
    }

    pub fn save_activity(
        &mut self,
        context: &OwnerContext,
        activity: Activity,
        expected_digest: Option<&str>,
    ) -> Result<Activity, ServiceError> {
        self.authorize(context, &activity.owner_ref, &context.acting_principal)?;
        if !valid_key_id(&activity.activity_id) {
            return Err(ServiceError::Policy(PolicyError::ForbiddenContent));
        }
        let mut durable = activity;
        durable.current_surface = None;
        durable.validate()?;
        scan_for_forbidden_content(&durable)?;
        if durable.schema != ACTIVITY_SCHEMA {
            return Err(ServiceError::CorruptState);
        }
        let scope = scope_for_owner(&durable.owner_ref)?;
        let current_pointer = self.backend.activity_pointer(&scope, &durable.activity_id);
        let expected = if let Some(base) = expected_digest {
            if current_pointer
                .as_ref()
                .is_some_and(|pointer| pointer.activity_digest == base)
            {
                Some(ActivityPointer {
                    activity_digest: base.to_owned(),
                })
            } else {
                let current = self.read_activity_pointer(current_pointer)?;
                return Err(ServiceError::ActivityConflict(Box::new(
                    ActivityConflictReport {
                        current,
                        incoming: durable,
                    },
                )));
            }
        } else if current_pointer.is_some() {
            let current = self.read_activity_pointer(current_pointer)?;
            return Err(ServiceError::ActivityConflict(Box::new(
                ActivityConflictReport {
                    current,
                    incoming: durable,
                },
            )));
        } else {
            None
        };
        let bytes = canonical(&durable)?;
        let activity_digest = digest(&durable)?;
        self.backend
            .save_activity(
                &scope,
                &durable.activity_id,
                &activity_digest,
                &bytes,
                expected.as_ref(),
            )
            .map_err(|error| match error {
                BackendError::CasMismatch => {
                    let pointer = self.backend.activity_pointer(&scope, &durable.activity_id);
                    let current = self.read_activity_pointer(pointer).ok().flatten();
                    ServiceError::ActivityConflict(Box::new(ActivityConflictReport {
                        current,
                        incoming: durable.clone(),
                    }))
                }
                other => other.into(),
            })?;
        Ok(durable)
    }

    pub fn read_activity(
        &self,
        context: &OwnerContext,
        activity_id: &str,
    ) -> Result<Option<Activity>, ServiceError> {
        if !valid_key_id(activity_id) {
            return Err(ServiceError::Policy(PolicyError::ForbiddenContent));
        }
        self.authorize(context, &context.owner, &context.acting_principal)?;
        let scope = scope_for_owner(&context.owner)?;
        let pointer = self.backend.activity_pointer(&scope, activity_id);
        self.read_activity_pointer(pointer)
    }

    fn read_activity_pointer(
        &self,
        pointer: Option<ActivityPointer>,
    ) -> Result<Option<Activity>, ServiceError> {
        match pointer {
            None => Ok(None),
            Some(pointer) => {
                let bytes = self.backend.activity_bytes(&pointer)?;
                let activity: Activity = parse_canonical(&bytes)?;
                if activity.schema != ACTIVITY_SCHEMA
                    || activity.current_surface.is_some()
                    || digest(&activity)? != pointer.activity_digest
                {
                    return Err(ServiceError::CorruptState);
                }
                Ok(Some(activity))
            }
        }
    }

    fn authorize(
        &self,
        context: &OwnerContext,
        owner: &capsule_surface_model::activity::OpaqueOwnerRef,
        actor: &capsule_surface_model::activity::OpaquePrincipalRef,
    ) -> Result<(), ServiceError> {
        if owner != &context.owner || actor != &context.acting_principal {
            return Err(ServiceError::Policy(PolicyError::AuthorizationDenied));
        }
        self.grants
            .authorize(&context.grant, &context.owner, &context.acting_principal)
            .map_err(ServiceError::Policy)
    }

    fn require_recipe_scope(
        &self,
        context: &OwnerContext,
        recipe_id: &str,
    ) -> Result<String, ServiceError> {
        if !valid_key_id(recipe_id) {
            return Err(ServiceError::Policy(PolicyError::ForbiddenContent));
        }
        self.authorize(context, &context.owner, &context.acting_principal)?;
        scope_for_owner(&context.owner)
    }
}

pub fn transition_recipe_owner(
    source: &Recipe,
    destination: capsule_surface_model::activity::OpaqueOwnerRef,
) -> Result<Recipe, ServiceError> {
    source.validate()?;
    scan_for_forbidden_content(source)?;
    let mut destination_recipe = Recipe::new(
        destination,
        source.recipe_id.clone(),
        source.theme_id.clone(),
        source.root.clone(),
    )?;
    destination_recipe.metadata = source.metadata.clone();
    destination_recipe.refresh_after_declared_change()?;
    scan_for_forbidden_content(&destination_recipe)?;
    Ok(destination_recipe)
}

pub fn transition_activity_owner(
    source: &Activity,
    destination: capsule_surface_model::activity::OpaqueOwnerRef,
) -> Result<Activity, ServiceError> {
    source.validate()?;
    scan_for_forbidden_content(source)?;
    let destination_activity = Activity::new(
        source.activity_id.clone(),
        destination,
        source.title.clone(),
        source.recipe_id.clone(),
    )?;
    scan_for_forbidden_content(&destination_activity)?;
    Ok(destination_activity)
}

fn commit_recipe(
    backend: &mut FixtureBackend,
    location: &RecipeLocation,
    recipe: &Recipe,
    patch: Option<&Patch>,
    expected: Option<CurrentPointer>,
    next: CurrentPointer,
) -> Result<(), ServiceError> {
    let recipe_bytes = canonical(recipe)?;
    let envelope = RevisionEnvelope {
        recipe: recipe.clone(),
        patch: patch.cloned(),
    };
    backend
        .put_object_once(location, &recipe.digest, &recipe_bytes)
        .map_err(commit_error)?;
    backend
        .put_revision_once(location, recipe.revision, &envelope)
        .map_err(commit_error)?;
    backend
        .cas_current(location, expected.as_ref(), next)
        .map_err(commit_error)?;
    if let Some(patch) = patch {
        backend
            .put_journal_once(
                location,
                JournalRecord {
                    patch_id: patch.patch_id.clone(),
                    recipe_id: patch.recipe_id.clone(),
                    revision: recipe.revision,
                    recipe_digest: recipe.digest.clone(),
                    intent_digest: patch.intent_digest()?,
                },
            )
            .map_err(commit_error)?;
    }
    Ok(())
}

fn commit_error(error: BackendError) -> ServiceError {
    match error {
        BackendError::Crash(stage) => ServiceError::Crash(stage),
        other => ServiceError::Backend(other),
    }
}

pub fn scope_for_owner(
    owner: &capsule_surface_model::activity::OpaqueOwnerRef,
) -> Result<String, ServiceError> {
    scan_for_forbidden_content(owner)?;
    let bytes = canonical(owner)?;
    Ok(scope_key(&bytes))
}

fn report(candidate: ConflictCandidate, incoming: Patch) -> ConflictReport {
    ConflictReport {
        current: candidate.current,
        incoming,
        merged: candidate.merged,
    }
}

fn conflict_candidate(
    store: &RecipeStore,
    incoming: &Patch,
    merged_patch_id: &str,
) -> Result<ConflictCandidate, ServiceError> {
    match store.conflict_candidate(incoming, merged_patch_id) {
        Ok(candidate) => Ok(candidate),
        Err(PatchError::DisjointFieldMergeRejected) => {
            let current = store
                .get(&incoming.recipe_id)
                .cloned()
                .ok_or(ServiceError::Model(PatchError::UnknownRecipe))?;
            Ok(ConflictCandidate {
                current,
                incoming: incoming.clone(),
                merged: None,
            })
        }
        Err(error) => Err(error.into()),
    }
}
