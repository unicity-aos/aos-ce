use super::{Fixture, RECORDING_RUNTIME};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::PathBuf;

const PRIOR_PRODUCT: &str = "2026.10.0-rc.6";
const PRIOR_RUNTIME: &str = "2026.10.0-rc.4";

fn prior_receipt(fixture: &Fixture) -> PathBuf {
    let release = fixture.home.join("releases").join(PRIOR_PRODUCT);
    fixture.install_signed_distro_at(&release, PRIOR_PRODUCT, Some(PRIOR_RUNTIME));
    let manifest = fs::read(release.join("Distro.toml")).unwrap();
    let parsed: toml::Value = std::str::from_utf8(&manifest).unwrap().parse().unwrap();
    let key = parsed["distro"]["signing"]["pubkey"].as_str().unwrap();
    let receipt = serde_json::json!({
        "schema": 1, "kind": "aos-distro-apply-active-v1", "active": true,
        "cutover_complete": true, "distro_id": "unicity-ce", "distro_version": PRIOR_PRODUCT,
        "manifest_blake3": format!("blake3:{}", blake3::hash(&manifest)),
        "signing_pubkey": key, "pin_blake3": blake3::hash(key.as_bytes()).to_string(),
        "lock_blake3": format!("blake3:{}", blake3::hash(&fs::read(release.join("Distro.lock")).unwrap())),
        "signature_blake3": format!("blake3:{}", blake3::hash(&fs::read(release.join("Distro.sig")).unwrap())),
        "principal": "operator", "astrid_runtime_version": PRIOR_RUNTIME,
    });
    let path = fixture.home.join("receipts/unicity-ce.active.json");
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(&path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
    path
}

#[test]
fn authenticated_rc4_runtime_receipt_rolls_forward_after_successful_apply() {
    let fixture = Fixture::new("receipt-upgrade-success");
    fixture.install_runtime(RECORDING_RUNTIME);
    fixture.install_signed_distro();
    let path = prior_receipt(&fixture);
    let before = fs::read(&path).unwrap();
    let output = fixture
        .command()
        .args([
            "distro",
            "apply",
            "--principal",
            "operator",
            "--yes",
            "--offline",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let after = fs::read(&path).unwrap();
    assert_ne!(before, after);
    let receipt: serde_json::Value = serde_json::from_slice(&after).unwrap();
    assert_eq!(receipt["distro_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(receipt["principal"], "operator");
    let compatibility: toml::Value = include_str!("../../../../release/runtime-compatibility.toml")
        .parse()
        .unwrap();
    assert_eq!(
        receipt["astrid_runtime_version"],
        compatibility["runtime"]["version"].as_str().unwrap()
    );
    assert_eq!(
        fs::metadata(path).unwrap().permissions().mode() & 0o7777,
        0o600
    );
    assert!(fixture.root.join("stop-args").exists());
}

#[test]
fn failed_apply_preserves_authenticated_prior_receipt_bytes() {
    let fixture = Fixture::new("receipt-upgrade-failure");
    fixture.install_runtime(RECORDING_RUNTIME);
    fixture.install_signed_distro();
    let path = prior_receipt(&fixture);
    let before = fs::read(&path).unwrap();
    let output = fixture
        .command()
        .env("AOS_TEST_APPLY_EXIT", "41")
        .args([
            "distro",
            "apply",
            "--principal",
            "operator",
            "--yes",
            "--offline",
        ])
        .output()
        .unwrap();
    assert!(!output.status.success());
    assert!(fixture.root.join("apply-args").exists());
    assert!(fixture.root.join("stop-args").exists());
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn prior_receipt_upgrade_refuses_wrong_owner_key_and_unverified_release() {
    for damage in [
        "principal",
        "key",
        "lock",
        "missing",
        "redirected",
        "future",
        "same_version",
        "future_runtime",
        "substituted_runtime",
        "runtime_build_metadata",
    ] {
        let fixture = Fixture::new(&format!("receipt-upgrade-{damage}"));
        fixture.install_runtime(RECORDING_RUNTIME);
        fixture.install_signed_distro();
        let path = prior_receipt(&fixture);
        let prior = fixture.home.join("releases").join(PRIOR_PRODUCT);
        let mut receipt: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        match damage {
            "principal" => receipt["principal"] = "foreign".into(),
            "key" => receipt["signing_pubkey"] = "ed25519:foreign".into(),
            "lock" => fs::write(prior.join("Distro.lock"), b"tampered").unwrap(),
            "missing" => fs::remove_dir_all(&prior).unwrap(),
            "redirected" => {
                let outside = fixture.root.join("redirected-release");
                fs::rename(&prior, &outside).unwrap();
                std::os::unix::fs::symlink(outside, &prior).unwrap();
            }
            "future" => receipt["distro_version"] = "2027.1.0".into(),
            "same_version" => receipt["distro_version"] = env!("CARGO_PKG_VERSION").into(),
            "future_runtime" => receipt["astrid_runtime_version"] = "2027.1.0".into(),
            "substituted_runtime" => receipt["astrid_runtime_version"] = "2026.10.0-rc.5".into(),
            "runtime_build_metadata" => {
                receipt["astrid_runtime_version"] = "2026.10.0-rc.4+substituted".into()
            }
            _ => unreachable!(),
        }
        fs::write(&path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
        let before = fs::read(&path).unwrap();
        let output = fixture
            .command()
            .args([
                "distro",
                "apply",
                "--principal",
                "operator",
                "--yes",
                "--offline",
            ])
            .output()
            .unwrap();
        assert!(!output.status.success(), "{damage}");
        assert!(!fixture.root.join("start-args").exists(), "{damage}");
        assert_eq!(fs::read(path).unwrap(), before, "{damage}");
    }
}

#[test]
fn historical_runtime_labels_cannot_select_the_runtime_of_the_new_apply() {
    let fixture = Fixture::new("receipt-upgrade-runtime-authority");
    fixture.install_runtime(RECORDING_RUNTIME);
    fixture.install_signed_distro();
    let path = prior_receipt(&fixture);
    let mut receipt: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    receipt["astrid_runtime_version"] = "2026.10.0-rc.5".into();
    fs::write(&path, serde_json::to_vec_pretty(&receipt).unwrap()).unwrap();
    let inventory_path = fixture
        .home
        .join("releases")
        .join(PRIOR_PRODUCT)
        .join("release-manifest.json");
    let mut inventory: serde_json::Value =
        serde_json::from_slice(&fs::read(&inventory_path).unwrap()).unwrap();
    inventory["runtime"]["version"] = "2026.10.0-rc.5".into();
    fs::write(
        inventory_path,
        serde_json::to_vec_pretty(&inventory).unwrap(),
    )
    .unwrap();
    let output = fixture
        .command()
        .args([
            "distro",
            "apply",
            "--principal",
            "operator",
            "--yes",
            "--offline",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let current: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    let compatibility: toml::Value = include_str!("../../../../release/runtime-compatibility.toml")
        .parse()
        .unwrap();
    assert_eq!(
        current["astrid_runtime_version"],
        compatibility["runtime"]["version"].as_str().unwrap()
    );
    assert_eq!(current["distro_version"], env!("CARGO_PKG_VERSION"));
    assert!(fixture.root.join("apply-args").exists());
}

#[test]
fn distro_apply_refuses_a_foreign_runtime_override_before_any_dispatch() {
    let fixture = Fixture::new("receipt-upgrade-foreign-runtime");
    fixture.install_runtime(RECORDING_RUNTIME);
    fixture.install_signed_distro();
    let receipt = prior_receipt(&fixture);
    let before = fs::read(&receipt).unwrap();
    let foreign = fixture.root.join("foreign-runtime");
    fs::write(
        &foreign,
        "#!/bin/sh\nprintf foreign > \"$AOS_TEST_FOREIGN_MARKER\"\nexit 0\n",
    )
    .unwrap();
    Fixture::make_executable(&foreign);
    let marker = fixture.root.join("foreign-dispatched");
    let output = fixture
        .command()
        .env("UNICITY_AOS_RUNTIME_BIN", foreign)
        .env("AOS_TEST_FOREIGN_MARKER", &marker)
        .args([
            "distro",
            "apply",
            "--principal",
            "operator",
            "--yes",
            "--offline",
        ])
        .output()
        .unwrap();
    assert!(!marker.exists(), "foreign runtime was dispatched");
    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("requires the selected bundled runtime")
    );
    assert_eq!(fs::read(receipt).unwrap(), before);
}

#[test]
fn capsule_update_discovery_keeps_verified_ownership_with_a_runtime_override() {
    let fixture = Fixture::new("capsule-update-runtime-override");
    fixture.install_signed_distro();
    fixture.install_runtime(r#"#!/bin/sh
case "$3" in
  agent) printf '%s' '[{"principal":"operator","enabled":true}]' ;;
  capsule) printf '%s' '{"schema_version":1,"principal":"operator","items":[{"name":"user-probe","installed_version":"1.0.0","wasm_hash":null,"candidate_version":"1.1.0","availability":"available","message":"New version"}]}' ;;
  *) exit 93 ;;
esac
"#);
    let foreign = fixture.root.join("foreign-discovery-runtime");
    fs::copy(&fixture.runtime, &foreign).unwrap();
    Fixture::make_executable(&foreign);
    let output = fixture
        .command()
        .env("UNICITY_AOS_RUNTIME_BIN", foreign)
        .args(["updates", "capsules", "--principal", "operator", "--json"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let inventory: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
    assert_eq!(inventory["items"][0]["candidate_version"], "1.1.0");
    let message = inventory["items"][0]["message"].as_str().unwrap();
    assert!(message.contains("Review with: aos"), "{message}");
    assert!(
        !message.contains("ownership could not be verified"),
        "{message}"
    );
}
