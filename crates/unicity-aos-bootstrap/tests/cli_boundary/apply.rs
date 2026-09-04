use crate::support::*;
use std::fs;
use std::path::Path;

fn state(fixture: &super::support::Fixture, name: &str) -> String {
    fs::read_to_string(fixture.root.join(name)).expect("read runtime projection state")
}

#[test]
fn runtime_child_uses_product_run_dir_over_an_inherited_override() {
    let fixture = Fixture::new("run-dir-override");
    fixture.install_runtime(RECORDING_RUNTIME);
    let hostile = fixture.root.join("hostile-run");
    fs::create_dir_all(&hostile).expect("create hostile inherited run directory");
    let recorded_prefix = fixture.root.join("runtime-run-dir");

    let status = fixture
        .command()
        .env("ASTRID_RUN_DIR", &hostile)
        .env("AOS_TEST_RUN_DIR_PREFIX", &recorded_prefix)
        .args(["--principal", "alice", "distro", "apply", "--yes"])
        .status()
        .expect("run apply with hostile inherited run directory");
    assert!(status.success());

    let expected = format!("{}\n", fixture.home.join("run").display());
    for command in ["start", "apply", "stop"] {
        assert_eq!(
            fs::read_to_string(fixture.root.join(format!("runtime-run-dir.{command}")))
                .expect("read child run directory"),
            expected,
            "child {command} must use the product-owned run directory"
        );
    }
    assert!(
        fs::read_dir(&hostile)
            .expect("read hostile inherited run directory")
            .next()
            .is_none(),
        "inherited run directory must remain unused"
    );
}

#[test]
fn distribution_apply_seeds_missing_mounted_pin_before_dispatch() {
    let fixture = Fixture::new("missing-mounted-pin");
    fixture.install_runtime(RECORDING_RUNTIME);
    let selected = fixture.selected_distro();
    let expected_key = unicity_aos_bootstrap::distro_trust::selected_signing_key(&selected)
        .expect("read packaged distribution signing key");
    let pin_prefix = fixture.root.join("runtime-pin");

    let status = fixture
        .command()
        .env("AOS_TEST_NO_MOUNTED_PIN", "1")
        .env(
            "AOS_TEST_RUNTIME_STATE_PREFIX",
            fixture.root.join("runtime-state"),
        )
        .env("AOS_TEST_RUNTIME_PIN_PREFIX", &pin_prefix)
        .args(["--principal", "alice", "distro", "apply", "--yes"])
        .status()
        .expect("run apply without a pre-existing mounted pin");
    assert!(status.success());

    assert_eq!(
        fs::read_to_string(fixture.root.join("runtime-pin.start"))
            .expect("read start pin projection"),
        "",
        "the fake runtime must not pre-seed the mounted pin"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("runtime-pin.apply"))
            .expect("read apply pin projection")
            .trim(),
        expected_key,
        "AOS must seed the packaged authorized identity on the mounted projection"
    );
    assert!(
        fs::read_to_string(fixture.root.join("apply-args"))
            .expect("read runtime dispatch args")
            .contains("<distro>"),
        "AOS must dispatch only after seeding the missing mounted pin"
    );
    assert!(state(&fixture, "runtime-state.apply").contains("trust"));
    assert!(!fixture.home.join("trust").exists());
}

#[test]
fn selected_distribution_apply_is_the_single_transaction() {
    let fixture = Fixture::new("distro-apply-default");
    fixture.install_runtime(RECORDING_RUNTIME);
    let selected = fixture.selected_distro();

    let status = fixture
        .command()
        .env(
            "AOS_TEST_RUNTIME_STATE_PREFIX",
            fixture.root.join("runtime-state"),
        )
        .env("AOS_TEST_SELF", fixture.root.join("runtime-self"))
        .args([
            "--principal",
            "alice",
            "distro",
            "apply",
            "--offline",
            "--yes",
            "--var",
            "model=gpt-5",
        ])
        .status()
        .expect("run canonical product distribution apply");
    assert!(status.success());
    let args = fs::read_to_string(fixture.root.join("apply-args")).expect("read apply args");
    assert_eq!(
        args,
        format!(
            "<--principal>\n<alice>\n<distro>\n<apply>\n<{}>\n<--yes>\n<--offline>\n<--var>\n<model=gpt-5>\n",
            selected.display()
        )
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("child-client-config")).expect("read client config"),
        format!(
            "{}\n",
            fixture.home.join("etc/astrid/client.toml").display()
        )
    );
    assert!(
        !fixture.bootstrap_args.exists(),
        "there must be no second init pass"
    );
    assert!(!args.contains("--grant-capsules"));
    assert_eq!(
        fs::read_to_string(fixture.root.join("child-distro")).expect("read enforced distro"),
        format!("{}\n", fixture.selected_distro().display())
    );
    assert_eq!(
        state(&fixture, "runtime-state.start"),
        "",
        "AOS must leave the runtime home childless before mount"
    );
    assert!(state(&fixture, "runtime-state.apply").contains("trust"));
    assert!(state(&fixture, "runtime-state.stop-before").contains("trust"));
    assert!(!fixture.home.join("runtime/bin").exists());
    let selected_runtime =
        fs::read_to_string(fixture.root.join("runtime-self")).expect("read selected runtime path");
    assert_eq!(Path::new(selected_runtime.trim()), fixture.runtime);
    let manifest: toml::Value = fs::read_to_string(fixture.selected_distro())
        .expect("read enforced manifest")
        .parse()
        .expect("parse enforced manifest");
    for capsule in manifest["capsule"].as_array().expect("manifest capsules") {
        let source = Path::new(capsule["source"].as_str().expect("capsule source"));
        assert!(
            source.is_relative(),
            "packaged capsule sources stay relative"
        );
        assert!(
            fixture
                .selected_distro()
                .parent()
                .expect("release root")
                .join(source)
                .is_file()
        );
    }
    let alias_args =
        fs::read_to_string(fixture.root.join("apply-args")).expect("read canonical transaction");
    fs::remove_file(fixture.root.join("apply-args")).expect("reset recording between alias runs");
    let status = fixture
        .command()
        .args([
            "--principal",
            "alice",
            "init",
            "--offline",
            "--yes",
            "--var",
            "model=gpt-5",
        ])
        .status()
        .expect("run compatibility alias");
    assert!(status.success());
    assert_eq!(
        alias_args,
        fs::read_to_string(fixture.root.join("apply-args")).expect("read alias transaction")
    );
}

#[test]
fn clean_home_apply_never_provisions_a_compatibility_bin_directory() {
    let fixture = Fixture::new("clean-apply-layout");
    fixture.install_runtime(RECORDING_RUNTIME);
    assert!(!fixture.home.join("runtime").exists());

    let status = fixture
        .command()
        .env(
            "AOS_TEST_RUNTIME_STATE_PREFIX",
            fixture.root.join("runtime-state"),
        )
        .env("AOS_TEST_SELF", fixture.root.join("runtime-self"))
        .args([
            "--principal",
            "alice",
            "distro",
            "apply",
            "--offline",
            "--yes",
        ])
        .status()
        .expect("run clean-home distribution apply");

    assert!(status.success());
    let stopped_entries: Vec<_> = fs::read_dir(fixture.home.join("runtime"))
        .expect("read stopped runtime")
        .collect();
    assert_eq!(stopped_entries.len(), 1);
    assert!(
        fixture.home.join("runtime/astrid.volume").is_file(),
        "stopped apply must leave exactly the durable volume"
    );
    assert_eq!(
        state(&fixture, "runtime-state.start"),
        "",
        "AOS must create no runtime children before mount"
    );
    let selected_runtime =
        fs::read_to_string(fixture.root.join("runtime-self")).expect("read selected runtime path");
    assert_eq!(Path::new(selected_runtime.trim()), fixture.runtime);
}

#[test]
fn apply_fails_closed_without_dispatch_on_a_foreign_mounted_pin() {
    let fixture = Fixture::new("foreign-pin");
    fixture.install_runtime(RECORDING_RUNTIME);

    let output = fixture
        .command()
        .env("AOS_TEST_FOREIGN_PIN", "1")
        .args(["--principal", "alice", "distro", "apply", "--yes"])
        .output()
        .expect("run foreign-pin apply");

    assert!(!output.status.success());
    let args = fs::read_to_string(&fixture.args).expect("read lifecycle args");
    assert!(!args.contains("<distro>"), "foreign pin dispatched: {args}");
}

#[test]
fn apply_fails_closed_without_dispatch_on_a_symlinked_pin() {
    let fixture = Fixture::new("symlink-pin");
    fixture.install_runtime(RECORDING_RUNTIME);
    let target_dir = fixture.root.join("pin-target");

    let output = fixture
        .command()
        .env("AOS_TEST_SYMLINK_PIN", "1")
        .env("AOS_TEST_PIN_TARGET_DIR", &target_dir)
        .args(["--principal", "alice", "distro", "apply", "--yes"])
        .output()
        .expect("run symlink-pin apply");

    assert!(!output.status.success());
    let args = fs::read_to_string(&fixture.args).expect("read lifecycle args");
    assert!(
        !args.contains("<distro>"),
        "symlinked pin dispatched: {args}"
    );
}

#[test]
fn distribution_apply_stops_on_runtime_failure_without_a_second_pass() {
    let fixture = Fixture::new("init-bootstrap-failure");
    fixture.install_runtime(RECORDING_RUNTIME);

    let output = fixture
        .command()
        .env("AOS_TEST_EXIT", "42")
        .args(["--principal", "alice", "distro", "apply"])
        .output()
        .expect("run distribution apply with a failing runtime");

    assert!(!output.status.success());
    assert!(fixture.args.exists());
    assert!(!fixture.bootstrap_args.exists());
}

#[test]
fn distribution_apply_requires_an_explicit_principal() {
    let fixture = Fixture::new("distro-apply-principal-required");
    fixture.install_runtime(RECORDING_RUNTIME);

    let output = fixture
        .command()
        .args(["distro", "apply", "--yes"])
        .output()
        .expect("run distribution apply without a principal");

    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("`--principal PRINCIPAL` is required")
    );
    assert!(!fixture.args.exists());
}

#[test]
fn distribution_apply_fails_closed_without_signature_siblings() {
    let fixture = Fixture::new("distro-apply-signature-required");
    fixture.install_runtime(RECORDING_RUNTIME);
    fs::remove_file(
        fixture
            .home
            .join("releases")
            .join(env!("CARGO_PKG_VERSION"))
            .join("Distro.sig"),
    )
    .expect("remove fixture signature");

    let output = fixture
        .command()
        .args(["--principal", "alice", "distro", "apply", "--yes"])
        .output()
        .expect("run distribution apply without signature");

    assert_eq!(output.status.code(), Some(1));
    assert!(!fixture.args.exists());
}

#[test]
fn distribution_apply_preserves_explicit_principal() {
    let fixture = Fixture::new("distro-apply-principals");
    fixture.install_runtime(RECORDING_RUNTIME);
    let selected = fixture.selected_distro();

    let status = fixture
        .command()
        .args(["--principal", "alice", "distro", "apply", "--yes"])
        .status()
        .expect("run distribution apply with explicit principals");
    assert!(status.success());
    assert_eq!(
        fs::read_to_string(fixture.root.join("apply-args")).expect("read explicit-principal args"),
        format!(
            "<--principal>\n<alice>\n<distro>\n<apply>\n<{}>\n<--yes>\n",
            selected.display()
        )
    );
}

#[test]
fn selected_apply_keeps_the_runtime_offline_flag_and_uses_only_local_capsules() {
    let fixture = Fixture::new("distro-offline");
    fixture.install_runtime(RECORDING_RUNTIME);

    let status = fixture
        .command()
        .args(["--principal", "alice", "distro", "apply", "--offline"])
        .status()
        .expect("run offline distribution apply");
    assert!(status.success());
    let selected = fixture.selected_distro();
    assert_eq!(
        fs::read_to_string(fixture.root.join("apply-args")).expect("read offline args"),
        format!(
            "<--principal>\n<alice>\n<distro>\n<apply>\n<{}>\n<--yes>\n<--offline>\n",
            selected.display()
        )
    );
    let manifest_path = fixture.selected_distro();
    let manifest: toml::Value = fs::read_to_string(manifest_path)
        .expect("read materialized manifest")
        .parse()
        .expect("parse materialized manifest");
    let capsules = manifest["capsule"].as_array().expect("capsule entries");
    let embedded: toml::Value =
        include_str!("../../../../distros/community/unicity-ce/Distro.toml")
            .parse()
            .expect("parse embedded distro fixture");
    assert_eq!(
        capsules.len(),
        embedded["capsule"]
            .as_array()
            .expect("embedded capsule entries")
            .len()
    );
    assert!(capsules.iter().all(|capsule| {
        let source = Path::new(capsule["source"].as_str().expect("source"));
        source.is_relative()
            && fixture
                .selected_distro()
                .parent()
                .expect("release root")
                .join(source)
                .is_file()
    }));
}

#[test]
fn package_manager_capsule_override_is_absolute_exact_and_enforced() {
    let fixture = Fixture::new("capsule-override");
    fixture.install_runtime(RECORDING_RUNTIME);
    let custom = fixture.root.join("homebrew/libexec/capsules");
    fs::create_dir_all(custom.parent().expect("custom capsule parent"))
        .expect("create custom capsule parent");
    fs::rename(fixture.default_capsule_dir(), &custom).expect("move capsules to package prefix");

    let output = fixture
        .command()
        .env("UNICITY_AOS_CAPSULE_DIR", &custom)
        .arg("doctor")
        .output()
        .expect("run with package-manager capsule directory");
    assert!(output.status.success());
    let manifest: toml::Value = fs::read_to_string(fixture.selected_distro())
        .expect("read materialized override manifest")
        .parse()
        .expect("parse materialized override manifest");
    assert!(
        manifest["capsule"]
            .as_array()
            .expect("capsules")
            .iter()
            .all(|capsule| Path::new(capsule["source"].as_str().expect("source")).is_relative())
    );

    fs::remove_file(&fixture.args).expect("reset delegated args");
    let invalid = fixture
        .command()
        .env("UNICITY_AOS_CAPSULE_DIR", "relative/capsules")
        .arg("doctor")
        .output()
        .expect("run invalid override");
    assert!(!invalid.status.success());
    assert!(!fixture.args.exists());
    assert!(
        String::from_utf8(invalid.stderr)
            .expect("utf8 stderr")
            .contains("UNICITY_AOS_CAPSULE_DIR must be an absolute path")
    );
}

#[test]
fn init_alias_preserves_the_explicit_principal() {
    let fixture = Fixture::new("init-operator-target");
    fixture.install_runtime(RECORDING_RUNTIME);
    let selected = fixture.selected_distro();

    let status = fixture
        .command()
        .args(["--principal", "operator", "init", "--yes"])
        .status()
        .expect("run product init with an explicit operator");

    assert!(status.success());
    assert_eq!(
        fs::read_to_string(fixture.root.join("apply-args")).expect("read operator init args"),
        format!(
            "<--principal>\n<operator>\n<distro>\n<apply>\n<{}>\n<--yes>\n",
            selected.display()
        )
    );
}

#[test]
fn product_init_rejects_caller_distro_selection() {
    let fixture = Fixture::new("init-distro-override");
    fixture.install_runtime(RECORDING_RUNTIME);

    let output = fixture
        .command()
        .args(["--principal", "alice", "init", "/other/Distro.toml"])
        .output()
        .expect("run protected init");
    assert_eq!(output.status.code(), Some(1));
    assert!(!fixture.args.exists());
    assert!(
        String::from_utf8(output.stderr)
            .expect("utf8 stderr")
            .contains("failed to select the requested distribution")
    );
}
