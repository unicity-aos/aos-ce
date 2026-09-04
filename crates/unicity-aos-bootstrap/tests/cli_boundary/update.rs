use crate::support::*;
use std::fs;
use std::os::unix::fs::PermissionsExt;

#[test]
fn direct_update_fails_closed_without_an_installed_trusted_updater() {
    let fixture = Fixture::new("direct-update");
    fixture.install_runtime(RECORDING_RUNTIME);

    for alias in ["update", "self-update", "self_update"] {
        let output = fixture
            .command()
            .env_remove("UNICITY_AOS_INSTALL_METHOD")
            .arg(alias)
            .output()
            .expect("run direct update without installed updater");

        assert!(!output.status.success());
        assert!(!fixture.args.exists());
        let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
        assert!(stderr.contains("trusted installed updater is unavailable"));
    }
}

#[test]
fn direct_update_uses_the_installed_signed_updater() {
    let fixture = Fixture::new("direct-update-installed");
    fixture.install_runtime(RECORDING_RUNTIME);
    let libexec = fixture.home.join("libexec");
    fs::create_dir_all(&libexec).expect("create updater directory");
    let installer = libexec.join("install.sh");
    fs::write(
        &installer,
        r#"#!/bin/sh
for arg in "$@"; do
    printf '<%s>\n' "$arg"
done > "$AOS_TEST_ARGS"
exit 23
"#,
    )
    .expect("write installed updater");

    let output = fixture
        .command()
        .env_remove("UNICITY_AOS_INSTALL_METHOD")
        .args(["update", "--channel", "dev"])
        .output()
        .expect("run installed updater");
    assert_eq!(output.status.code(), Some(23));
    assert_eq!(
        fs::read_to_string(&fixture.args).expect("read updater args"),
        "<--channel>\n<dev>\n<--yes>\n<--no-migrate-prompt>\n"
    );

    let output = fixture
        .command()
        .args(["update", "--version", "2026.13.0"])
        .output()
        .expect("run exact installed updater");
    assert_eq!(output.status.code(), Some(23));
    assert_eq!(
        fs::read_to_string(&fixture.args).expect("read updater args"),
        "<--version>\n<2026.13.0>\n<--yes>\n<--no-migrate-prompt>\n"
    );

    for args in [
        vec!["update", "--version", "2026.01.0"],
        vec!["update", "--version", "2025.9.0"],
        vec!["update", "--channel", "dev", "--version", "2026.1.3"],
    ] {
        assert!(
            !fixture
                .command()
                .args(args)
                .status()
                .expect("reject update selector")
                .success()
        );
    }
}

#[test]
fn homebrew_update_uses_the_formula_upgrade_path() {
    let fixture = Fixture::new("homebrew-update");
    fixture.install_runtime(RECORDING_RUNTIME);
    let bin = fixture.root.join("bin");
    fs::create_dir_all(&bin).expect("create fake bin");
    let brew = bin.join("brew");
    fs::write(
        &brew,
        r#"#!/bin/sh
for arg in "$@"; do
    printf '<%s>\n' "$arg"
done > "$AOS_TEST_ARGS"
exit 23
"#,
    )
    .expect("write fake brew");
    let mut permissions = fs::metadata(&brew).expect("brew metadata").permissions();
    permissions.set_mode(0o700);
    fs::set_permissions(&brew, permissions).expect("make fake brew executable");

    let output = fixture
        .command()
        .env("UNICITY_AOS_INSTALL_METHOD", "homebrew")
        .env("PATH", &bin)
        .arg("update")
        .output()
        .expect("run Homebrew product update");

    assert_eq!(output.status.code(), Some(23));
    assert_eq!(
        fs::read_to_string(&fixture.args).expect("read brew args"),
        "<upgrade>\n<unicity-aos/tap/aos>\n"
    );

    let output = fixture
        .command()
        .env("UNICITY_AOS_INSTALL_METHOD", "homebrew")
        .env("PATH", &bin)
        .args(["update", "--channel", "nightly"])
        .output()
        .expect("reject non-stable Homebrew update");
    assert_eq!(output.status.code(), Some(2));
}
