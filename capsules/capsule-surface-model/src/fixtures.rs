//! Deterministic canonical fixtures shared by tests and callers.

use crate::activity::{OpaqueOwnerRef, OpaquePrincipalRef};
use crate::bindings::{
    Binding, CachePolicy, CapsuleBinding, DataBinding, FreshnessPolicy, NativeBinding,
    RestorePolicy, RhaiReference,
};
use crate::components::{ComponentKind, PropValue, SemanticNode, StateSet};
use crate::recipe::{Activity, Recipe};
use crate::store::{Patch, PatchOp, ReviewAcceptance};
use std::collections::BTreeMap;

pub fn owner() -> OpaqueOwnerRef {
    OpaqueOwnerRef::Principal("uid:principal:fixture".to_owned())
}

pub fn actor() -> OpaquePrincipalRef {
    OpaquePrincipalRef::Agent("uid:agent:fixture".to_owned())
}

pub fn reviewer() -> OpaquePrincipalRef {
    OpaquePrincipalRef::User("uid:user:fixture-reviewer".to_owned())
}

pub fn fixture_root() -> SemanticNode {
    let mut root = SemanticNode::new("fixture/root", ComponentKind::Region, "Fixture");
    let mut text = SemanticNode::new("fixture/text", ComponentKind::Text, "Summary");
    text.props = BTreeMap::from([(
        "text".to_owned(),
        PropValue::Text("Canonical surface".to_owned()),
    )])
    .try_into()
    .expect("fixture properties");
    root.push(text).expect("fixture child");
    root
}

pub fn fixture_recipe() -> Recipe {
    let mut recipe = Recipe::new(owner(), "fixture-recipe", "fixture-theme", fixture_root())
        .expect("fixture recipe");
    recipe.bindings.binding = Some(Binding::Capsule(CapsuleBinding {
        capsule: "fixture-capsule".to_owned(),
        interface: "fixture:surface".to_owned(),
        contract_version: "1.0.0".to_owned(),
        requested_route: "fixture.v1.surface".to_owned(),
    }));
    recipe.bindings.rhai = Some(RhaiReference::NamedProfile {
        profile: "fixture-profile".to_owned(),
    });
    recipe
        .refresh_after_declared_change()
        .expect("fixture digest");
    recipe
}

pub fn fixture_activity() -> Activity {
    let mut activity = Activity::new(
        "fixture-activity",
        owner(),
        "Canonical fixture",
        "fixture-recipe",
    )
    .expect("fixture activity");
    activity.bindings.binding = Some(Binding::Native(NativeBinding {
        app_identity: "dev.fixture.application".to_owned(),
        portal_contract: "fixture:portal".to_owned(),
        descriptor_schema: "fixture:descriptor-v1".to_owned(),
        descriptor_id: "fixture-object-1".to_owned(),
        restore_policy: RestorePolicy::RestoreRecipe,
    }));
    activity
}

pub fn fixture_data_binding() -> Binding {
    Binding::Data(DataBinding {
        owner: owner(),
        kernel_object_id: "kernel-object-1".to_owned(),
        grant_id: Some("grant-1".to_owned()),
        content_hash: "f".repeat(64),
        mime: "application/json".to_owned(),
        freshness: FreshnessPolicy::OlderThanMs(60_000),
        cache: CachePolicy::Private,
    })
}

pub fn fixture_accepted_patch(recipe: &Recipe, patch_id: &str) -> Patch {
    let root_id = recipe.root.id;
    Patch {
        schema: crate::recipe::PATCH_SCHEMA.to_owned(),
        owner_ref: owner(),
        acting_principal: actor(),
        proposal_id: format!("proposal:{patch_id}"),
        review: ReviewAcceptance {
            reviewer: reviewer(),
            receipt: format!("receipt:{patch_id}"),
        },
        patch_id: patch_id.to_owned(),
        recipe_id: recipe.recipe_id.clone(),
        base_revision: recipe.revision,
        base_digest: recipe.digest.clone(),
        summary: "Fixture reviewed update".to_owned(),
        operations: vec![PatchOp::SetState {
            node_id: root_id,
            state: StateSet::empty().with(StateSet::FOCUS, true),
        }],
    }
    .tap_valid()
}

trait FixturePatch {
    fn tap_valid(self) -> Self;
}

impl FixturePatch for Patch {
    fn tap_valid(self) -> Self {
        self.validate().expect("fixture patch");
        self
    }
}

pub const ACTIVITY_CANONICAL_JSON: &str = r#"{"activity_id":"fixture-activity","binding":{"app_identity":"dev.fixture.application","descriptor_id":"fixture-object-1","descriptor_schema":"fixture:descriptor-v1","kind":"native","portal_contract":"fixture:portal","restore_policy":"restore-recipe"},"owner_ref":{"id":"uid:principal:fixture","kind":"principal"},"recipe_id":"fixture-recipe","schema":"aos.activity@1","title":"Canonical fixture"}"#;

pub const RECIPE_CANONICAL_JSON: &str = r#"{"binding":{"capsule":"fixture-capsule","contract_version":"1.0.0","interface":"fixture:surface","kind":"capsule","requested_route":"fixture.v1.surface"},"digest":"888ac8aaf92fed7c575e31e4c5f1b438b220878a1246b5fca9a8b7b6502d6b7d","metadata":{},"owner_ref":{"id":"uid:principal:fixture","kind":"principal"},"recipe_id":"fixture-recipe","revision":1,"rhai":{"kind":"named-profile","profile":"fixture-profile"},"root":{"accessibility":{"name":"Fixture","role":"Structure"},"children":[{"accessibility":{"name":"Summary","role":"Content"},"children":[],"id":6968700761525072255,"kind":"Text","props":{"text":"Canonical surface"},"state":0}],"id":3897659296475492384,"kind":"Region","props":{},"state":0},"schema":"aos.recipe@1","theme_id":"fixture-theme"}"#;

pub const SURFACE_CANONICAL_JSON: &str = r#"{"binding":{"capsule":"fixture-capsule","contract_version":"1.0.0","interface":"fixture:surface","kind":"capsule","requested_route":"fixture.v1.surface"},"incarnation":1,"recipe_digest":"888ac8aaf92fed7c575e31e4c5f1b438b220878a1246b5fca9a8b7b6502d6b7d","recipe_id":"fixture-recipe","recipe_revision":1,"rhai":{"kind":"named-profile","profile":"fixture-profile"},"root":{"accessibility":{"name":"Fixture","role":"Structure"},"children":[{"accessibility":{"name":"Summary","role":"Content"},"children":[],"id":6968700761525072255,"kind":"Text","props":{"text":"Canonical surface"},"state":0}],"id":3897659296475492384,"kind":"Region","props":{},"state":0},"schema":"aos.surface@1","surface_id":"fixture-surface-1"}"#;

pub const PATCH_CANONICAL_JSON: &str = r#"{"acting_principal":{"id":"uid:agent:fixture","kind":"agent"},"base_digest":"888ac8aaf92fed7c575e31e4c5f1b438b220878a1246b5fca9a8b7b6502d6b7d","base_revision":1,"operations":[{"SetState":{"node_id":3897659296475492384,"state":4}}],"owner_ref":{"id":"uid:principal:fixture","kind":"principal"},"patch_id":"fixture-patch-1","proposal_id":"proposal:fixture-patch-1","recipe_id":"fixture-recipe","review":{"receipt":"receipt:fixture-patch-1","reviewer":{"id":"uid:user:fixture-reviewer","kind":"user"}},"schema":"aos.patch@1","summary":"Fixture reviewed update"}"#;

#[cfg(test)]
mod fixture_tests {
    use super::*;
    use crate::canonical::parse_canonical;
    use crate::store::Patch;
    use crate::{Activity, Recipe, Surface};

    #[test]
    fn exact_canonical_fixtures_round_trip() {
        let recipe = fixture_recipe();
        let surface = Surface::from_recipe(&recipe, "fixture-surface-1", 1).unwrap();
        let patch = fixture_accepted_patch(&recipe, "fixture-patch-1");
        assert_eq!(
            crate::canonical::canonical_string(&fixture_activity()).unwrap(),
            ACTIVITY_CANONICAL_JSON
        );
        assert_eq!(
            crate::canonical::canonical_string(&recipe).unwrap(),
            RECIPE_CANONICAL_JSON
        );
        assert_eq!(
            crate::canonical::canonical_string(&surface).unwrap(),
            SURFACE_CANONICAL_JSON
        );
        assert_eq!(
            crate::canonical::canonical_string(&patch).unwrap(),
            PATCH_CANONICAL_JSON
        );
        let _: Activity = parse_canonical(ACTIVITY_CANONICAL_JSON.as_bytes()).unwrap();
        let _: Recipe = parse_canonical(RECIPE_CANONICAL_JSON.as_bytes()).unwrap();
        let _: Surface = parse_canonical(SURFACE_CANONICAL_JSON.as_bytes()).unwrap();
        let _: Patch = parse_canonical(PATCH_CANONICAL_JSON.as_bytes()).unwrap();
    }
}
