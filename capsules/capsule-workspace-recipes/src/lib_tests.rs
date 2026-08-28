use super::backend::{FixtureBackend, RecipeLocation};
use super::policy::{
    GrantDirectory, GrantProof, GrantRecord, OwnerContext, PolicyError, scan_for_forbidden_content,
    valid_key_id,
};
use super::service::{
    CommitStage, RecipeService, ServiceError, transition_activity_owner, transition_recipe_owner,
};
use capsule_surface_model::activity::{OpaqueOwnerRef, OpaquePrincipalRef};
use capsule_surface_model::canonical::{
    MAX_CANONICAL_DOCUMENT_BYTES, canonical_bytes, canonical_string, parse_canonical,
};
use capsule_surface_model::components::PropValue;
use capsule_surface_model::fixtures::{fixture_activity, fixture_recipe, fixture_root, reviewer};
use capsule_surface_model::recipe::{Activity, Recipe, RecipeValidationError, SurfacePointer};
use capsule_surface_model::store::{
    Patch, PatchError, PatchOp, PatchOutcome, RecipeStore, RollbackRequest,
};
use capsule_surface_model::{Binding, CachePolicy, DataBinding, FreshnessPolicy};
use serde_json::json;
use std::collections::BTreeMap;

fn context_for(
    owner: OpaqueOwnerRef,
    acting_principal: OpaquePrincipalRef,
) -> (OwnerContext, GrantDirectory) {
    let grant_id = "grant-1".to_owned();
    let record = GrantRecord {
        owner: owner.clone(),
        authorized_actor: acting_principal.clone(),
        generation: 1,
        revoked: false,
    };
    let mut grants = GrantDirectory::default();
    grants.insert(grant_id.clone(), record);
    (
        OwnerContext {
            owner,
            acting_principal,
            grant: GrantProof {
                grant_id,
                generation: 1,
            },
        },
        grants,
    )
}

fn recipe_for(owner: OpaqueOwnerRef, recipe_id: &str) -> Recipe {
    Recipe::new(owner, recipe_id, "fixture-theme", fixture_root()).expect("fixture recipe")
}

fn patch_for(
    recipe: &Recipe,
    patch_id: &str,
    actor_ref: OpaquePrincipalRef,
    operation: PatchOp,
) -> Patch {
    Patch::new(
        recipe.owner_ref.clone(),
        actor_ref,
        reviewer(),
        format!("receipt:{patch_id}"),
        patch_id,
        recipe.recipe_id.clone(),
        recipe.revision,
        recipe.digest.clone(),
        "reviewed fixture update",
        vec![operation],
    )
    .expect("valid patch")
}

fn current_recipe(service: &RecipeService, context: &OwnerContext, recipe_id: &str) -> Recipe {
    service
        .current_recipe(context, recipe_id)
        .expect("current recipe read")
        .expect("recipe exists")
}

#[test]
fn owner_variants_require_an_external_grant_and_keep_actor_separate() {
    let cases = [
        (
            OpaqueOwnerRef::User("user:fixture".to_owned()),
            OpaquePrincipalRef::Agent("agent:user".to_owned()),
        ),
        (
            OpaqueOwnerRef::Principal("principal:fixture".to_owned()),
            OpaquePrincipalRef::Service("service:principal".to_owned()),
        ),
        (
            OpaqueOwnerRef::Fleet("fleet:fixture".to_owned()),
            OpaquePrincipalRef::User("user:fleet-actor".to_owned()),
        ),
    ];

    for (owner, actor_ref) in cases {
        let (context, grants) = context_for(owner.clone(), actor_ref.clone());
        let mut service = RecipeService::open(FixtureBackend::default(), grants).expect("open");
        let recipe = recipe_for(owner.clone(), "shared-id");
        service
            .create_recipe(&context, recipe.clone())
            .expect("create under authorized owner");
        assert_eq!(
            current_recipe(&service, &context, "shared-id").owner_ref,
            owner
        );

        let wrong_owner = OwnerContext {
            owner: OpaqueOwnerRef::Fleet("fleet:other".to_owned()),
            acting_principal: context.acting_principal.clone(),
            grant: context.grant.clone(),
        };
        assert!(matches!(
            service.current_recipe(&wrong_owner, "shared-id"),
            Err(ServiceError::Policy(PolicyError::AuthorizationDenied))
        ));

        let wrong_actor = OwnerContext {
            owner: context.owner.clone(),
            acting_principal: OpaquePrincipalRef::Agent("agent:other".to_owned()),
            grant: context.grant.clone(),
        };
        assert!(matches!(
            service.current_recipe(&wrong_actor, "shared-id"),
            Err(ServiceError::Policy(PolicyError::AuthorizationDenied))
        ));

        let stale_grant = OwnerContext {
            owner: context.owner.clone(),
            acting_principal: context.acting_principal.clone(),
            grant: GrantProof {
                grant_id: context.grant.grant_id.clone(),
                generation: 2,
            },
        };
        assert!(matches!(
            service.current_recipe(&stale_grant, "shared-id"),
            Err(ServiceError::Policy(PolicyError::AuthorizationDenied))
        ));

        service.grants.insert(
            context.grant.grant_id.clone(),
            GrantRecord {
                owner: context.owner.clone(),
                authorized_actor: context.acting_principal.clone(),
                generation: 1,
                revoked: true,
            },
        );
        assert!(matches!(
            service.current_recipe(&context, "shared-id"),
            Err(ServiceError::Policy(PolicyError::AuthorizationDenied))
        ));

        let foreign_recipe = recipe_for(
            OpaqueOwnerRef::Principal("principal:other".to_owned()),
            "foreign",
        );
        assert!(matches!(
            service.create_recipe(&context, foreign_recipe),
            Err(ServiceError::Policy(PolicyError::AuthorizationDenied))
        ));
    }
}

#[test]
fn recipe_ids_are_isolated_by_owner_scope() {
    let owner_a = OpaqueOwnerRef::User("user:one".to_owned());
    let actor_a = OpaquePrincipalRef::Agent("agent:one".to_owned());
    let owner_b = OpaqueOwnerRef::Fleet("fleet:two".to_owned());
    let actor_b = OpaquePrincipalRef::Service("service:two".to_owned());
    let (context_a, mut grants) = context_for(owner_a.clone(), actor_a.clone());
    grants.insert(
        "grant-2",
        GrantRecord {
            owner: owner_b.clone(),
            authorized_actor: actor_b.clone(),
            generation: 1,
            revoked: false,
        },
    );
    let context_b = OwnerContext {
        owner: owner_b.clone(),
        acting_principal: actor_b.clone(),
        grant: GrantProof {
            grant_id: "grant-2".to_owned(),
            generation: 1,
        },
    };
    let mut service = RecipeService::open(FixtureBackend::default(), grants.clone()).expect("open");
    let recipe_a = service
        .create_recipe(&context_a, recipe_for(owner_a.clone(), "same-id"))
        .expect("owner a create");
    let recipe_b = service
        .create_recipe(&context_b, recipe_for(owner_b.clone(), "same-id"))
        .expect("owner b create");
    assert_eq!(current_recipe(&service, &context_a, "same-id"), recipe_a);
    assert_eq!(current_recipe(&service, &context_b, "same-id"), recipe_b);

    let patch_a = patch_for(
        &recipe_a,
        "patch-a",
        actor_a,
        PatchOp::SetTheme {
            theme_id: "theme-a".to_owned(),
        },
    );
    let patch_b = patch_for(
        &recipe_b,
        "patch-b",
        actor_b,
        PatchOp::SetTheme {
            theme_id: "theme-b".to_owned(),
        },
    );
    service
        .apply_patch(&context_a, patch_a)
        .expect("owner a patch");
    service
        .apply_patch(&context_b, patch_b)
        .expect("owner b patch");
    assert_eq!(
        current_recipe(&service, &context_a, "same-id").theme_id,
        "theme-a"
    );
    assert_eq!(
        current_recipe(&service, &context_b, "same-id").theme_id,
        "theme-b"
    );

    let reopened = RecipeService::open(service.backend.clone(), grants).expect("restart");
    assert_eq!(
        current_recipe(&reopened, &context_a, "same-id").theme_id,
        "theme-a"
    );
    assert_eq!(
        current_recipe(&reopened, &context_b, "same-id").theme_id,
        "theme-b"
    );
}

#[test]
fn canonical_commit_reopen_cas_and_idempotency_are_deterministic() {
    let owner = OpaqueOwnerRef::Principal("principal:fixture".to_owned());
    let actor_ref = OpaquePrincipalRef::Agent("agent:fixture".to_owned());
    let (context, grants) = context_for(owner.clone(), actor_ref.clone());
    let mut service = RecipeService::open(FixtureBackend::default(), grants.clone()).expect("open");
    let created = service
        .create_recipe(&context, recipe_for(owner.clone(), "fixture-recipe"))
        .expect("create");
    let patch = patch_for(
        &created,
        "patch-1",
        actor_ref.clone(),
        PatchOp::SetRecipeMetadata {
            key: "mode".to_owned(),
            value: PropValue::Text("focused".to_owned()),
        },
    );
    let applied = service.apply_patch(&context, patch.clone()).expect("apply");
    let applied_recipe = match applied {
        PatchOutcome::Applied { recipe, .. } => recipe,
        PatchOutcome::AlreadyApplied(_) => panic!("first patch was unexpectedly duplicate"),
    };
    assert_eq!(applied_recipe.revision, 2);
    assert_eq!(applied_recipe.parent_revision, Some(1));
    assert_eq!(applied_recipe.parent_digest, created.digest);

    assert!(matches!(
        service.apply_patch(&context, patch.clone()),
        Ok(PatchOutcome::AlreadyApplied(_))
    ));
    let changed_intent = patch_for(&created, "patch-1", actor_ref.clone(), PatchOp::ClearRoot);
    assert!(matches!(
        service.apply_patch(&context, changed_intent),
        Err(ServiceError::Model(PatchError::PatchIdConflict { .. }))
    ));

    let stale = patch_for(
        &created,
        "patch-stale",
        actor_ref.clone(),
        PatchOp::ClearRoot,
    );
    assert!(matches!(
        service.apply_patch(&context, stale.clone()),
        Err(ServiceError::Conflict(_))
    ));
    let scope = super::service::scope_for_owner(&owner).expect("owner scope");
    assert!(
        service
            .retained_conflict(&scope, &created.recipe_id, &stale.patch_id)
            .is_some()
    );

    let location = RecipeLocation {
        scope,
        recipe_id: created.recipe_id.clone(),
    };
    let pointer = service
        .backend
        .current(&location)
        .expect("pointer")
        .expect("current");
    assert_eq!(pointer.revision, 2);
    assert_eq!(pointer.recipe_digest, applied_recipe.digest);
    let envelope = service
        .backend
        .revision_envelope(&location.scope, &location.recipe_id, 2)
        .expect("revision envelope");
    assert_eq!(envelope.recipe, applied_recipe);
    assert_eq!(envelope.patch.expect("patch journal").patch_id, "patch-1");

    let replay = RecipeStore::replay(&canonical_bytes(&created).expect("canonical"), &[patch]);
    assert_eq!(replay.expect("deterministic replay"), applied_recipe);

    let reopened_backend = service.backend.clone();
    let reopened = RecipeService::open(reopened_backend, grants).expect("restart");
    assert_eq!(
        current_recipe(&reopened, &context, &created.recipe_id),
        applied_recipe
    );
    assert_eq!(
        reopened
            .revision_recipe(&context, &created.recipe_id, 1)
            .expect("history read")
            .expect("revision one"),
        created
    );
}

#[test]
fn narrow_merge_records_the_merged_patch_and_rollback_is_append_only() {
    let owner = OpaqueOwnerRef::Principal("principal:fixture".to_owned());
    let actor_ref = OpaquePrincipalRef::Agent("agent:fixture".to_owned());
    let (context, grants) = context_for(owner.clone(), actor_ref.clone());
    let mut service = RecipeService::open(FixtureBackend::default(), grants.clone()).expect("open");
    let created = service
        .create_recipe(&context, recipe_for(owner.clone(), "merge-recipe"))
        .expect("create");
    let first = patch_for(
        &created,
        "first",
        actor_ref.clone(),
        PatchOp::SetRecipeMetadata {
            key: "field-a".to_owned(),
            value: PropValue::Text("a".to_owned()),
        },
    );
    let first_recipe = match service.apply_patch(&context, first).expect("first patch") {
        PatchOutcome::Applied { recipe, .. } => recipe,
        PatchOutcome::AlreadyApplied(_) => panic!("first patch duplicate"),
    };
    let stale = patch_for(
        &created,
        "stale",
        actor_ref.clone(),
        PatchOp::SetRecipeMetadata {
            key: "field-b".to_owned(),
            value: PropValue::Text("b".to_owned()),
        },
    );
    let merged_recipe = match service
        .apply_patch_with_narrow_merge(&context, stale, "merged")
        .expect("narrow merge")
    {
        PatchOutcome::Applied { recipe, .. } => recipe,
        PatchOutcome::AlreadyApplied(_) => panic!("merge duplicate"),
    };
    assert_eq!(merged_recipe.revision, 3);
    assert_eq!(
        merged_recipe.metadata["field-a"],
        PropValue::Text("a".to_owned())
    );
    assert_eq!(
        merged_recipe.metadata["field-b"],
        PropValue::Text("b".to_owned())
    );
    let scope = super::service::scope_for_owner(&owner).expect("owner scope");
    let envelope = service
        .backend
        .revision_envelope(&scope, &created.recipe_id, 3)
        .expect("merged envelope");
    let merged_patch = envelope.patch.expect("merged patch persisted");
    assert_eq!(merged_patch.patch_id, "merged");
    assert_eq!(merged_patch.base_revision, first_recipe.revision);
    assert_eq!(merged_patch.base_digest, first_recipe.digest);
    assert_eq!(merged_patch.acting_principal, actor_ref);

    let overlapping = patch_for(
        &created,
        "overlap",
        actor_ref.clone(),
        PatchOp::SetRecipeMetadata {
            key: "field-a".to_owned(),
            value: PropValue::Text("different".to_owned()),
        },
    );
    assert!(matches!(
        service.apply_patch(&context, overlapping.clone()),
        Err(ServiceError::Conflict(_))
    ));
    assert!(
        service
            .retained_conflict(&scope, &created.recipe_id, &overlapping.patch_id)
            .is_some()
    );

    let request = RollbackRequest {
        owner_ref: owner.clone(),
        recipe_id: created.recipe_id.clone(),
        target_revision: 1,
        acting_principal: context.acting_principal.clone(),
        reviewer: reviewer(),
        receipt: "rollback-review".to_owned(),
    };
    let rolled = service
        .rollback(&context, request.clone(), "rollback-1")
        .expect("rollback");
    assert_eq!(rolled.revision, 4);
    assert_eq!(rolled.parent_revision, Some(3));
    assert_eq!(rolled.root, created.root);
    assert!(
        service
            .revision_recipe(&context, &created.recipe_id, 2)
            .expect("revision two")
            .is_some()
    );
    let repeated = service
        .rollback(&context, request, "rollback-1")
        .expect("idempotent rollback");
    assert_eq!(repeated, rolled);

    let reopened = RecipeService::open(service.backend.clone(), grants).expect("restart");
    assert_eq!(
        current_recipe(&reopened, &context, &created.recipe_id),
        rolled
    );
}

#[test]
fn every_recipe_commit_stage_recovers_without_partial_visibility() {
    let stages = [
        CommitStage::Object,
        CommitStage::Revision,
        CommitStage::Pointer,
        CommitStage::Journal,
    ];
    for stage in stages {
        let owner = OpaqueOwnerRef::Principal(format!("principal:{stage:?}"));
        let actor_ref = OpaquePrincipalRef::Agent(format!("agent:{stage:?}"));
        let (context, grants) = context_for(owner.clone(), actor_ref.clone());
        let mut service =
            RecipeService::open(FixtureBackend::default(), grants.clone()).expect("open");
        let created = service
            .create_recipe(&context, recipe_for(owner.clone(), "crash-recipe"))
            .expect("create");
        let patch = patch_for(
            &created,
            "crash-patch",
            actor_ref,
            PatchOp::SetTheme {
                theme_id: "after-crash".to_owned(),
            },
        );
        service.backend.set_fail_after(stage);
        assert!(matches!(
            service.apply_patch(&context, patch.clone()),
            Err(ServiceError::Crash(found)) if found == stage
        ));

        let mut reopened = RecipeService::open(service.backend.clone(), grants).expect("recover");
        match stage {
            CommitStage::Object | CommitStage::Revision => {
                let retained = current_recipe(&reopened, &context, &created.recipe_id);
                assert_eq!(retained.revision, 1);
                assert_eq!(retained.digest, created.digest);
                assert!(reopened.backend.quarantine_len() >= 1);
                let retried = reopened
                    .apply_patch(&context, patch)
                    .expect("retry after quarantine");
                assert!(matches!(retried, PatchOutcome::Applied { .. }));
            }
            CommitStage::Pointer | CommitStage::Journal => {
                let recovered = current_recipe(&reopened, &context, &created.recipe_id);
                assert_eq!(recovered.revision, 2);
                assert_eq!(recovered.theme_id, "after-crash");
                assert!(matches!(
                    reopened.apply_patch(&context, patch),
                    Ok(PatchOutcome::AlreadyApplied(_))
                ));
            }
        }
    }
}

#[test]
fn activity_persistence_strips_surface_and_uses_digest_cas() {
    let owner = OpaqueOwnerRef::Principal("uid:principal:fixture".to_owned());
    let actor_ref = OpaquePrincipalRef::Agent("uid:agent:fixture".to_owned());
    let (context, grants) = context_for(owner, actor_ref);
    let mut service = RecipeService::open(FixtureBackend::default(), grants).expect("open");

    let mut activity = fixture_activity();
    activity.current_surface = Some(SurfacePointer {
        surface_id: "ephemeral-surface".to_owned(),
        incarnation: 1,
    });
    let saved = service
        .save_activity(&context, activity, None)
        .expect("save activity");
    assert!(saved.current_surface.is_none());
    let scope = super::service::scope_for_owner(&context.owner).expect("owner scope");
    let pointer = service
        .backend
        .activity_pointer(&scope, &saved.activity_id)
        .expect("activity pointer");
    let bytes = service
        .backend
        .activity_bytes(&pointer)
        .expect("activity object");
    let text = String::from_utf8(bytes.clone()).expect("canonical JSON");
    assert!(!text.contains("surface_id"));
    assert!(!text.contains("aos.surface@1"));
    assert_eq!(
        parse_canonical::<Activity>(&bytes).expect("activity parse"),
        saved
    );
    assert_eq!(
        service
            .read_activity(&context, &saved.activity_id)
            .expect("activity read")
            .expect("saved activity"),
        saved
    );

    let conflict = service.save_activity(&context, fixture_activity(), None);
    assert!(matches!(conflict, Err(ServiceError::ActivityConflict(_))));
    let mut changed = fixture_activity();
    changed.title = "Changed title".to_owned();
    let changed = service
        .save_activity(&context, changed, Some(&pointer.activity_digest))
        .expect("CAS update");
    assert_ne!(changed.digest_for_test(), pointer.activity_digest);
    assert!(matches!(
        service.save_activity(&context, fixture_activity(), Some("f".repeat(64).as_str())),
        Err(ServiceError::ActivityConflict(_))
    ));

    let mut bad_binding = fixture_activity();
    bad_binding.bindings.binding = Some(Binding::Data(DataBinding {
        owner: context.owner.clone(),
        kernel_object_id: "home://secret".to_owned(),
        grant_id: Some("grant-1".to_owned()),
        content_hash: "f".repeat(64),
        mime: "application/json".to_owned(),
        freshness: FreshnessPolicy::Always,
        cache: CachePolicy::NoStore,
    }));
    assert!(service.save_activity(&context, bad_binding, None).is_err());
}

#[test]
fn canonical_and_policy_boundaries_reject_duplicates_paths_secrets_and_bad_lineage() {
    let activity_json = canonical_string(&fixture_activity()).expect("canonical activity");
    let duplicate = activity_json.replacen(
        "\"schema\":\"aos.activity@1\"",
        "\"schema\":\"aos.activity@1\",\"schema\":\"aos.activity@1\"",
        1,
    );
    assert!(parse_canonical::<Activity>(duplicate.as_bytes()).is_err());
    assert!(parse_canonical::<Activity>(&vec![b' '; MAX_CANONICAL_DOCUMENT_BYTES + 1]).is_err());
    let unknown = format!(
        "{},\"authority\":\"grant-all\"}}",
        activity_json.trim_end_matches('}')
    );
    assert!(parse_canonical::<Activity>(unknown.as_bytes()).is_err());

    assert!(!valid_key_id("../escape"));
    assert!(!valid_key_id("a/b"));
    assert!(!valid_key_id("a\0b"));
    assert!(valid_key_id("patch-1:ok_id"));
    assert!(
        Recipe::new(
            OpaqueOwnerRef::Principal("home://owner".to_owned()),
            "recipe",
            "theme",
            fixture_root(),
        )
        .is_err()
    );

    let mut forbidden = BTreeMap::new();
    forbidden.insert("host_path".to_owned(), "home://secret".to_owned());
    assert!(scan_for_forbidden_content(&forbidden).is_err());

    let mut malformed = fixture_recipe();
    malformed.parent_revision = Some(1);
    malformed.parent_digest = "f".repeat(64);
    assert_eq!(malformed.validate(), Err(RecipeValidationError::ParentLink));

    let owner = OpaqueOwnerRef::Principal("principal:fixture".to_owned());
    let actor_ref = OpaquePrincipalRef::Agent("agent:fixture".to_owned());
    let (context, grants) = context_for(owner.clone(), actor_ref.clone());
    let mut service = RecipeService::open(FixtureBackend::default(), grants).expect("open");
    let created = service
        .create_recipe(&context, recipe_for(owner, "forbidden-recipe"))
        .expect("create");
    let forbidden_patch = patch_for(
        &created,
        "forbidden-patch",
        actor_ref,
        PatchOp::SetTheme {
            theme_id: "home://theme".to_owned(),
        },
    );
    assert!(matches!(
        service.apply_patch(&context, forbidden_patch),
        Err(ServiceError::Policy(PolicyError::ForbiddenContent))
    ));
}

#[test]
fn owner_transition_is_a_destination_copy_without_source_capabilities() {
    let source_recipe = fixture_recipe();
    let destination = OpaqueOwnerRef::Fleet("fleet:destination".to_owned());
    let copied_recipe =
        transition_recipe_owner(&source_recipe, destination.clone()).expect("copy recipe");
    assert_eq!(copied_recipe.owner_ref, destination);
    assert_eq!(copied_recipe.revision, 1);
    assert!(copied_recipe.parent_revision.is_none());
    assert!(copied_recipe.bindings.binding.is_none());
    assert!(copied_recipe.bindings.rhai.is_none());

    let source_activity = fixture_activity();
    let copied_activity = transition_activity_owner(
        &source_activity,
        OpaqueOwnerRef::User("user:destination".to_owned()),
    )
    .expect("copy activity");
    assert_eq!(
        copied_activity.owner_ref,
        OpaqueOwnerRef::User("user:destination".to_owned())
    );
    assert!(copied_activity.bindings.binding.is_none());
    assert!(copied_activity.current_surface.is_none());

    let value = json!({"path":"/Users/owner/.secret","token":"bearer secret"});
    assert!(scan_for_forbidden_content(&value).is_err());
}

trait ActivityDigestForTest {
    fn digest_for_test(&self) -> String;
}

impl ActivityDigestForTest for Activity {
    fn digest_for_test(&self) -> String {
        super::documents::digest(self).expect("activity digest")
    }
}
