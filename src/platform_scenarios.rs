//! End-to-end platform scenarios driven by real-shape IDs.
//!
//! Each test models how a specific platform (Discord, Slack, Telegram,
//! Matrix, X, IRC, Mastodon, Email, SMS, Nostr, Passkey, GitHub) maps
//! its native identifier model onto the capsule's
//! `(platform, platform_instance?, platform_user_id, context_id?)`
//! coordinate space. The goal is to catch contract-level mismatches
//! between the capsule's data model and real-world platforms before
//! any uplink consumes the WIT.
//!
//! The tests use the in-memory backend; they exercise the public API
//! exactly as a future uplink would over IPC.

#![cfg(test)]

use crate::store::Store;
use crate::store::test_support::MemBackend;

fn make_store() -> Store<MemBackend> {
    Store::new(MemBackend::default())
}

// =====================================================================
// Discord
// =====================================================================
// Identity: snowflake — 64-bit int, ~18-19 digit numeric string.
// Instance: None (Discord is one world).
// Context : per-guild nickname overlay.
// =====================================================================

#[test]
fn discord_basic_link_and_resolve() {
    let store = make_store();
    let alice = store.create_user(Some("Alice")).unwrap();
    store
        .link(
            "discord",
            None,
            "719275466049224704", // snowflake
            alice.id,
            "chat_command",
            Some("alice"), // global Discord username
        )
        .unwrap();

    let (user, name) = store
        .resolve("discord", None, "719275466049224704", None)
        .unwrap();
    assert_eq!(user.unwrap().id, alice.id);
    let n = name.unwrap();
    assert_eq!(n.name, "alice");
    assert_eq!(n.source, "link");
}

#[test]
fn discord_per_guild_nickname_layering() {
    // Alice is in three guilds with different nicknames. Resolve in
    // each guild returns the right name; resolve without context
    // falls back to the link's global name.
    let store = make_store();
    let alice = store.create_user(Some("Alice Smith")).unwrap();
    store
        .link(
            "discord",
            None,
            "719275466049224704",
            alice.id,
            "system",
            Some("alice"),
        )
        .unwrap();
    store
        .set_context(
            "discord",
            None,
            "719275466049224704",
            "guild:1078123456789012345",
            "AliceTheGamer",
        )
        .unwrap();
    store
        .set_context(
            "discord",
            None,
            "719275466049224704",
            "guild:9988123456789012345",
            "Sis",
        )
        .unwrap();

    let gamer = store
        .resolve(
            "discord",
            None,
            "719275466049224704",
            Some("guild:1078123456789012345"),
        )
        .unwrap()
        .1
        .unwrap();
    assert_eq!(gamer.name, "AliceTheGamer");
    assert_eq!(gamer.source, "context");

    let sis = store
        .resolve(
            "discord",
            None,
            "719275466049224704",
            Some("guild:9988123456789012345"),
        )
        .unwrap()
        .1
        .unwrap();
    assert_eq!(sis.name, "Sis");

    // Unknown guild falls back to the link's global username.
    let fallback = store
        .resolve(
            "discord",
            None,
            "719275466049224704",
            Some("guild:does-not-exist"),
        )
        .unwrap()
        .1
        .unwrap();
    assert_eq!(fallback.name, "alice");
    assert_eq!(fallback.source, "link");
}

#[test]
fn discord_unlink_strips_all_guild_overlays() {
    let store = make_store();
    let alice = store.create_user(None).unwrap();
    store
        .link(
            "discord",
            None,
            "719275466049224704",
            alice.id,
            "system",
            None,
        )
        .unwrap();
    for guild in &["guild:111", "guild:222", "guild:333"] {
        store
            .set_context("discord", None, "719275466049224704", guild, "name")
            .unwrap();
    }
    assert!(store.unlink("discord", None, "719275466049224704").unwrap());
    // No overlays remain.
    let page = store.list_context_for_user(alice.id, None, None).unwrap();
    assert!(page.items.is_empty());
}

// =====================================================================
// Slack
// =====================================================================
// Identity: U-prefixed ID, workspace-scoped.
// Instance: T-prefixed workspace ID.
// Context : per-channel profile (C-prefixed channel ID).
// =====================================================================

#[test]
fn slack_workspace_scoping_disambiguates_same_user_id() {
    // U01234ABCDE in workspace T01 is NOT the same human as U01234ABCDE
    // in workspace T02. The platform-instance field captures this.
    let store = make_store();
    let acme_alice = store.create_user(Some("Alice (Acme)")).unwrap();
    let initech_alice = store.create_user(Some("Alice (Initech)")).unwrap();

    store
        .link(
            "slack",
            Some("T01ACMEXXXX"),
            "U01234ABCDE",
            acme_alice.id,
            "oauth",
            Some("alice.acme"),
        )
        .unwrap();
    store
        .link(
            "slack",
            Some("T02INITECHX"),
            "U01234ABCDE",
            initech_alice.id,
            "oauth",
            Some("alice.initech"),
        )
        .unwrap();

    assert_eq!(
        store
            .resolve("slack", Some("T01ACMEXXXX"), "U01234ABCDE", None)
            .unwrap()
            .0
            .unwrap()
            .id,
        acme_alice.id
    );
    assert_eq!(
        store
            .resolve("slack", Some("T02INITECHX"), "U01234ABCDE", None)
            .unwrap()
            .0
            .unwrap()
            .id,
        initech_alice.id
    );
}

#[test]
fn slack_per_channel_context_overlay() {
    let store = make_store();
    let alice = store.create_user(Some("Alice")).unwrap();
    store
        .link(
            "slack",
            Some("T01ACMEXXXX"),
            "U01234ABCDE",
            alice.id,
            "oauth",
            Some("alice.acme"),
        )
        .unwrap();
    store
        .set_context(
            "slack",
            Some("T01ACMEXXXX"),
            "U01234ABCDE",
            "channel:C0ENG",
            "Alice (Engineering)",
        )
        .unwrap();

    let (_, name) = store
        .resolve(
            "slack",
            Some("T01ACMEXXXX"),
            "U01234ABCDE",
            Some("channel:C0ENG"),
        )
        .unwrap();
    assert_eq!(name.unwrap().name, "Alice (Engineering)");
}

// =====================================================================
// Telegram
// =====================================================================
// Identity: numeric user_id (int64 as string).
// Instance: None.
// Context : None typically; chat_id is routing context, not identity.
// =====================================================================

#[test]
fn telegram_simple_link() {
    let store = make_store();
    let bob = store.create_user(Some("Bob")).unwrap();
    store
        .link(
            "telegram",
            None,
            "123456789", // Telegram user id (int64)
            bob.id,
            "passkey_share",
            Some("bobsmith"), // optional @username
        )
        .unwrap();
    assert_eq!(
        store
            .resolve("telegram", None, "123456789", None)
            .unwrap()
            .0
            .unwrap()
            .id,
        bob.id
    );
}

#[test]
fn telegram_username_change_via_relink() {
    // Telegram @usernames are mutable. The uplink re-links to refresh
    // the platform-side display-name; the stable user-id is unchanged.
    let store = make_store();
    let bob = store.create_user(None).unwrap();
    store
        .link(
            "telegram",
            None,
            "123456789",
            bob.id,
            "system",
            Some("bobsmith"),
        )
        .unwrap();
    // Bob renames himself on Telegram to @bobthebuilder.
    store
        .link(
            "telegram",
            None,
            "123456789",
            bob.id,
            "system",
            Some("bobthebuilder"),
        )
        .unwrap();

    let (_, name) = store.resolve("telegram", None, "123456789", None).unwrap();
    assert_eq!(name.unwrap().name, "bobthebuilder");
}

// =====================================================================
// Matrix
// =====================================================================
// Identity: '@user:server.org' — homeserver baked into the user_id.
// Instance: None (federation already encoded in the ID).
// Context : per-room display name.
// =====================================================================

#[test]
fn matrix_federated_id_no_instance() {
    let store = make_store();
    let alice = store.create_user(Some("Alice")).unwrap();
    store
        .link(
            "matrix",
            None,
            "@alice:matrix.org",
            alice.id,
            "passkey_share",
            Some("Alice (matrix)"),
        )
        .unwrap();
    assert_eq!(
        store
            .resolve("matrix", None, "@alice:matrix.org", None)
            .unwrap()
            .0
            .unwrap()
            .id,
        alice.id
    );
}

#[test]
fn matrix_per_room_display_name() {
    let store = make_store();
    let alice = store.create_user(Some("Alice")).unwrap();
    store
        .link(
            "matrix",
            None,
            "@alice:matrix.org",
            alice.id,
            "system",
            None,
        )
        .unwrap();
    store
        .set_context(
            "matrix",
            None,
            "@alice:matrix.org",
            "room:!abcDEF123:matrix.org",
            "Alice (Rust devs)",
        )
        .unwrap();
    let (_, name) = store
        .resolve(
            "matrix",
            None,
            "@alice:matrix.org",
            Some("room:!abcDEF123:matrix.org"),
        )
        .unwrap();
    assert_eq!(name.unwrap().name, "Alice (Rust devs)");
}

// =====================================================================
// X (Twitter)
// =====================================================================
// Identity: numeric id_str.
// Instance: None.
// Context : None (single global feed).
// =====================================================================

#[test]
fn x_handle_change_does_not_break_link() {
    // The numeric ID is stable; the @handle is mutable. Re-link to
    // refresh the platform-side display name.
    let store = make_store();
    let alice = store.create_user(None).unwrap();
    store
        .link("x", None, "1234567890", alice.id, "oauth", Some("@alice"))
        .unwrap();
    store
        .link(
            "x",
            None,
            "1234567890",
            alice.id,
            "oauth",
            Some("@alice_new"),
        )
        .unwrap();
    let (_, name) = store.resolve("x", None, "1234567890", None).unwrap();
    assert_eq!(name.unwrap().name, "@alice_new");
}

// =====================================================================
// IRC
// =====================================================================
// Identity: nick within a network.
// Instance: network (libera.chat, OFTC, EFNet, ...).
// Context : per-channel display rare in practice.
// =====================================================================

#[test]
fn irc_per_network_scoping() {
    // 'alice' on Libera is a different human than 'alice' on OFTC.
    let store = make_store();
    let libera_alice = store.create_user(Some("Alice (Libera)")).unwrap();
    let oftc_alice = store.create_user(Some("Alice (OFTC)")).unwrap();
    store
        .link(
            "irc",
            Some("libera.chat"),
            "alice",
            libera_alice.id,
            "nickserv",
            None,
        )
        .unwrap();
    store
        .link(
            "irc",
            Some("oftc.net"),
            "alice",
            oftc_alice.id,
            "nickserv",
            None,
        )
        .unwrap();
    assert_eq!(
        store
            .resolve("irc", Some("libera.chat"), "alice", None)
            .unwrap()
            .0
            .unwrap()
            .id,
        libera_alice.id
    );
    assert_eq!(
        store
            .resolve("irc", Some("oftc.net"), "alice", None)
            .unwrap()
            .0
            .unwrap()
            .id,
        oftc_alice.id
    );
}

// =====================================================================
// Mastodon
// =====================================================================
// Identity: '@user@instance' — instance baked into the handle.
// Instance: None (encoded in the ID).
// Context : None.
// =====================================================================

#[test]
fn mastodon_federated_id_no_instance() {
    let store = make_store();
    let alice = store.create_user(None).unwrap();
    store
        .link(
            "mastodon",
            None,
            "@alice@mastodon.social",
            alice.id,
            "passkey_share",
            Some("Alice"),
        )
        .unwrap();
    assert_eq!(
        store
            .resolve("mastodon", None, "@alice@mastodon.social", None)
            .unwrap()
            .0
            .unwrap()
            .id,
        alice.id
    );
}

// =====================================================================
// Email
// =====================================================================
// Identity: full address (already globally unique).
// Instance: None.
// Context : None.
// =====================================================================

#[test]
fn email_link_resolves_by_full_address() {
    let store = make_store();
    let alice = store.create_user(None).unwrap();
    store
        .link(
            "email",
            None,
            "alice@example.com",
            alice.id,
            "passkey_share",
            Some("Alice Smith"),
        )
        .unwrap();
    assert_eq!(
        store
            .resolve("email", None, "alice@example.com", None)
            .unwrap()
            .0
            .unwrap()
            .id,
        alice.id
    );
}

// =====================================================================
// SMS
// =====================================================================
// Identity: E.164 phone number.
// Instance: None.
// Context : None.
// =====================================================================

#[test]
fn sms_link_by_e164() {
    let store = make_store();
    let alice = store.create_user(None).unwrap();
    store
        .link("sms", None, "+14155552671", alice.id, "passkey_share", None)
        .unwrap();
    assert_eq!(
        store
            .resolve("sms", None, "+14155552671", None)
            .unwrap()
            .0
            .unwrap()
            .id,
        alice.id
    );
}

// =====================================================================
// Nostr
// =====================================================================
// Identity: npub (bech32-encoded ed25519 pubkey).
// Instance: None.
// Context : None typically.
// =====================================================================

#[test]
fn nostr_link_with_pubkey_on_user() {
    // Nostr identity IS a pubkey. Store it both on the link (as the
    // platform-user-id) and on the AstridUser (for cryptographic ops).
    let store = make_store();
    let alice = store.create_user(Some("Alice")).unwrap();
    let pubkey = [0x42u8; 32];
    store.set_public_key(alice.id, Some(pubkey)).unwrap();
    store
        .link(
            "nostr",
            None,
            "npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqs3luca5",
            alice.id,
            "self_declared",
            None,
        )
        .unwrap();

    let user = store.get_user(alice.id).unwrap().unwrap();
    assert_eq!(user.public_key, Some(pubkey));
    assert_eq!(
        store
            .resolve(
                "nostr",
                None,
                "npub1qqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqqs3luca5",
                None
            )
            .unwrap()
            .0
            .unwrap()
            .id,
        alice.id
    );
}

// =====================================================================
// Passkey / FIDO2
// =====================================================================
// Identity: credential id (opaque byte string, base64url-encoded).
// Public key lives on the AstridUser.
// Instance: None.
// Context : None.
// =====================================================================

#[test]
fn passkey_credential_link_with_pubkey() {
    let store = make_store();
    let alice = store.create_user(Some("Alice")).unwrap();
    let pubkey = [0xCDu8; 32];
    store.set_public_key(alice.id, Some(pubkey)).unwrap();
    store
        .link(
            "passkey",
            None,
            "AQIDBAUGBwgJCgsMDQ4PEBESExQVFhcYGRobHB0eHyAhIiMkJSYnKCkqKywtLi8wMTI",
            alice.id,
            "passkey_register",
            None,
        )
        .unwrap();
    let user = store.get_user(alice.id).unwrap().unwrap();
    assert_eq!(user.public_key, Some(pubkey));
}

// =====================================================================
// GitHub
// =====================================================================
// Identity: numeric ID (stable). Login is mutable.
// Instance: None.
// =====================================================================

#[test]
fn github_login_change_via_relink() {
    let store = make_store();
    let alice = store.create_user(None).unwrap();
    store
        .link("github", None, "584148", alice.id, "oauth", Some("alice"))
        .unwrap();
    // Alice changes her login to alice-codes.
    store
        .link(
            "github",
            None,
            "584148",
            alice.id,
            "oauth",
            Some("alice-codes"),
        )
        .unwrap();
    let (_, name) = store.resolve("github", None, "584148", None).unwrap();
    assert_eq!(name.unwrap().name, "alice-codes");
}

// =====================================================================
// Cross-platform unification
// =====================================================================

#[test]
fn one_human_six_platforms() {
    // Alice is reachable on six platforms; each link points at the same
    // AstridUser. Listing links surfaces all of them.
    let store = make_store();
    let alice = store.create_user(Some("Alice")).unwrap();
    store
        .link(
            "discord",
            None,
            "719275466049224704",
            alice.id,
            "self_declared",
            Some("alice"),
        )
        .unwrap();
    store
        .link(
            "slack",
            Some("T01ACME"),
            "U01ABC",
            alice.id,
            "oauth",
            Some("alice"),
        )
        .unwrap();
    store
        .link(
            "telegram",
            None,
            "123456789",
            alice.id,
            "passkey_share",
            Some("@alice"),
        )
        .unwrap();
    store
        .link(
            "matrix",
            None,
            "@alice:matrix.org",
            alice.id,
            "passkey_share",
            None,
        )
        .unwrap();
    store
        .link(
            "email",
            None,
            "alice@example.com",
            alice.id,
            "passkey_share",
            None,
        )
        .unwrap();
    store
        .link(
            "nostr",
            None,
            "npub1zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz3luca5",
            alice.id,
            "self_declared",
            None,
        )
        .unwrap();

    let links = store.list_links(alice.id).unwrap();
    assert_eq!(links.len(), 6);

    // Resolving from any platform returns the same AstridUser.
    for (platform, instance, uid) in &[
        ("discord", None, "719275466049224704"),
        ("slack", Some("T01ACME"), "U01ABC"),
        ("telegram", None, "123456789"),
        ("matrix", None, "@alice:matrix.org"),
        ("email", None, "alice@example.com"),
        (
            "nostr",
            None,
            "npub1zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz3luca5",
        ),
    ] {
        let resolved = store
            .resolve(platform, *instance, uid, None)
            .unwrap()
            .0
            .unwrap();
        assert_eq!(resolved.id, alice.id);
    }
}

// =====================================================================
// Realistic group setting
// =====================================================================

#[test]
fn discord_guild_member_roster_via_context_overlays() {
    // A 5-member Discord guild. Each member is linked with a global
    // username; each also has a per-guild nickname. list_in_context
    // returns the roster paginated.
    let store = make_store();
    let guild = "guild:1078123456789012345";

    let members = [
        ("alice", "AliceTheGamer", "100000000000000001"),
        ("bob", "BobByrd", "100000000000000002"),
        ("charlie", "ChazTheDM", "100000000000000003"),
        ("dave", "Dave_M", "100000000000000004"),
        ("eve", "EveOfTheDawn", "100000000000000005"),
    ];

    for (username, _nickname, snowflake) in &members {
        let user = store.create_user(Some(username)).unwrap();
        store
            .link(
                "discord",
                None,
                snowflake,
                user.id,
                "guild_join",
                Some(username),
            )
            .unwrap();
    }
    for (_, nickname, snowflake) in &members {
        store
            .set_context("discord", None, snowflake, guild, nickname)
            .unwrap();
    }

    // Paginate at 2/page across the 5-member roster.
    let p1 = store
        .list_context_in_context("discord", None, guild, None, Some(2))
        .unwrap();
    let p2 = store
        .list_context_in_context("discord", None, guild, p1.next_cursor.as_deref(), Some(2))
        .unwrap();
    let p3 = store
        .list_context_in_context("discord", None, guild, p2.next_cursor.as_deref(), Some(2))
        .unwrap();
    let total = p1.items.len() + p2.items.len() + p3.items.len();
    assert_eq!(total, 5);
    assert!(p3.next_cursor.is_none());

    // Every roster row carries the resolved AstridUserId.
    for (ci, uid) in p1
        .items
        .iter()
        .chain(p2.items.iter())
        .chain(p3.items.iter())
    {
        assert!(uid.is_some(), "every linked member resolves to a user");
        assert_eq!(ci.platform, "discord");
        assert_eq!(ci.context_id, guild);
    }
}

// =====================================================================
// Bot vs human
// =====================================================================

#[test]
fn bot_account_linked_with_audit_method() {
    // A Discord bot integration: same data shape, just a different
    // `method` for audit.
    let store = make_store();
    let bot = store.create_user(Some("Astrid Bot")).unwrap();
    let link = store
        .link(
            "discord",
            None,
            "999000111222333444",
            bot.id,
            "bot",
            Some("astrid-bot"),
        )
        .unwrap();
    assert_eq!(link.method, "bot");
}
