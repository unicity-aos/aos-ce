use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

use super::{Fixture, NATIVE_SETUP_RUNTIME};

fn native_setup_args() -> [&'static str; 6] {
    [
        "native-setup",
        "--principal",
        "alice",
        "--confirm-enroll",
        "--confirm-route",
        "--json",
    ]
}

fn native_setup_home_paths(fixture: &Fixture) -> (PathBuf, PathBuf, PathBuf) {
    (
        fixture.home.join("native-input/connection.json"),
        fixture.home.join("runtime/config.toml"),
        fixture.home.join("runtime/keys/local/aos-tray.ed25519"),
    )
}

fn assert_no_token_in_product_output(output: &std::process::Output, receipt: &str) {
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        !stdout.contains("astrid_pair_"),
        "stdout leaked a pairing token"
    );
    assert!(
        !stderr.contains("astrid_pair_"),
        "stderr leaked a pairing token"
    );
    assert!(
        !receipt.contains("astrid_pair_"),
        "receipt leaked a pairing token"
    );
    assert!(!receipt.to_ascii_lowercase().contains("privatekey"));
    assert!(!receipt.to_ascii_lowercase().contains("token"));
}

#[test]
fn native_setup_enrolls_local_personal_device_without_overwriting_or_leaking_tokens() {
    let fixture = Fixture::new("native-setup-success");
    fixture.install_runtime(NATIVE_SETUP_RUNTIME);
    let (connection, config, key) = native_setup_home_paths(&fixture);
    fs::create_dir_all(config.parent().expect("config parent")).expect("config dir");
    fs::write(&config, "strict = true\n").expect("seed unrelated operator config");
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600)).expect("config mode");

    let output = fixture
        .command()
        .args(native_setup_args())
        .output()
        .expect("run native-setup");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let receipt = String::from_utf8(output.stdout.clone()).expect("utf8 receipt");
    let document: serde_json::Value = serde_json::from_str(&receipt).expect("parse receipt");
    assert_eq!(document["scope"], "local-personal");
    assert_eq!(document["authority"], "setup");
    assert_eq!(document["principal"], "alice");
    assert_eq!(document["restartRequired"], true);
    assert_eq!(document["connected"], false);
    assert_eq!(
        document["connectionPath"],
        connection.to_string_lossy().as_ref()
    );
    assert_no_token_in_product_output(&output, &receipt);

    let recorded = fs::read_to_string(&fixture.args).expect("read setup args");
    assert!(recorded.contains("<keypair>\n<generate>\n<--name>\n<aos-tray>\n<--raw>\n"));
    assert!(recorded.contains(
        "<pair-device>\n<issue>\n<--scope>\n<use-only>\n<--label>\n<aos-tray>\n<--raw>\n"
    ));
    assert!(recorded.contains("<pair-device>\n<redeem>\n<--public-key>\n"));
    assert!(recorded.contains("STDIN:astrid_pair_device-token\n"));
    for line in recorded.lines() {
        if line.starts_with('<') {
            assert!(!line.contains("astrid_pair_"));
            assert_ne!(line, "<--force>");
            assert!(!line.contains("token"));
        }
    }

    let connection_mode = fs::metadata(&connection)
        .expect("connection metadata")
        .permissions()
        .mode()
        & 0o777;
    let key_mode = fs::metadata(&key)
        .expect("key metadata")
        .permissions()
        .mode()
        & 0o777;
    let config_mode = fs::metadata(&config)
        .expect("config metadata")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(connection_mode, 0o600);
    assert_eq!(key_mode, 0o600);
    assert_eq!(config_mode, 0o600);
    let connection_json = fs::read_to_string(&connection).expect("read connection");
    assert!(connection_json.contains(r#""principal": "alice""#));
    assert!(!connection_json.contains("astrid_pair_"));
    let config_toml = fs::read_to_string(&config).expect("read operator config");
    assert!(config_toml.contains("strict = true"));
    assert!(config_toml.contains("[native_input.responders]"));
    assert!(config_toml.contains("alice"));
    assert!(config_toml.contains("0123456789abcdef"));

    let again = fixture
        .command()
        .args(native_setup_args())
        .output()
        .expect("refuse second native-setup");
    assert_eq!(again.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&again.stderr).contains("already exists; not overwritten"),
        "stderr: {}",
        String::from_utf8_lossy(&again.stderr)
    );
    assert_eq!(
        fs::read_to_string(&connection).expect("reread connection"),
        connection_json
    );
}

#[test]
fn native_setup_refuses_existing_connection_key_or_responders_without_runtime_pairing() {
    let fixture = Fixture::new("native-setup-existing");
    fixture.install_runtime(NATIVE_SETUP_RUNTIME);
    let (connection, config, key) = native_setup_home_paths(&fixture);

    fs::create_dir_all(connection.parent().expect("connection parent")).expect("connection dir");
    fs::write(&connection, b"keep-me").expect("seed connection");
    fs::set_permissions(&connection, fs::Permissions::from_mode(0o600)).expect("connection mode");
    let existing_connection = fixture
        .command()
        .args(native_setup_args())
        .output()
        .expect("refuse existing connection");
    assert_eq!(existing_connection.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&existing_connection.stderr)
            .contains("a native-input connection already exists; not overwritten")
    );
    assert_eq!(
        fs::read(&connection).expect("preserve connection"),
        b"keep-me"
    );
    assert!(!fixture.args.exists());
    fs::remove_file(&connection).expect("clear connection");

    fs::create_dir_all(key.parent().expect("key parent")).expect("key dir");
    fs::write(&key, [7u8; 32]).expect("seed key");
    fs::set_permissions(&key, fs::Permissions::from_mode(0o600)).expect("key mode");
    let existing_key = fixture
        .command()
        .args(native_setup_args())
        .output()
        .expect("refuse existing key");
    assert_eq!(existing_key.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&existing_key.stderr)
            .contains("device key aos-tray already exists; not overwritten")
    );
    assert_eq!(fs::read(&key).expect("preserve key").len(), 32);
    assert!(!fixture.args.exists());
    fs::remove_file(&key).expect("clear key");

    fs::create_dir_all(config.parent().expect("config parent")).expect("config dir");
    fs::write(
        &config,
        "[native_input.responders]\nbob = \"fedcba9876543210\"\n",
    )
    .expect("seed responders");
    let existing_route = fixture
        .command()
        .args(native_setup_args())
        .output()
        .expect("refuse existing responders");
    assert_eq!(existing_route.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&existing_route.stderr)
            .contains("native-input operator routing already exists; not overwritten")
    );
    assert!(
        fs::read_to_string(&config)
            .expect("preserve config")
            .contains("bob = \"fedcba9876543210\"")
    );
    assert!(!fixture.args.exists());
}

#[test]
fn native_setup_fails_closed_without_confirms_json_or_a_valid_principal() {
    let fixture = Fixture::new("native-setup-invalid");
    fixture.install_runtime(NATIVE_SETUP_RUNTIME);

    let missing = fixture
        .command()
        .args(["native-setup", "--principal", "alice"])
        .output()
        .expect("reject missing confirms");
    assert_eq!(missing.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&missing.stderr)
            .contains("requires --json --confirm-enroll --confirm-route")
    );
    assert!(!fixture.args.exists());

    let missing_principal = fixture
        .command()
        .args([
            "native-setup",
            "--json",
            "--confirm-enroll",
            "--confirm-route",
        ])
        .output()
        .expect("reject missing principal");
    assert_eq!(missing_principal.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&missing_principal.stderr)
            .contains("requires an explicit '--principal PRINCIPAL'")
    );
    assert!(!fixture.args.exists());

    let invalid = fixture
        .command()
        .args([
            "native-setup",
            "--principal",
            "not/a/principal",
            "--confirm-enroll",
            "--confirm-route",
            "--json",
        ])
        .output()
        .expect("reject invalid principal");
    assert_eq!(invalid.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&invalid.stderr).contains("invalid setup principal"));
    assert!(!fixture.args.exists());

    let anonymous = fixture
        .command()
        .args([
            "native-setup",
            "--principal",
            "anonymous",
            "--confirm-enroll",
            "--confirm-route",
            "--json",
        ])
        .output()
        .expect("reject anonymous principal");
    assert_eq!(anonymous.status.code(), Some(2));
    assert!(!fixture.args.exists());

    let conflict = fixture
        .command()
        .args([
            "--principal",
            "alice",
            "native-setup",
            "--principal",
            "bob",
            "--json",
            "--confirm-enroll",
            "--confirm-route",
        ])
        .output()
        .expect("reject duplicate principals");
    assert_eq!(conflict.status.code(), Some(2));
    assert!(!fixture.args.exists());
}

#[test]
fn native_setup_does_not_fall_back_when_pair_device_is_unsupported_or_fails() {
    let fixture = Fixture::new("native-setup-unsupported");
    fixture.install_runtime(NATIVE_SETUP_RUNTIME);

    let unsupported = fixture
        .command()
        .args(native_setup_args())
        .env("AOS_TEST_UNSUPPORTED", "1")
        .output()
        .expect("unsupported native-setup");
    assert_eq!(unsupported.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&unsupported.stderr)
            .contains("native-input setup is not supported by this runtime"),
        "stderr: {}",
        String::from_utf8_lossy(&unsupported.stderr)
    );
    let recorded = fs::read_to_string(&fixture.args).expect("read unsupported args");
    assert!(recorded.contains("<keypair>\n<generate>"));
    assert!(!recorded.contains("<agent>\n<list>"));
    let (connection, _, key) = native_setup_home_paths(&fixture);
    assert!(
        !key.exists(),
        "unsupported generate must not leave a device key"
    );
    assert!(!connection.exists());

    let failed = fixture
        .command()
        .args(native_setup_args())
        .env("AOS_TEST_FAIL", "1")
        .output()
        .expect("failed native-setup");
    assert_eq!(failed.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&failed.stderr)
            .contains("native-input setup failed: principal requires a delegated device"),
        "stderr: {}",
        String::from_utf8_lossy(&failed.stderr)
    );
    assert!(
        !key.exists(),
        "failed pairing after generate must not leave a device key"
    );
    assert!(!connection.exists());
    assert!(!fixture.home.join("runtime/config.toml").exists());
}
