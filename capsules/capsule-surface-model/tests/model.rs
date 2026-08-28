use capsule_surface_model::a2ui::{
    A2uiAdapter, A2uiError, A2uiVersion, CreateSurfaceMessage, DeleteSurfaceMessage, ExportCatalog,
    NonStorageCall, StorageImport, UpdateComponentsMessage, UpdateDataModelMessage,
    export_projection,
};
use capsule_surface_model::activity::OpaqueOwnerRef;
use capsule_surface_model::canonical::{CanonicalJson, canonical_string, parse_canonical};
use capsule_surface_model::components::{ComponentKind, PropValue, SemanticNode, StateSet};
use capsule_surface_model::error::{ExtensionError, Extensions};
use capsule_surface_model::fixtures::{
    actor, fixture_activity, fixture_recipe, fixture_root, owner, reviewer,
};
use capsule_surface_model::recipe::{Recipe, RecipeValidationError};
use capsule_surface_model::store::{Patch, PatchError, PatchOp, RecipeStore, RollbackRequest};
use capsule_surface_model::{Activity, Surface};
use serde_json::json;
use std::collections::BTreeMap;

fn canonical(value: serde_json::Value) -> CanonicalJson {
    serde_json::from_value(value).expect("canonical JSON value")
}

fn reviewed_patch(recipe: &Recipe, id: &str, op: PatchOp) -> Patch {
    Patch::new(
        owner(),
        actor(),
        reviewer(),
        format!("receipt:{id}"),
        id,
        recipe.recipe_id.clone(),
        recipe.revision,
        recipe.digest.clone(),
        "reviewed fixture",
        vec![op],
    )
    .expect("valid patch")
}

#[test]
fn accepted_patch_is_cas_idempotent_and_parent_linked() {
    let recipe = fixture_recipe();
    let mut store = RecipeStore::new();
    store.insert(recipe.clone()).expect("insert");
    let patch = reviewed_patch(
        &recipe,
        "patch-1",
        PatchOp::SetState {
            node_id: recipe.root.id,
            state: StateSet::empty().with(StateSet::FOCUS, true),
        },
    );
    let applied = store.apply_patch(&patch).expect("applied");
    let replay = store.apply_patch(&patch).expect("idempotent");
    assert!(matches!(
        applied,
        capsule_surface_model::store::PatchOutcome::Applied {
            visual_changed: true,
            ..
        }
    ));
    assert!(matches!(
        replay,
        capsule_surface_model::store::PatchOutcome::AlreadyApplied(_)
    ));
    let current = store.get(&recipe.recipe_id).unwrap();
    assert_eq!(current.revision, 2);
    assert_eq!(current.parent_revision, Some(1));
    assert_eq!(current.parent_digest, recipe.digest);
    assert_ne!(current.digest, recipe.digest);

    let stale = reviewed_patch(&recipe, "patch-stale", PatchOp::ClearRoot);
    assert!(matches!(
        store.apply_patch(&stale),
        Err(PatchError::Conflict {
            current_revision: 2,
            ..
        })
    ));
}

#[test]
fn patch_id_conflict_self_review_cross_owner_and_bounds() {
    let recipe = fixture_recipe();
    let mut store = RecipeStore::new();
    store.insert(recipe.clone()).expect("insert");
    let accepted = reviewed_patch(
        &recipe,
        "same-id",
        PatchOp::SetState {
            node_id: recipe.root.id,
            state: StateSet::empty().with(StateSet::FOCUS, true),
        },
    );
    store.apply_patch(&accepted).expect("accepted");
    let changed = reviewed_patch(
        &recipe,
        "same-id",
        PatchOp::SetState {
            node_id: recipe.root.id,
            state: StateSet::empty().with(StateSet::DISABLED, true),
        },
    );
    assert!(matches!(
        store.apply_patch(&changed),
        Err(PatchError::PatchIdConflict { .. })
    ));

    let mut self_review = reviewed_patch(&recipe, "self", PatchOp::ClearRoot);
    self_review.review.reviewer = self_review.acting_principal.clone();
    assert_eq!(self_review.validate(), Err(PatchError::SelfApproved));

    let foreign = Patch::new(
        OpaqueOwnerRef::User("uid:user:other".to_owned()),
        actor(),
        reviewer(),
        "receipt",
        "foreign",
        recipe.recipe_id.clone(),
        recipe.revision,
        recipe.digest.clone(),
        "cross owner",
        vec![PatchOp::ClearRoot],
    )
    .expect("patch shape");
    assert_eq!(store.apply_patch(&foreign), Err(PatchError::OwnerMismatch));

    let mut oversized = reviewed_patch(&recipe, "oversized", PatchOp::ClearRoot);
    oversized.operations = (0..=capsule_surface_model::store::MAX_PATCH_OPS)
        .map(|_| PatchOp::ClearRoot)
        .collect();
    assert_eq!(oversized.validate(), Err(PatchError::TooManyOperations));
}

#[test]
fn rollback_appends_pointer_and_keeps_history() {
    let recipe = fixture_recipe();
    let mut store = RecipeStore::new();
    store.insert(recipe.clone()).expect("insert");
    store
        .apply_patch(&reviewed_patch(&recipe, "one", PatchOp::ClearRoot))
        .expect("one");
    store
        .apply_patch(&reviewed_patch(
            store.get(recipe.recipe_id.as_str()).unwrap(),
            "two",
            PatchOp::SetTheme {
                theme_id: "second-theme".to_owned(),
            },
        ))
        .expect("two");
    let request = RollbackRequest {
        owner_ref: owner(),
        recipe_id: recipe.recipe_id.clone(),
        target_revision: 1,
        acting_principal: actor(),
        reviewer: reviewer(),
        receipt: "rollback-review".to_owned(),
    };
    let rolled = store
        .rollback(request.clone(), "rollback-1")
        .expect("rollback");
    assert_eq!(rolled.revision, 4);
    assert_eq!(rolled.parent_revision, Some(3));
    assert_eq!(rolled.root, recipe.root);
    assert_eq!(rolled.theme_id, recipe.theme_id);
    assert!(store.revision(recipe.recipe_id.as_str(), 2).is_some());
    assert!(store.revision(recipe.recipe_id.as_str(), 1).is_some());
    let replayed = store
        .rollback(request, "rollback-1")
        .expect("idempotent rollback");
    assert_eq!(replayed.revision, 4);
    assert_eq!(replayed.digest, rolled.digest);
}

#[test]
fn conflict_merges_only_disjoint_fields() {
    let mut recipe = fixture_recipe();
    recipe.metadata.clear();
    let mut store = RecipeStore::new();
    store.insert(recipe.clone()).expect("insert");
    let accepted = reviewed_patch(
        &recipe,
        "accepted",
        PatchOp::SetRecipeMetadata {
            key: "field-a".to_owned(),
            value: PropValue::Text("a".to_owned()),
        },
    );
    store.apply_patch(&accepted).expect("accepted");
    let stale = reviewed_patch(
        &recipe,
        "stale",
        PatchOp::SetRecipeMetadata {
            key: "field-b".to_owned(),
            value: PropValue::Text("b".to_owned()),
        },
    );
    let candidate = store
        .conflict_candidate(&stale, "merged")
        .expect("candidate");
    assert!(candidate.merged.is_some());
    let outcome = store
        .apply_patch_with_narrow_merge(&stale, "merged")
        .expect("disjoint merge");
    assert!(matches!(
        outcome,
        capsule_surface_model::store::PatchOutcome::Applied { .. }
    ));

    let conflicting = reviewed_patch(
        &recipe,
        "conflicting",
        PatchOp::SetRecipeMetadata {
            key: "field-a".to_owned(),
            value: PropValue::Text("different".to_owned()),
        },
    );
    assert!(matches!(
        store.conflict_candidate(&conflicting, "merged-2"),
        Err(PatchError::DisjointFieldMergeRejected)
    ));
}

#[test]
fn canonical_json_rejects_duplicate_oversized_and_unknown_input() {
    let activity_json =
        capsule_surface_model::canonical::canonical_string(&fixture_activity()).expect("canonical");
    let parsed: Activity = parse_canonical(activity_json.as_bytes()).expect("canonical activity");
    assert_eq!(parsed.schema, "aos.activity@1");

    let duplicate = activity_json.replace(
        "\"schema\":\"aos.activity@1\"",
        "\"schema\":\"aos.activity@1\",\"schema\":\"aos.activity@1\"",
    );
    assert!(parse_canonical::<Activity>(duplicate.as_bytes()).is_err());
    assert!(
        parse_canonical::<Activity>(
            &[b'{'; capsule_surface_model::canonical::MAX_CANONICAL_DOCUMENT_BYTES + 1]
        )
        .is_err()
    );

    let malformed = format!("{activity_json},extra");
    assert!(parse_canonical::<Activity>(malformed.as_bytes()).is_err());
}

#[test]
fn hostile_identity_and_ownership_payloads_fail_closed() {
    let recipe = fixture_recipe();
    let path_owner = Patch::new(
        OpaqueOwnerRef::Principal("../../home/owner".to_owned()),
        actor(),
        reviewer(),
        "receipt",
        "path-owner",
        recipe.recipe_id.clone(),
        recipe.revision,
        recipe.digest.clone(),
        "path substitution",
        vec![PatchOp::ClearRoot],
    );
    assert!(path_owner.is_err());

    let mut path_binding = fixture_recipe();
    path_binding.bindings.binding = Some(capsule_surface_model::Binding::Data(
        capsule_surface_model::DataBinding {
            owner: owner(),
            kernel_object_id: "$HOME/secret".to_owned(),
            grant_id: None,
            content_hash: "a".repeat(64),
            mime: "application/json".to_owned(),
            freshness: capsule_surface_model::FreshnessPolicy::Always,
            cache: capsule_surface_model::CachePolicy::NoStore,
        },
    ));
    assert_eq!(
        path_binding.validate(),
        Err(RecipeValidationError::Identity)
    );

    let unknown = r#"{"schema":"aos.activity@1","owner_ref":{"kind":"user","id":"u"},"authority":"grant-all"}"#;
    assert!(parse_canonical::<Activity>(unknown.as_bytes()).is_err());
}

#[test]
fn replay_is_deterministic() {
    let recipe = fixture_recipe();
    let mut store = RecipeStore::new();
    store.insert(recipe.clone()).expect("insert");
    let first = reviewed_patch(
        &recipe,
        "one",
        PatchOp::SetRecipeMetadata {
            key: "replay".to_owned(),
            value: PropValue::Bool(true),
        },
    );
    let first_recipe = match store.apply_patch(&first).expect("one") {
        capsule_surface_model::store::PatchOutcome::Applied { recipe, .. } => recipe,
        _ => unreachable!(),
    };
    let second = reviewed_patch(
        &first_recipe,
        "two",
        PatchOp::SetState {
            node_id: recipe.root.id,
            state: StateSet::empty().with(StateSet::DISABLED, true),
        },
    );
    store.apply_patch(&second).expect("two");
    let patches = vec![first, second];
    let recipe_json =
        capsule_surface_model::canonical::canonical_bytes(&recipe).expect("canonical");
    let replay = RecipeStore::replay(&recipe_json, &patches).expect("replay");
    let live = store.get(recipe.recipe_id.as_str()).unwrap();
    assert_eq!(replay, live.clone());
    assert_eq!(
        capsule_surface_model::canonical::canonical_string(&replay).unwrap(),
        capsule_surface_model::canonical::canonical_string(live).unwrap()
    );
}

#[test]
fn unknown_extensions_are_preserved_with_explicit_fallback() {
    let mut recipe = fixture_recipe();
    recipe.bindings.extensions = Extensions::new(vec![(
        "vendor.example/extension".to_owned(),
        canonical(json!({"opaque":[1,2,3]})),
    )])
    .expect("extension");
    recipe
        .refresh_after_declared_change()
        .expect("refresh extension");
    let json = capsule_surface_model::canonical::canonical_string(&recipe).unwrap();
    let parsed: Recipe = parse_canonical(json.as_bytes()).unwrap();
    let preserved = parsed
        .bindings
        .extensions
        .get("vendor.example/extension")
        .expect("unknown extension is preserved verbatim");
    assert_eq!(preserved, &canonical(json!({"opaque":[1,2,3]})));
    assert!(matches!(
        parsed.bindings.extensions.get("vendor.example/missing"),
        Err(ExtensionError::Unsupported(_))
    ));
    assert!(parsed.downgrade_boundary("aos.recipe@0.9").is_err());
    let mut plain = fixture_recipe();
    plain.bindings.binding = None;
    plain.bindings.rhai = None;
    plain
        .refresh_after_declared_change()
        .expect("refresh downgradable fixture");
    assert!(plain.downgrade_boundary("aos.recipe@0.9").is_ok());
    assert_eq!(
        plain.downgrade_boundary("aos.recipe@9.9"),
        Err(RecipeValidationError::Schema)
    );
}

#[test]
fn a2ui_imports_create_storage_documents_but_not_authority() {
    let recipe = fixture_recipe();
    let create = CreateSurfaceMessage {
        version: A2uiVersion::V0_9,
        surface_id: "surface-1".to_owned(),
        recipe_id: recipe.recipe_id.clone(),
        root: fixture_root(),
    };
    let imported =
        A2uiAdapter::import_create_surface(&create, owner(), actor(), "fixture-theme".to_owned())
            .expect("create recipe");
    assert_eq!(imported.owner_ref, owner());

    let update = UpdateComponentsMessage {
        version: A2uiVersion::V1_0,
        surface_id: "surface-1".to_owned(),
        recipe_id: recipe.recipe_id.clone(),
        root: fixture_root(),
    };
    let patch = A2uiAdapter::import_update_components(
        &update,
        owner(),
        actor(),
        reviewer(),
        "a2ui-review".to_owned(),
        &recipe,
    )
    .expect("component patch");
    assert!(matches!(
        patch.operations.first(),
        Some(PatchOp::ReplaceRoot { .. })
    ));

    let non_storage = NonStorageCall::AgentFunctionCall {
        function: "grant_capability".to_owned(),
        arguments: canonical(json!({"scope":"all"})),
    };
    assert_eq!(
        A2uiAdapter::reject_non_storage(&non_storage),
        Err(A2uiError::NonStorage)
    );

    let delete = DeleteSurfaceMessage {
        version: A2uiVersion::V1_0,
        surface_id: "surface-1".to_owned(),
        recipe_id: recipe.recipe_id.clone(),
    };
    assert!(matches!(
        A2uiAdapter::import_delete_surface(&delete),
        Ok(capsule_surface_model::a2ui::ImportedDocument::EphemeralDelete)
    ));
}

#[test]
fn a2ui_data_model_rejects_authority_and_is_declared_lossy() {
    let recipe = fixture_recipe();
    let authority = UpdateDataModelMessage {
        version: A2uiVersion::V1_0,
        surface_id: "surface-1".to_owned(),
        recipe_id: recipe.recipe_id.clone(),
        data: canonical(json!({"grant":"all"})),
    };
    assert_eq!(
        A2uiAdapter::import_update_data_model(
            &authority,
            owner(),
            actor(),
            reviewer(),
            "review".to_owned(),
            &recipe,
        ),
        Err(A2uiError::AuthorityField)
    );
    let nested_authority = UpdateDataModelMessage {
        version: A2uiVersion::V1_0,
        surface_id: "surface-1".to_owned(),
        recipe_id: recipe.recipe_id.clone(),
        data: canonical(json!({"metadata":{"owner":"spoofed"}})),
    };
    assert_eq!(
        A2uiAdapter::import_update_data_model(
            &nested_authority,
            owner(),
            actor(),
            reviewer(),
            "review".to_owned(),
            &recipe,
        ),
        Err(A2uiError::AuthorityField)
    );

    let supported = UpdateDataModelMessage {
        version: A2uiVersion::V0_9,
        surface_id: "surface-1".to_owned(),
        recipe_id: recipe.recipe_id.clone(),
        data: canonical(json!({"title":"fixture","count":2})),
    };
    let patch = A2uiAdapter::import_update_data_model(
        &supported,
        owner(),
        actor(),
        reviewer(),
        "review".to_owned(),
        &recipe,
    )
    .expect("data patch");
    assert_eq!(patch.operations.len(), 2);

    let unknown = r#"{"message":"create-surface","authority":"root"}"#;
    assert!(parse_canonical::<StorageImport>(unknown.as_bytes()).is_err());
}

#[test]
fn a2ui_export_degrades_unsupported_catalog() {
    let mut root = fixture_root();
    let mut child = SemanticNode::new(
        "fixture/native-portal",
        ComponentKind::NativePortal,
        "Portal",
    );
    child.props = BTreeMap::from([("status".to_owned(), PropValue::Text("ready".to_owned()))])
        .try_into()
        .unwrap();
    root.push(child).expect("child");
    let mut recipe = Recipe::new(owner(), "export-recipe", "theme", root).unwrap();
    recipe
        .refresh_after_declared_change()
        .expect("refresh export fixture");
    let projection = export_projection(
        &recipe,
        "surface-1",
        A2uiVersion::V1_0,
        &ExportCatalog::minimal(),
    )
    .unwrap();
    assert!(projection.lossy);
    assert_eq!(projection.degraded_nodes, 1);
    let bytes = capsule_surface_model::canonical::canonical_bytes(&projection).unwrap();
    assert!(bytes.windows(11).any(|window| window == b"unsupported"));
}

#[test]
fn surfaces_are_ephemeral_and_migration_is_strict() {
    let recipe = fixture_recipe();
    let surface = Surface::from_recipe(&recipe, "surface-1", 1).unwrap();
    surface.validate().unwrap();
    assert_eq!(surface.recipe_digest, recipe.digest);

    let json = capsule_surface_model::canonical::canonical_string(&recipe).unwrap();
    let parsed: Recipe = parse_canonical(json.as_bytes()).unwrap();
    assert_eq!(parsed, recipe);
    assert_eq!(
        parsed.downgrade_boundary("aos.recipe@0.9"),
        Err(RecipeValidationError::Identity)
    );
}

#[test]
fn activity_and_surface_docs_validate() {
    let recipe = fixture_recipe();
    let activity = fixture_activity();
    assert_eq!(activity.schema, "aos.activity@1");
    activity.validate().unwrap();
    let surface = Surface::from_recipe(&recipe, "surface-1", 1).unwrap();
    surface.validate().unwrap();
    assert_eq!(surface.schema, "aos.surface@1");
}

#[test]
fn hostile_owner_extension_and_surface_documents_fail_closed() {
    assert!(
        Activity::new(
            "activity",
            OpaqueOwnerRef::Principal("../../home/owner".to_owned()),
            "title",
            "recipe",
        )
        .is_err()
    );
    let path_owner = r#"{"activity_id":"activity","owner_ref":{"id":"../../home/owner","kind":"principal"},"recipe_id":"recipe","schema":"aos.activity@1","title":"title"}"#;
    assert!(parse_canonical::<Activity>(path_owner.as_bytes()).is_err());

    assert!(
        Extensions::new(vec![(
            "../../home".to_owned(),
            CanonicalJson::String("path".to_owned()),
        )])
        .is_err()
    );
    assert!(
        Extensions::new(vec![(
            "vendor.example/oversize".to_owned(),
            CanonicalJson::String("x".repeat(4097)),
        )])
        .is_err()
    );

    let mut recipe = fixture_recipe();
    recipe.bindings.extensions = Extensions(vec![
        (
            "../../home".to_owned(),
            CanonicalJson::String("path".to_owned()),
        ),
        (
            "vendor.example/oversize".to_owned(),
            CanonicalJson::String("x".repeat(4097)),
        ),
    ]);
    assert_eq!(recipe.validate(), Err(RecipeValidationError::Identity));
    let hostile_extensions = canonical_string(&recipe).expect("serialize constructed recipe");
    assert!(parse_canonical::<Recipe>(hostile_extensions.as_bytes()).is_err());

    assert!(Surface::from_recipe(&fixture_recipe(), "../../home/surface", 1).is_err());
    let mut surface = Surface::from_recipe(&fixture_recipe(), "surface-1", 1).unwrap();
    surface.schema = "aos.surface@evil".to_owned();
    surface.surface_id = "../../home/surface".to_owned();
    let surface_json = canonical_string(&surface).expect("serialize constructed surface");
    assert!(parse_canonical::<Surface>(surface_json.as_bytes()).is_err());

    let mut path_recipe = Surface::from_recipe(&fixture_recipe(), "surface-1", 1).unwrap();
    path_recipe.recipe_id = "../../home/recipe".to_owned();
    let path_recipe_json = canonical_string(&path_recipe).expect("serialize path recipe_id");
    assert!(parse_canonical::<Surface>(path_recipe_json.as_bytes()).is_err());
}

#[test]
fn recipe_parent_link_requires_contiguous_blake3_digest() {
    let mut recipe = fixture_recipe();
    recipe.revision = 10;
    recipe.parent_revision = Some(8);
    recipe.parent_digest = "not-a-digest".to_owned();
    assert_eq!(recipe.validate(), Err(RecipeValidationError::ParentLink));
    assert!(recipe.refresh_after_declared_change().is_err());
    let json = canonical_string(&recipe).expect("serialize constructed recipe");
    assert!(parse_canonical::<Recipe>(json.as_bytes()).is_err());
}

#[test]
fn a2ui_export_projects_button_action_and_declares_unprojected_loss() {
    let mut root = fixture_root();
    let mut button = SemanticNode::new("button", ComponentKind::Button, "Save");
    button.props = BTreeMap::from([
        ("label".to_owned(), PropValue::Text("Save".to_owned())),
        ("action".to_owned(), PropValue::Token("save".to_owned())),
    ])
    .try_into()
    .expect("button props");
    root.push(button.clone()).expect("button child");
    let recipe = Recipe::new(owner(), "button-recipe", "theme", root).unwrap();
    let projection = export_projection(
        &recipe,
        "surface",
        A2uiVersion::V1_0,
        &ExportCatalog::minimal(),
    )
    .expect("projection");
    assert!(!projection.lossy);
    assert_eq!(projection.degraded_nodes, 0);
    let json = canonical_string(&projection).expect("projection serializes");
    assert!(json.contains("save"));

    let mut lossy_root = fixture_root();
    let mut extra = button;
    extra.props = extra
        .props
        .with("status".to_owned(), PropValue::Text("ready".to_owned()))
        .expect("extra prop");
    lossy_root.push(extra).expect("lossy button");
    let lossy_recipe = Recipe::new(owner(), "button-lossy", "theme", lossy_root).unwrap();
    let lossy = export_projection(
        &lossy_recipe,
        "surface",
        A2uiVersion::V0_9,
        &ExportCatalog::minimal(),
    )
    .expect("lossy projection");
    assert!(lossy.lossy);
    assert!(lossy.degraded_nodes >= 1);
}

#[test]
fn rollback_restore_patch_replays_and_rejects_cross_owner() {
    let recipe = fixture_recipe();
    let mut store = RecipeStore::new();
    store.insert(recipe.clone()).expect("insert");
    let first = reviewed_patch(&recipe, "one", PatchOp::ClearRoot);
    store.apply_patch(&first).expect("one");
    let after_one = store.get(recipe.recipe_id.as_str()).unwrap().clone();
    let second = reviewed_patch(
        &after_one,
        "two",
        PatchOp::SetTheme {
            theme_id: "second-theme".to_owned(),
        },
    );
    store.apply_patch(&second).expect("two");
    let head = store.get(recipe.recipe_id.as_str()).unwrap().clone();
    let restore = Patch::new(
        owner(),
        actor(),
        reviewer(),
        "rollback-review",
        "rollback-1",
        recipe.recipe_id.clone(),
        head.revision,
        head.digest.clone(),
        "rollback to revision 1",
        vec![PatchOp::RestoreRevision {
            target_revision: 1,
            target_digest: recipe.digest.clone(),
        }],
    )
    .expect("restore patch");
    let applied = store.apply_patch(&restore).expect("restore applied");
    assert!(matches!(
        applied,
        capsule_surface_model::store::PatchOutcome::Applied { .. }
    ));
    let replayed = store.apply_patch(&restore).expect("restore idempotent");
    assert!(matches!(
        replayed,
        capsule_surface_model::store::PatchOutcome::AlreadyApplied(_)
    ));
    let live = store.get(recipe.recipe_id.as_str()).unwrap().clone();
    assert_eq!(live.revision, 4);
    assert_eq!(live.root, recipe.root);
    assert_eq!(live.theme_id, recipe.theme_id);

    let recipe_json =
        capsule_surface_model::canonical::canonical_bytes(&recipe).expect("canonical");
    let replay =
        RecipeStore::replay(&recipe_json, &[first, second, restore.clone()]).expect("replay");
    assert_eq!(replay, live);

    let foreign = Patch::new(
        OpaqueOwnerRef::User("uid:user:other".to_owned()),
        actor(),
        reviewer(),
        "receipt",
        "foreign-rollback",
        recipe.recipe_id.clone(),
        live.revision,
        live.digest.clone(),
        "cross owner restore",
        vec![PatchOp::RestoreRevision {
            target_revision: 1,
            target_digest: recipe.digest.clone(),
        }],
    )
    .expect("foreign restore shape");
    assert_eq!(store.apply_patch(&foreign), Err(PatchError::OwnerMismatch));
    assert_eq!(
        store.rollback(
            RollbackRequest {
                owner_ref: OpaqueOwnerRef::User("uid:user:other".to_owned()),
                recipe_id: recipe.recipe_id.clone(),
                target_revision: 1,
                acting_principal: actor(),
                reviewer: reviewer(),
                receipt: "foreign-helper".to_owned(),
            },
            "foreign-helper",
        ),
        Err(PatchError::OwnerMismatch)
    );
}
