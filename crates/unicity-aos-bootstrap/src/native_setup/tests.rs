use super::*;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;

fn principal() -> PrincipalId {
    PrincipalId::new("alice").expect("valid principal")
}

fn temp_root(label: &str) -> PathBuf {
    let root = std::env::temp_dir().join(format!(
        "aos-native-setup-{label}-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    ));
    fs::create_dir_all(&root).expect("temp");
    root
}

fn failed(error: SetupError) -> String {
    match error {
        SetupError::Failed(message) => message,
        SetupError::Unsupported => panic!("expected a failed setup, not unsupported"),
    }
}

#[test]
fn pairing_args_never_include_a_token_or_force_flag() {
    let principal = principal();
    let generate: Vec<String> = runtime_generate_args(&principal)
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let issue: Vec<String> = runtime_issue_args(&principal)
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    let redeem: Vec<String> = runtime_redeem_args(&principal, &"ab".repeat(32))
        .expect("valid public key")
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    assert_eq!(
        generate,
        [
            "--principal",
            "alice",
            "keypair",
            "generate",
            "--name",
            "aos-tray",
            "--raw"
        ]
    );
    assert_eq!(
        issue,
        [
            "--principal",
            "alice",
            "pair-device",
            "issue",
            "--scope",
            "use-only",
            "--label",
            "aos-tray",
            "--raw"
        ]
    );
    assert_eq!(
        redeem,
        [
            "--principal",
            "alice",
            "pair-device",
            "redeem",
            "--public-key",
            &"ab".repeat(32)
        ]
    );
    for args in [&generate, &issue, &redeem] {
        assert!(!args.iter().any(|arg| arg.contains("astrid_pair_")));
        assert!(!args.iter().any(|arg| arg == "--force"));
        assert!(!args.iter().any(|arg| arg.contains("token")));
    }
}

#[test]
fn product_paths_stay_under_the_aos_home() {
    let home = AosHome::from_root("/tmp/aos-setup-home");
    assert_eq!(
        connection_path(&home),
        PathBuf::from("/tmp/aos-setup-home/native-input/connection.json")
    );
    assert_eq!(
        operator_config_path(&home),
        PathBuf::from("/tmp/aos-setup-home/runtime/config.toml")
    );
    assert_eq!(
        private_key_path(&home),
        PathBuf::from("/tmp/aos-setup-home/runtime/keys/local/aos-tray.ed25519")
    );
    assert_eq!(
        socket_path(&home),
        PathBuf::from("/tmp/aos-setup-home/run/system.sock")
    );
    assert_eq!(
        token_path(&home),
        PathBuf::from("/tmp/aos-setup-home/run/system.token")
    );
    assert_ne!(
        socket_path(&home),
        home.runtime_home().join("run/system.sock")
    );
}

#[test]
fn responder_insert_is_narrow_and_refuses_existing_native_input() {
    let principal = principal();
    let written = insert_responder("", &principal, "0123456789abcdef").expect("insert");
    assert!(written.contains("[native_input.responders]"));
    assert!(written.contains("alice"));
    assert!(written.contains("0123456789abcdef"));

    let preserved = insert_responder("strict = true\n", &principal, "0123456789abcdef")
        .expect("insert beside unrelated keys");
    assert!(preserved.contains("strict = true"));
    assert!(preserved.contains("[native_input.responders]"));

    let error = insert_responder(
        "[native_input.responders]\nbob = 'fedcba9876543210'\n",
        &principal,
        "0123456789abcdef",
    )
    .expect_err("existing native_input");
    assert!(failed(error).contains("already exists; not overwritten"));
}

#[test]
fn public_key_token_and_redeem_parsing_stay_strict() {
    let principal = principal();
    assert_eq!(
        parse_public_key(format!("{}\n", "ab".repeat(32)).as_bytes()).expect("key"),
        "ab".repeat(32)
    );
    assert!(parse_public_key(b"short").is_err());
    assert!(parse_pair_token(b"astrid_pair_device-token\n").is_ok());
    assert!(parse_pair_token(b"astrid_inv_device-token\n").is_err());
    let redeemed = parse_redeemed(
        br#"{"principal":"alice","public_key_fingerprint":"blake3:aa","key_id":"0123456789abcdef"}"#,
        &principal,
    )
    .expect("redeem");
    assert_eq!(redeemed.key_id, "0123456789abcdef");
    assert!(parse_redeemed(
        br#"{"principal":"bob","public_key_fingerprint":"blake3:aa","key_id":"0123456789abcdef"}"#,
        &principal,
    )
    .is_err());
}

#[test]
fn unsupported_runtime_is_classified_without_a_global_fallback() {
    let unsupported = Output {
        status: ExitStatusExt::from_raw(2 << 8),
        stdout: Vec::new(),
        stderr: b"error: unexpected argument '--scope' found".to_vec(),
    };
    assert!(runtime_unsupported(&unsupported));
    let failed_output = Output {
        status: ExitStatusExt::from_raw(1 << 8),
        stdout: Vec::new(),
        stderr: b"principal requires a delegated device".to_vec(),
    };
    assert!(!runtime_unsupported(&failed_output));
}

#[test]
fn private_connection_file_is_created_0600_and_not_overwritten() {
    let root = temp_root("connection-mode");
    let path = root.join("connection.json");
    write_private_create_new(&path, b"{\"principal\":\"alice\"}").expect("write");
    let mode = fs::metadata(&path).expect("stat").permissions().mode() & 0o777;
    assert_eq!(mode, 0o600);
    let error = write_private_create_new(&path, b"other").expect_err("overwrite");
    assert!(failed(error).contains("already exists"));
    assert_eq!(fs::read(&path).expect("read"), b"{\"principal\":\"alice\"}");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn owned_cleanup_removes_only_files_this_attempt_created() {
    let root = temp_root("owned-cleanup");
    let connection = root.join("connection.json");
    let key = root.join("aos-tray.ed25519");
    let public_hex = root.join("aos-tray.pub.hex");
    let meta = root.join("aos-tray.meta.toml");
    let keep = root.join("unrelated");
    fs::write(&connection, b"foreign-connection").expect("connection");
    fs::write(&key, [1u8; 32]).expect("key");
    fs::write(&public_hex, b"ab").expect("pub");
    fs::write(&meta, b"note = true\n").expect("meta");
    fs::write(&keep, b"keep").expect("keep");

    let artifacts = KeyArtifacts::from_private(key.clone()).expect("artifacts");
    let before = KeyPresence {
        private: false,
        public_hex: false,
        meta: false,
    };
    let mut owned = OwnedCleanup::default();
    owned
        .claim_new_keys(&artifacts, &before)
        .expect("claim new keys");
    owned.rollback();

    assert_eq!(
        fs::read(&connection).expect("foreign connection survived"),
        b"foreign-connection"
    );
    assert!(!key.exists());
    assert!(!public_hex.exists());
    assert!(!meta.exists());
    assert_eq!(fs::read(&keep).expect("kept"), b"keep");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn preexisting_key_artifacts_are_not_claimed_for_cleanup() {
    let root = temp_root("preexisting-not-claimed");
    let key = root.join("aos-tray.ed25519");
    let public_hex = root.join("aos-tray.pub.hex");
    let meta = root.join("aos-tray.meta.toml");
    fs::write(&key, [3u8; 32]).expect("key");
    fs::write(&public_hex, b"keep-pub").expect("pub");
    fs::write(&meta, b"keep = true\n").expect("meta");

    let artifacts = KeyArtifacts::from_private(key.clone()).expect("artifacts");
    let before = artifacts.presence().expect("presence");
    assert!(before.any());
    let mut owned = OwnedCleanup::default();
    owned
        .claim_new_keys(&artifacts, &before)
        .expect("claim none");
    owned.rollback();

    assert_eq!(fs::read(&key).expect("preserve key").len(), 32);
    assert_eq!(fs::read(&public_hex).expect("preserve pub"), b"keep-pub");
    assert_eq!(fs::read(&meta).expect("preserve meta"), b"keep = true\n");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn preexisting_public_and_meta_sidecars_survive_failed_setup() {
    let root = temp_root("preexisting-sidecars");
    let home = AosHome::from_root(&root);
    let artifacts = KeyArtifacts::from_private(private_key_path(&home)).expect("artifacts");
    fs::create_dir_all(artifacts.private.parent().expect("key dir")).expect("key dir");
    fs::write(&artifacts.public_hex, b"preexisting-public").expect("pub");
    fs::write(&artifacts.meta, b"note = \"keep\"\n").expect("meta");

    let error = run(&home, &principal()).expect_err("preexisting sidecars");
    assert!(failed(error).contains("device key aos-tray already exists; not overwritten"));
    assert_eq!(
        fs::read(&artifacts.public_hex).expect("preserve public"),
        b"preexisting-public"
    );
    assert_eq!(
        fs::read(&artifacts.meta).expect("preserve meta"),
        b"note = \"keep\"\n"
    );
    assert!(!artifacts.private.exists());
    assert!(!connection_path(&home).exists());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn preexisting_connection_survives_failed_setup() {
    let root = temp_root("preexisting-connection");
    let home = AosHome::from_root(&root);
    let connection = connection_path(&home);
    fs::create_dir_all(connection.parent().expect("connection dir")).expect("connection dir");
    fs::write(&connection, b"keep-me").expect("seed connection");

    let error = run(&home, &principal()).expect_err("preexisting connection");
    assert!(failed(error).contains("a native-input connection already exists; not overwritten"));
    assert_eq!(
        fs::read(&connection).expect("preserve connection"),
        b"keep-me"
    );
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn failed_setup_cleans_only_newly_created_key_files() {
    let root = temp_root("new-files-only");
    let connection = root.join("connection.json");
    let key = root.join("aos-tray.ed25519");
    let public_hex = root.join("aos-tray.pub.hex");
    let meta = root.join("aos-tray.meta.toml");
    let keep = root.join("unrelated");
    fs::write(&connection, b"already-there").expect("connection");
    fs::write(&keep, b"keep").expect("keep");
    fs::write(&key, [9u8; 32]).expect("new key");
    fs::write(&public_hex, b"new-pub").expect("new pub");
    fs::write(&meta, b"new = true\n").expect("new meta");

    let artifacts = KeyArtifacts::from_private(key.clone()).expect("artifacts");
    let before = KeyPresence {
        private: false,
        public_hex: false,
        meta: false,
    };
    let mut owned = OwnedCleanup::default();
    owned.claim_new_keys(&artifacts, &before).expect("claim");
    // Connection existed before this attempt, so it is not claimed.
    owned.rollback();

    assert_eq!(fs::read(&connection).expect("connection"), b"already-there");
    assert!(!key.exists());
    assert!(!public_hex.exists());
    assert!(!meta.exists());
    assert_eq!(fs::read(&keep).expect("kept"), b"keep");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn setup_lock_release_does_not_wait_for_duplicate_descriptor() {
    let root = temp_root("setup-lock-duplicate");
    let home = AosHome::from_root(&root);
    let held = SetupLock::acquire(&home).expect("first lock");
    // A concurrent fork can temporarily retain the same open file description
    // until exec closes CLOEXEC descriptors. Model that lifetime deterministically.
    let duplicate = held._file.try_clone().expect("duplicate descriptor");
    assert!(SetupLock::acquire(&home).is_err());
    drop(held);
    let released = SetupLock::acquire(&home).expect("guard releases lock immediately");
    drop(duplicate);
    assert!(
        SetupLock::acquire(&home).is_err(),
        "new owner remains locked"
    );
    drop(released);
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn setup_lock_serializes_the_operation() {
    let root = temp_root("setup-lock");
    let home = AosHome::from_root(&root);
    let held = SetupLock::acquire(&home).expect("first lock");
    let error = run(&home, &principal()).expect_err("serialized setup");
    assert!(failed(error).contains("another native-input setup is already in progress"));
    drop(held);
    let _released = SetupLock::acquire(&home).expect("lock released");
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn tmp_alias_home_canonicalizes_without_mutation() {
    let name = format!(
        "aos-native-setup-canon-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .expect("clock")
            .as_nanos()
    );
    let alias = PathBuf::from("/tmp").join(&name);
    assert!(!alias.exists());
    let canonical = canonical_setup_home(&alias).expect("canonicalize alias");
    let expected = fs::canonicalize("/tmp").expect("tmp").join(&name);
    assert_eq!(canonical, expected);
    assert!(!alias.exists());
    assert!(!expected.exists());
}

#[test]
fn dangling_symlink_home_fails_closed_without_mutation() {
    let root = temp_root("dangle-parent");
    let missing = root.join("missing-target");
    let dangle = root.join("dangle");
    std::os::unix::fs::symlink(&missing, &dangle).expect("dangling symlink");
    let before: Vec<_> = fs::read_dir(&root)
        .expect("list")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    let error = run(&AosHome::from_root(&dangle), &principal()).expect_err("dangle");
    assert!(
        failed(error).contains("AOS_HOME cannot be resolved"),
        "expected resolve failure"
    );
    let after: Vec<_> = fs::read_dir(&root)
        .expect("list")
        .map(|entry| entry.expect("entry").file_name())
        .collect();
    assert_eq!(before, after);
    assert!(!missing.exists());
    assert!(!dangle.join("native-input").exists());
    let _ = fs::remove_dir_all(&root);
}

#[test]
fn dot_and_parent_components_are_rejected_before_pairing() {
    let root = temp_root("dot-components");
    let dotted = root.join(".").join("nested");
    let parented = root.join("child").join("..").join("other");
    assert!(canonical_setup_home(&dotted).is_err());
    assert!(canonical_setup_home(&parented).is_err());
    assert!(fs::read_dir(&root).expect("list").next().is_none());
    let _ = fs::remove_dir_all(&root);
}
