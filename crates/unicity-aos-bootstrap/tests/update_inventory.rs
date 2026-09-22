#![cfg(unix)]
//! Exercise the real CLI with a recording updater, never a live installation.
use std::{
    fs,
    os::unix::fs::PermissionsExt,
    path::PathBuf,
    process::{Command, Output},
};

struct Fixture(PathBuf);
impl Fixture {
    fn new() -> Self {
        let home =
            std::env::temp_dir().join(format!("aos-update-journey-{}", uuid::Uuid::new_v4()));
        fs::create_dir(&home).unwrap();
        fs::set_permissions(&home, fs::Permissions::from_mode(0o700)).unwrap();
        fs::create_dir(home.join("libexec")).unwrap();
        fs::write(home.join("libexec/install.sh"), r#"#!/bin/sh
set -eu
case "$*" in
  *--check*)
    case "$*" in *'--channel dev'*) channel=dev;; *) channel=stable;; esac
    printf '{"schema_version":1,"kind":"aos","installed_version":"%s","channel_version":"2026.10.0","channel":"%s","target":"test","artifact_sha256":"%s","verification":"metadata","binds_candidate_digest":true}\n' "$AOS_INSTALLED_VERSION" "$channel" "${TEST_DIGEST:-aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa}"
    ;;
  *) printf '%s\n' "$*" "$AOS_EXPECTED_ARTIFACT_SHA256" > "$AOS_HOME/applied"; exit "${TEST_APPLY_EXIT:-0}";;
esac
"#).unwrap();
        Self(home)
    }
    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_aos"));
        command
            .env("AOS_HOME", &self.0)
            .env_remove("UNICITY_AOS_INSTALL_METHOD");
        command
    }
    fn run(&self, args: &[&str]) -> Output {
        self.command().args(args).output().unwrap()
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn read_check_confirm_and_apply_are_distinct() {
    let fixture = Fixture::new();
    let output = fixture.run(&["updates", "list", "--json"]);
    assert!(output.status.success());
    assert!(
        !fixture.0.join("update").exists(),
        "list must not write state"
    );
    let check = fixture.run(&["updates", "check", "--channel", "dev", "--json"]);
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );
    let inventory: serde_json::Value = serde_json::from_slice(&check.stdout).unwrap();
    assert_eq!(inventory["items"][0]["availability"], "available");
    assert!(!fixture.0.join("applied").exists());
    assert!(!fixture.run(&["updates", "apply", "all"]).status.success());
    let apply = fixture.run(&["updates", "apply", "all", "--yes", "--json"]);
    assert!(
        apply.status.success(),
        "{}",
        String::from_utf8_lossy(&apply.stderr)
    );
    let result: serde_json::Value = serde_json::from_slice(&apply.stdout).unwrap();
    assert_eq!(result["items"][0]["availability"], "activation_required");
    let args = fs::read_to_string(fixture.0.join("applied")).unwrap();
    assert!(args.contains("--version 2026.10.0"));
    assert!(args.contains(&"a".repeat(64)));
    assert!(!fixture.0.join("runtime").exists());
}

#[test]
fn changed_candidate_refuses_installation() {
    let fixture = Fixture::new();
    assert!(fixture.run(&["updates", "check"]).status.success());
    let output = fixture
        .command()
        .env("TEST_DIGEST", "b".repeat(64))
        .args(["updates", "apply", "aos", "--yes", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(!fixture.0.join("applied").exists());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["items"][0]["availability"], "failed");
}

#[test]
fn refresh_never_selects_a_channel_or_runs_installer_on_first_launch() {
    let fixture = Fixture::new();
    let output = fixture.run(&["updates", "refresh", "--json"]);
    assert!(output.status.success());
    assert!(!fixture.0.join("update").exists());
    assert!(!fixture.0.join("applied").exists());
}

#[test]
fn failure_is_visible_and_retry_requires_fresh_check() {
    let fixture = Fixture::new();
    assert!(fixture.run(&["updates", "check"]).status.success());
    let output = fixture
        .command()
        .env("TEST_APPLY_EXIT", "8")
        .args(["updates", "apply", "aos", "--yes", "--json"])
        .output()
        .unwrap();
    assert!(!output.status.success());
    let result: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(result["items"][0]["availability"], "failed");
    assert!(
        !fixture
            .run(&["updates", "apply", "all", "--yes"])
            .status
            .success()
    );
    assert!(fixture.run(&["updates", "check"]).status.success());
    assert!(
        fixture
            .run(&["updates", "apply", "all", "--yes"])
            .status
            .success()
    );
}

#[test]
fn concurrent_operation_and_symlink_cache_fail_closed() {
    use fs2::FileExt;
    let fixture = Fixture::new();
    assert!(fixture.run(&["updates", "check"]).status.success());
    let state = fixture.0.join("update/command-center");
    let lock = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(state.join("operation.lock"))
        .unwrap();
    lock.lock_exclusive().unwrap();
    assert!(!fixture.run(&["updates", "check"]).status.success());
    drop(lock);
    fs::remove_file(state.join("inventory.json")).unwrap();
    std::os::unix::fs::symlink(
        fixture.0.join("libexec/install.sh"),
        state.join("inventory.json"),
    )
    .unwrap();
    assert!(!fixture.run(&["updates", "list", "--json"]).status.success());
    assert!(
        !fixture
            .run(&["updates", "apply", "all", "--yes"])
            .status
            .success()
    );
}
