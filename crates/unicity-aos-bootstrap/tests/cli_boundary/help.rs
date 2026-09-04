use crate::support::*;
use std::fs;

#[test]
fn product_help_version_and_usage_errors_never_delegate() {
    let fixture = Fixture::new("product-roots");
    fixture.install_runtime(RECORDING_RUNTIME);

    for (args, expected_success) in [
        (vec!["--help"], true),
        (vec!["--version"], true),
        (vec!["init", "--help"], true),
        (vec!["init", "--grant-capsules"], false),
        (vec!["init", "--principal", "alice"], false),
        (vec!["migrate"], false),
        (vec!["update", "unexpected"], false),
        (vec!["self-update", "unexpected"], false),
        (vec!["serve-health", "unexpected"], false),
    ] {
        let status = fixture
            .command()
            .args(args)
            .status()
            .expect("run product command");
        assert_eq!(status.success(), expected_success);
        assert!(!fixture.args.exists());
    }
}

#[test]
fn bare_aos_shows_product_help_instead_of_claiming_native_chat() {
    let fixture = Fixture::new("bare-help");
    fixture.install_runtime(RECORDING_RUNTIME);

    let output = fixture.command().output().expect("run bare aos");

    assert!(output.status.success());
    assert!(!fixture.args.exists());
    let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
    assert!(stdout.contains("Running `aos` without a command displays product help"));
}

#[test]
fn runtime_verbs_are_first_class_aos_roots_without_a_nested_namespace() {
    let fixture = Fixture::new("direct-runtime-roots");
    fixture.install_runtime(RECORDING_RUNTIME);
    let contract: toml::Value = include_str!("../../../../release/runtime-command-surface.toml")
        .parse()
        .expect("parse runtime command surface");
    let roots = contract["roots"].as_table().expect("root classifications");
    let direct_roots = ["inherited", "hidden-inherited"]
        .into_iter()
        .flat_map(|bucket| roots[bucket].as_array().expect("root classification list"))
        .map(|root| root.as_str().expect("runtime root string"));

    seed_stopped_volume(&fixture);
    seed_active_receipt(&fixture);

    for root in direct_roots {
        let output = fixture
            .command()
            .args([root, "--aos-direct-root-probe"])
            .output()
            .expect("run direct AOS root");

        assert!(output.status.success(), "direct root failed: {root}");
        assert_eq!(
            fs::read_to_string(&fixture.args).expect("read delegated args"),
            format!("<{root}>\n<--aos-direct-root-probe>\n")
        );
        fs::remove_file(&fixture.args).expect("reset delegated args");
    }
}

#[test]
fn inherited_help_dispatches_byte_for_byte_while_product_help_stays_owned() {
    let fixture = Fixture::new("help-inheritance");
    fixture.install_runtime(RECORDING_RUNTIME);

    for args in [vec!["help", "doctor"], vec!["help", "capsule"]] {
        let output = fixture
            .command()
            .args(&args)
            .output()
            .expect("run inherited help");
        assert!(output.status.success());
        let expected = args
            .iter()
            .map(|argument| format!("<{argument}>\n"))
            .collect::<String>();
        assert_eq!(
            fs::read_to_string(&fixture.args).expect("read delegated help"),
            expected
        );
        fs::remove_file(&fixture.args).expect("reset delegated args");
    }

    for args in [
        vec!["help"],
        vec!["help", "init"],
        vec!["help", "status"],
        vec!["help", "daemon"],
    ] {
        let output = fixture
            .command()
            .args(args)
            .output()
            .expect("run product help");
        assert!(output.status.success());
        assert!(!fixture.args.exists());
    }
}
