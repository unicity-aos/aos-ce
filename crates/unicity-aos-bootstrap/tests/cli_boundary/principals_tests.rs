use std::fs;

use super::{DISCOVERY_RUNTIME, Fixture, RECORDING_RUNTIME};

fn assert_discovery_args(args: &str, principal: &str) {
    assert_eq!(
        args,
        format!("<--principal>\n<{principal}>\n<agent>\n<list>\n<--mine>\n<--format>\n<json>\n")
    );
    assert!(args.contains("<--mine>\n"));
    assert_ne!(args, "<agent>\n<list>\n<--format>\n<json>\n");
}

#[test]
fn malformed_or_ambiguous_product_principals_never_delegate() {
    let fixture = Fixture::new("malformed-principals");
    fixture.install_runtime(RECORDING_RUNTIME);

    for args in [
        vec!["--principal", "init"],
        vec!["--principal", "init", "--yes"],
        vec!["--principal=", "init"],
        vec!["--principal", "operator", "init", "--target-principal"],
        vec!["--principal", "operator", "init", "--target-principal="],
        vec!["--principal", "operator", "--principal", "other", "init"],
    ] {
        let output = fixture
            .command()
            .args(args)
            .output()
            .expect("run malformed product invocation");

        assert_eq!(output.status.code(), Some(2));
        assert!(!fixture.args.exists());
    }
}

#[test]
fn owned_principal_discovery_always_uses_mine_and_never_falls_back_globally() {
    let fixture = Fixture::new("principals-mine");
    fixture.install_runtime(DISCOVERY_RUNTIME);

    let empty = fixture
        .command()
        .args(["principals", "--json"])
        .output()
        .expect("discover empty owned directory");
    assert!(
        empty.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&empty.stderr)
    );
    let document: serde_json::Value =
        serde_json::from_slice(&empty.stdout).expect("parse owned discovery");
    assert_eq!(document["scope"], "owned");
    assert_eq!(document["authority"], "discovery");
    assert_eq!(document["principals"], serde_json::json!([]));
    assert_discovery_args(
        &fs::read_to_string(&fixture.args).expect("read discovery args"),
        "default",
    );

    let named = fixture
        .command()
        .args(["--principal", "operator", "principals", "--json"])
        .env(
            "AOS_TEST_STDOUT",
            r#"[{"principal":"alice","enabled":true},{"principal":"default","enabled":false}]"#,
        )
        .output()
        .expect("discover named operator directory");
    assert!(
        named.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&named.stderr)
    );
    let document: serde_json::Value =
        serde_json::from_slice(&named.stdout).expect("parse named owned discovery");
    assert_eq!(document["scope"], "owned");
    assert_eq!(document["authority"], "discovery");
    assert_eq!(
        document["principals"],
        serde_json::json!([{"id":"alice","enabled":true},{"id":"default","enabled":false}])
    );
    assert!(document["principals"][0].get("groups").is_none());
    assert_discovery_args(
        &fs::read_to_string(&fixture.args).expect("read operator discovery args"),
        "operator",
    );
}

#[test]
fn owned_principal_discovery_fails_closed_without_json_or_a_valid_operator() {
    let fixture = Fixture::new("principals-invalid");
    fixture.install_runtime(DISCOVERY_RUNTIME);

    let missing_json = fixture
        .command()
        .args(["principals"])
        .output()
        .expect("reject principals without json");
    assert_eq!(missing_json.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&missing_json.stderr).contains("requires --json"),
        "stderr: {}",
        String::from_utf8_lossy(&missing_json.stderr)
    );
    assert!(!fixture.args.exists());

    let invalid = fixture
        .command()
        .args(["principals", "--json", "--principal", "not/a/principal"])
        .output()
        .expect("reject invalid discovery principal");
    assert_eq!(invalid.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&invalid.stderr).contains("invalid discovery principal"),
        "stderr: {}",
        String::from_utf8_lossy(&invalid.stderr)
    );
    assert!(!fixture.args.exists());

    let conflict = fixture
        .command()
        .args([
            "--principal",
            "alice",
            "principals",
            "--json",
            "--principal",
            "bob",
        ])
        .output()
        .expect("reject duplicate discovery principals");
    assert_eq!(conflict.status.code(), Some(2));
    assert!(!fixture.args.exists());
}

#[test]
fn owned_principal_discovery_does_not_retry_without_mine_when_unsupported_or_failed() {
    let fixture = Fixture::new("principals-unsupported");
    fixture.install_runtime(DISCOVERY_RUNTIME);

    let unsupported = fixture
        .command()
        .args(["principals", "--json"])
        .env("AOS_TEST_EXIT", "2")
        .env(
            "AOS_TEST_STDERR",
            "error: unexpected argument '--mine' found",
        )
        .output()
        .expect("unsupported owned discovery");
    assert_eq!(unsupported.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&unsupported.stderr)
            .contains("owned-principal discovery is not supported by this runtime"),
        "stderr: {}",
        String::from_utf8_lossy(&unsupported.stderr)
    );
    assert_discovery_args(
        &fs::read_to_string(&fixture.args).expect("read unsupported discovery args"),
        "default",
    );

    let failed = fixture
        .command()
        .args(["principals", "--json"])
        .env("AOS_TEST_EXIT", "1")
        .env(
            "AOS_TEST_STDERR",
            "principal discovery requires a user-delegated device",
        )
        .output()
        .expect("failed owned discovery");
    assert_eq!(failed.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&failed.stderr).contains(
            "owned-principal discovery failed: principal discovery requires a user-delegated device"
        ),
        "stderr: {}",
        String::from_utf8_lossy(&failed.stderr)
    );
    assert_discovery_args(
        &fs::read_to_string(&fixture.args).expect("read failed discovery args"),
        "default",
    );
}
