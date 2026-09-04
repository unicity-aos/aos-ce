use crate::support::*;
use std::fs;

#[test]
fn explicit_principal_is_accepted_in_either_product_status_position() {
    let fixture = Fixture::new("principal-status");
    fixture.install_runtime(RECORDING_RUNTIME);
    seed_stopped_volume(&fixture);
    seed_active_receipt(&fixture);

    for args in [
        ["--principal", "alice", "status", "--json"],
        ["status", "--principal", "alice", "--json"],
    ] {
        let output = fixture
            .command()
            .args(args)
            .output()
            .expect("run principal-scoped product status");

        assert!(
            output.status.success(),
            "args: {args:?}; stderr: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let status: serde_json::Value =
            serde_json::from_slice(&output.stdout).expect("parse product status JSON");
        assert_eq!(status["state"], "stopped");
    }
    let lifecycle = fs::read_to_string(fixture.root.join("lifecycle")).expect("read lifecycle");
    assert!(lifecycle.contains("start"));
    assert!(lifecycle.contains("stop"));
    fs::remove_file(&fixture.args).expect("reset lifecycle after successful reopen");

    let invalid = fixture
        .command()
        .args(["status", "--principal", "not/a/principal"])
        .output()
        .expect("reject invalid product status principal");
    assert_eq!(invalid.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&invalid.stderr).contains("invalid status principal"),
        "stderr: {}",
        String::from_utf8_lossy(&invalid.stderr)
    );

    let conflict = fixture
        .command()
        .args(["--principal", "alice", "status", "--principal", "bob"])
        .output()
        .expect("reject duplicate product status principals");
    assert_eq!(conflict.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&conflict.stderr)
            .contains("'--principal' was provided both before and after `status`"),
        "stderr: {}",
        String::from_utf8_lossy(&conflict.stderr)
    );
    assert!(
        !fixture.args.exists(),
        "invalid product status invocations must not delegate"
    );
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
fn native_status_reports_stopped_only_after_a_successful_reopen() {
    let fixture = Fixture::new("status");
    fixture.install_runtime(RECORDING_RUNTIME);

    let missing = fixture
        .command()
        .arg("status")
        .output()
        .expect("run aos status without runtime state");
    assert_eq!(missing.status.code(), Some(1));
    let missing_stderr = String::from_utf8(missing.stderr).expect("utf8 stderr");
    assert!(missing_stderr.contains("stopped runtime state is missing"));

    fs::create_dir_all(fixture.home.join("runtime")).expect("create runtime home");
    fs::write(fixture.home.join("runtime/astrid.volume"), b"volume-state")
        .expect("create runtime volume");
    seed_active_receipt(&fixture);

    for args in [vec!["status"], vec!["status", "--json"]] {
        let output = fixture
            .command()
            .args(args)
            .output()
            .expect("run aos status");

        assert!(output.status.success());
        assert!(fixture.args.exists());
        assert!(output.stderr.is_empty());
        let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
        assert!(stdout.contains("stopped"));
        assert!(stdout.contains("0.10.4"));
    }
}

#[test]
fn native_status_fails_a_receipt_less_volume_without_reopening_it() {
    let fixture = Fixture::new("receipt-less-status");
    fixture.install_runtime(RECORDING_RUNTIME);
    seed_stopped_volume(&fixture);

    let output = fixture
        .command()
        .arg("status")
        .output()
        .expect("read status");
    assert_eq!(output.status.code(), Some(1));
    assert!(!fixture.args.exists());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("active distribution receipt is missing")
    );
}
