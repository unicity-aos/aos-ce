//! Store tests live in a sibling file so neither `store.rs` nor this
//! file crosses CI's 1000-line cap. Covers identity layer, presentation
//! layer, mutation ops, cascade semantics, and pagination.

#![cfg(test)]

use uuid::Uuid;

use crate::store::Store;
use crate::store::test_support::MemBackend;
use crate::types::StoreError;

fn make_store() -> Store<MemBackend> {
    Store::new(MemBackend::default())
}

// ── User CRUD ───────────────────────────────────────────────────

#[test]
fn create_and_get_user() {
    let store = make_store();
    let u = store.create_user(Some("Alice")).unwrap();
    assert_eq!(u.display_name.as_deref(), Some("Alice"));
    assert_eq!(store.get_user(u.id).unwrap(), Some(u));
}

#[test]
fn create_user_no_name() {
    let store = make_store();
    let u = store.create_user(None).unwrap();
    assert!(u.display_name.is_none());
}

#[test]
fn create_user_rejects_slash_in_name() {
    assert!(matches!(
        make_store().create_user(Some("admin/root")),
        Err(StoreError::InvalidInput(_))
    ));
}

#[test]
fn get_user_missing_returns_none() {
    assert!(make_store().get_user(Uuid::new_v4()).unwrap().is_none());
}

// ── set_display_name ────────────────────────────────────────────

#[test]
fn set_display_name_updates_record() {
    let store = make_store();
    let u = store.create_user(Some("Alice")).unwrap();
    let updated = store.set_display_name(u.id, Some("Allison")).unwrap();
    assert_eq!(updated.display_name.as_deref(), Some("Allison"));
    assert_eq!(updated.id, u.id);
    assert_eq!(updated.created_at, u.created_at);
}

#[test]
fn set_display_name_clear_with_none() {
    let store = make_store();
    let u = store.create_user(Some("Alice")).unwrap();
    let cleared = store.set_display_name(u.id, None).unwrap();
    assert!(cleared.display_name.is_none());
}

#[test]
fn set_display_name_rejects_invalid_input() {
    let store = make_store();
    let u = store.create_user(None).unwrap();
    assert!(matches!(
        store.set_display_name(u.id, Some("bad/name")),
        Err(StoreError::InvalidInput(_))
    ));
}

#[test]
fn set_display_name_missing_user() {
    assert!(matches!(
        make_store().set_display_name(Uuid::new_v4(), Some("ghost")),
        Err(StoreError::UserNotFound(_))
    ));
}

// ── set_public_key ──────────────────────────────────────────────

#[test]
fn set_public_key_round_trips() {
    let store = make_store();
    let u = store.create_user(None).unwrap();
    let key = [0xAB; 32];
    let updated = store.set_public_key(u.id, Some(key)).unwrap();
    assert_eq!(updated.public_key, Some(key));
}

#[test]
fn set_public_key_clear() {
    let store = make_store();
    let u = store.create_user(None).unwrap();
    store.set_public_key(u.id, Some([1; 32])).unwrap();
    let cleared = store.set_public_key(u.id, None).unwrap();
    assert!(cleared.public_key.is_none());
}

// ── Link with platform_instance ─────────────────────────────────

#[test]
fn link_with_instance_round_trips() {
    let store = make_store();
    let u = store.create_user(None).unwrap();
    store
        .link("slack", Some("T01"), "U01", u.id, "admin", None)
        .unwrap();
    let (resolved, _) = store.resolve("slack", Some("T01"), "U01", None).unwrap();
    assert_eq!(resolved.unwrap().id, u.id);
}

#[test]
fn link_distinct_instances_are_separate() {
    let store = make_store();
    let alice = store.create_user(Some("Alice")).unwrap();
    let bob = store.create_user(Some("Bob")).unwrap();
    store
        .link("slack", Some("T01"), "U01", alice.id, "admin", None)
        .unwrap();
    store
        .link("slack", Some("T02"), "U01", bob.id, "admin", None)
        .unwrap();
    assert_eq!(
        store
            .resolve("slack", Some("T01"), "U01", None)
            .unwrap()
            .0
            .unwrap()
            .id,
        alice.id
    );
    assert_eq!(
        store
            .resolve("slack", Some("T02"), "U01", None)
            .unwrap()
            .0
            .unwrap()
            .id,
        bob.id
    );
}

#[test]
fn link_no_instance_is_separate_from_with_instance() {
    let store = make_store();
    let alice = store.create_user(Some("Alice")).unwrap();
    let bob = store.create_user(Some("Bob")).unwrap();
    store
        .link("discord", None, "12345", alice.id, "admin", None)
        .unwrap();
    store
        .link("discord", Some("T01"), "12345", bob.id, "admin", None)
        .unwrap();
    assert_eq!(
        store
            .resolve("discord", None, "12345", None)
            .unwrap()
            .0
            .unwrap()
            .id,
        alice.id
    );
    assert_eq!(
        store
            .resolve("discord", Some("T01"), "12345", None)
            .unwrap()
            .0
            .unwrap()
            .id,
        bob.id
    );
}

#[test]
fn link_rejects_reserved_instance_sentinel() {
    let store = make_store();
    let u = store.create_user(None).unwrap();
    assert!(matches!(
        store.link("slack", Some("_"), "U01", u.id, "admin", None),
        Err(StoreError::InvalidInput(_))
    ));
}

#[test]
fn link_with_display_name_round_trips() {
    let store = make_store();
    let u = store.create_user(None).unwrap();
    let link = store
        .link("discord", None, "12345", u.id, "admin", Some("alice"))
        .unwrap();
    assert_eq!(link.display_name.as_deref(), Some("alice"));
}

#[test]
fn link_rejects_slash_in_inputs() {
    let store = make_store();
    let u = store.create_user(None).unwrap();
    assert!(matches!(
        store.link("a/b", None, "1", u.id, "admin", None),
        Err(StoreError::InvalidInput(_))
    ));
    assert!(matches!(
        store.link("discord", None, "1/2", u.id, "admin", None),
        Err(StoreError::InvalidInput(_))
    ));
    assert!(matches!(
        store.link("discord", Some("inst/bad"), "1", u.id, "admin", None),
        Err(StoreError::InvalidInput(_))
    ));
}

#[test]
fn link_rejects_missing_user() {
    let err = make_store()
        .link("discord", None, "1", Uuid::new_v4(), "admin", None)
        .unwrap_err();
    assert!(matches!(err, StoreError::UserNotFound(_)));
}

#[test]
fn link_upsert_overwrites() {
    let store = make_store();
    let a = store.create_user(Some("Alice")).unwrap();
    let b = store.create_user(Some("Bob")).unwrap();
    store
        .link("discord", None, "1", a.id, "admin", None)
        .unwrap();
    store
        .link("discord", None, "1", b.id, "admin", None)
        .unwrap();
    assert_eq!(
        store
            .resolve("discord", None, "1", None)
            .unwrap()
            .0
            .unwrap()
            .id,
        b.id
    );
}

// ── Context-aware resolve (layering) ────────────────────────────

#[test]
fn resolve_no_context_uses_link_name() {
    let store = make_store();
    let u = store.create_user(Some("Alice Canonical")).unwrap();
    store
        .link("discord", None, "1", u.id, "admin", Some("alice-link"))
        .unwrap();
    let (_, name) = store.resolve("discord", None, "1", None).unwrap();
    let n = name.unwrap();
    assert_eq!(n.name, "alice-link");
    assert_eq!(n.source, "link");
}

#[test]
fn resolve_falls_back_to_canonical_when_link_has_no_name() {
    let store = make_store();
    let u = store.create_user(Some("Alice Canonical")).unwrap();
    store
        .link("discord", None, "1", u.id, "admin", None)
        .unwrap();
    let (_, name) = store.resolve("discord", None, "1", None).unwrap();
    let n = name.unwrap();
    assert_eq!(n.name, "Alice Canonical");
    assert_eq!(n.source, "canonical");
}

#[test]
fn resolve_context_overlay_takes_precedence() {
    let store = make_store();
    let u = store.create_user(Some("Alice Canonical")).unwrap();
    store
        .link("discord", None, "1", u.id, "admin", Some("alice-link"))
        .unwrap();
    store
        .set_context("discord", None, "1", "guild:G1", "Sis")
        .unwrap();
    let (_, name) = store
        .resolve("discord", None, "1", Some("guild:G1"))
        .unwrap();
    let n = name.unwrap();
    assert_eq!(n.name, "Sis");
    assert_eq!(n.source, "context");
}

#[test]
fn resolve_unknown_context_falls_back() {
    let store = make_store();
    let u = store.create_user(Some("Alice")).unwrap();
    store
        .link("discord", None, "1", u.id, "admin", Some("alice-link"))
        .unwrap();
    let (_, name) = store
        .resolve("discord", None, "1", Some("guild:UNK"))
        .unwrap();
    assert_eq!(name.unwrap().source, "link");
}

#[test]
fn resolve_missing_returns_none() {
    assert!(
        make_store()
            .resolve("discord", None, "ghost", None)
            .unwrap()
            .0
            .is_none()
    );
}

#[test]
fn resolve_no_display_anywhere() {
    let store = make_store();
    let u = store.create_user(None).unwrap();
    store
        .link("discord", None, "1", u.id, "admin", None)
        .unwrap();
    let (_, name) = store.resolve("discord", None, "1", None).unwrap();
    assert!(name.is_none());
}

// ── Context overlay CRUD ────────────────────────────────────────

#[test]
fn set_context_requires_existing_link() {
    let store = make_store();
    assert!(matches!(
        store.set_context("discord", None, "1", "guild:G1", "Sis"),
        Err(StoreError::LinkNotFound)
    ));
}

#[test]
fn set_context_upsert() {
    let store = make_store();
    let u = store.create_user(None).unwrap();
    store
        .link("discord", None, "1", u.id, "admin", None)
        .unwrap();
    store
        .set_context("discord", None, "1", "guild:G1", "Sis")
        .unwrap();
    store
        .set_context("discord", None, "1", "guild:G1", "Sister")
        .unwrap();
    let (overlay, _) = store.get_context("discord", None, "1", "guild:G1").unwrap();
    assert_eq!(overlay.unwrap().display_name, "Sister");
}

#[test]
fn clear_context_idempotent() {
    let store = make_store();
    let u = store.create_user(None).unwrap();
    store
        .link("discord", None, "1", u.id, "admin", None)
        .unwrap();
    store
        .set_context("discord", None, "1", "guild:G1", "Sis")
        .unwrap();
    assert!(
        store
            .clear_context("discord", None, "1", "guild:G1")
            .unwrap()
    );
    assert!(
        !store
            .clear_context("discord", None, "1", "guild:G1")
            .unwrap()
    );
}

#[test]
fn get_context_returns_astrid_user_id_when_link_exists() {
    let store = make_store();
    let u = store.create_user(None).unwrap();
    store
        .link("discord", None, "1", u.id, "admin", None)
        .unwrap();
    let (overlay, uid) = store.get_context("discord", None, "1", "guild:G1").unwrap();
    assert!(overlay.is_none()); // no overlay set
    assert_eq!(uid, Some(u.id)); // but link exists, so user is known
}

#[test]
fn get_context_with_no_link_returns_no_user() {
    let store = make_store();
    let (overlay, uid) = store
        .get_context("discord", None, "999", "guild:G1")
        .unwrap();
    assert!(overlay.is_none());
    assert!(uid.is_none());
}

// ── Cascade semantics ───────────────────────────────────────────

#[test]
fn unlink_cascades_context_overlays() {
    let store = make_store();
    let u = store.create_user(None).unwrap();
    store
        .link("discord", None, "1", u.id, "admin", None)
        .unwrap();
    store
        .set_context("discord", None, "1", "guild:G1", "Sis")
        .unwrap();
    store
        .set_context("discord", None, "1", "guild:G2", "AliceTheGamer")
        .unwrap();
    assert!(store.unlink("discord", None, "1").unwrap());
    assert!(
        store
            .get_context("discord", None, "1", "guild:G1")
            .unwrap()
            .0
            .is_none()
    );
    assert!(
        store
            .get_context("discord", None, "1", "guild:G2")
            .unwrap()
            .0
            .is_none()
    );
}

#[test]
fn unlink_preserves_other_users_overlays_in_same_context() {
    let store = make_store();
    let alice = store.create_user(Some("Alice")).unwrap();
    let bob = store.create_user(Some("Bob")).unwrap();
    store
        .link("discord", None, "1", alice.id, "admin", None)
        .unwrap();
    store
        .link("discord", None, "2", bob.id, "admin", None)
        .unwrap();
    store
        .set_context("discord", None, "1", "guild:G1", "Sis")
        .unwrap();
    store
        .set_context("discord", None, "2", "guild:G1", "Bro")
        .unwrap();
    store.unlink("discord", None, "1").unwrap();
    assert!(
        store
            .get_context("discord", None, "2", "guild:G1")
            .unwrap()
            .0
            .is_some()
    );
}

#[test]
fn delete_user_cascades_links_and_context_overlays() {
    let store = make_store();
    let alice = store.create_user(Some("Alice")).unwrap();
    store
        .link("discord", None, "1", alice.id, "admin", None)
        .unwrap();
    store
        .link("telegram", None, "tg-1", alice.id, "admin", None)
        .unwrap();
    store
        .set_context("discord", None, "1", "guild:G1", "Sis")
        .unwrap();
    store
        .set_context("telegram", None, "tg-1", "chat:C1", "Sis")
        .unwrap();
    assert!(store.delete_user(alice.id).unwrap());
    assert!(
        store
            .resolve("discord", None, "1", None)
            .unwrap()
            .0
            .is_none()
    );
    assert!(
        store
            .get_context("discord", None, "1", "guild:G1")
            .unwrap()
            .0
            .is_none()
    );
    assert!(
        store
            .get_context("telegram", None, "tg-1", "chat:C1")
            .unwrap()
            .0
            .is_none()
    );
}

// ── list_links ──────────────────────────────────────────────────

#[test]
fn list_links_includes_instances() {
    let store = make_store();
    let u = store.create_user(None).unwrap();
    store
        .link("slack", Some("T01"), "U01", u.id, "admin", None)
        .unwrap();
    store
        .link("slack", Some("T02"), "U02", u.id, "admin", None)
        .unwrap();
    store
        .link("discord", None, "9", u.id, "admin", None)
        .unwrap();
    let mut links = store.list_links(u.id).unwrap();
    // Stable order for assertion: sort by (platform, instance, user_id).
    links.sort_by(|a, b| {
        a.platform
            .cmp(&b.platform)
            .then(a.platform_instance.cmp(&b.platform_instance))
            .then(a.platform_user_id.cmp(&b.platform_user_id))
    });
    assert_eq!(links.len(), 3);
    assert_eq!(
        links
            .iter()
            .map(|l| (l.platform.as_str(), l.platform_instance.clone()))
            .collect::<Vec<_>>(),
        vec![
            ("discord", None),
            ("slack", Some("T01".into())),
            ("slack", Some("T02".into())),
        ]
    );
}

// ── Pagination on list_users ────────────────────────────────────

#[test]
fn list_users_paginated_returns_all_pages() {
    let store = make_store();
    let mut created: Vec<Uuid> = (0..5)
        .map(|i| store.create_user(Some(&format!("u{i}"))).unwrap().id)
        .collect();
    created.sort();

    let page1 = store.list_users_paginated(None, Some(2)).unwrap();
    assert_eq!(page1.items.len(), 2);
    assert!(page1.next_cursor.is_some());

    let page2 = store
        .list_users_paginated(page1.next_cursor.as_deref(), Some(2))
        .unwrap();
    assert_eq!(page2.items.len(), 2);
    assert!(page2.next_cursor.is_some());

    let page3 = store
        .list_users_paginated(page2.next_cursor.as_deref(), Some(2))
        .unwrap();
    assert_eq!(page3.items.len(), 1);
    assert!(page3.next_cursor.is_none());

    // Reassembling all three pages gives the full set, no duplicates.
    let mut got: Vec<Uuid> = page1
        .items
        .iter()
        .chain(page2.items.iter())
        .chain(page3.items.iter())
        .map(|u| u.id)
        .collect();
    got.sort();
    assert_eq!(got, created);
}

#[test]
fn list_users_paginated_default_limit() {
    let store = make_store();
    for i in 0..3 {
        store.create_user(Some(&format!("u{i}"))).unwrap();
    }
    let page = store.list_users_paginated(None, None).unwrap();
    assert_eq!(page.items.len(), 3);
    assert!(page.next_cursor.is_none());
}

// ── Pagination on list_context_for_user ─────────────────────────

#[test]
fn list_context_for_user_paginates() {
    let store = make_store();
    let u = store.create_user(None).unwrap();
    store
        .link("discord", None, "1", u.id, "admin", None)
        .unwrap();
    for i in 0..5 {
        store
            .set_context(
                "discord",
                None,
                "1",
                &format!("guild:G{i}"),
                &format!("name{i}"),
            )
            .unwrap();
    }
    let p1 = store.list_context_for_user(u.id, None, Some(2)).unwrap();
    let p2 = store
        .list_context_for_user(u.id, p1.next_cursor.as_deref(), Some(2))
        .unwrap();
    let p3 = store
        .list_context_for_user(u.id, p2.next_cursor.as_deref(), Some(2))
        .unwrap();
    assert_eq!(p1.items.len() + p2.items.len() + p3.items.len(), 5);
    assert!(p3.next_cursor.is_none());
}

// ── list_context_in_context ─────────────────────────────────────

#[test]
fn list_context_in_context_returns_member_roster() {
    let store = make_store();
    let alice = store.create_user(Some("Alice")).unwrap();
    let bob = store.create_user(Some("Bob")).unwrap();
    store
        .link("discord", None, "1", alice.id, "admin", None)
        .unwrap();
    store
        .link("discord", None, "2", bob.id, "admin", None)
        .unwrap();
    store
        .set_context("discord", None, "1", "guild:G1", "Sis")
        .unwrap();
    store
        .set_context("discord", None, "2", "guild:G1", "Bro")
        .unwrap();
    // Also a sibling context for confounder
    store
        .set_context("discord", None, "1", "guild:G2", "OtherName")
        .unwrap();

    let page = store
        .list_context_in_context("discord", None, "guild:G1", None, None)
        .unwrap();
    assert_eq!(page.items.len(), 2);
    let names: Vec<String> = page
        .items
        .iter()
        .map(|(c, _)| c.display_name.clone())
        .collect();
    assert!(names.contains(&"Sis".to_string()));
    assert!(names.contains(&"Bro".to_string()));
}

// ── KV key construction security ────────────────────────────────

#[test]
fn validate_key_component_rejects_traversal() {
    assert!(matches!(
        Store::<MemBackend>::validate_key_component("../user", "platform"),
        Err(StoreError::InvalidInput(_))
    ));
}

#[test]
fn validate_instance_rejects_sentinel() {
    assert!(matches!(
        Store::<MemBackend>::validate_instance(Some("_")),
        Err(StoreError::InvalidInput(_))
    ));
    assert!(Store::<MemBackend>::validate_instance(None).is_ok());
    assert!(Store::<MemBackend>::validate_instance(Some("T01")).is_ok());
}
