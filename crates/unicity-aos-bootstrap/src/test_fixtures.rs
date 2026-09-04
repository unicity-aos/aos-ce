#[cfg(test)]
pub(crate) mod fixtures {
    use crate::aos_home::AosHome;
    use crate::capsules::capsule_assets_from_manifest;
    use crate::fs_validation::validate_regular_file;
    use crate::release_inventory::{blake3_file, sha256_file};
    use crate::{
        PRODUCT_VERSION, RELEASE_MANIFEST_FILE, RELEASE_VERIFIER_VERSION, RUNTIME_EXECUTABLE_NAMES,
        UNICITY_CE_MANIFEST,
    };
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::process::Command;
    use std::time::{SystemTime, UNIX_EPOCH};

    pub(crate) fn set_private_file_permissions(path: &Path) -> io::Result<()> {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        }
        #[cfg(not(unix))]
        let _ = path;
        Ok(())
    }

    static FIXTURE_SEQUENCE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    pub(crate) fn temporary_home() -> PathBuf {
        let root = std::env::temp_dir().join(format!(
            "unicity-aos-bundled-manifest-{}-{}-{}",
            std::process::id(),
            FIXTURE_SEQUENCE.fetch_add(1, std::sync::atomic::Ordering::Relaxed),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create fixture home");
        root
    }

    pub(crate) fn install_capsule_fixtures(root: &std::path::Path) -> PathBuf {
        let release = root.join("releases").join(env!("CARGO_PKG_VERSION"));
        for directory in [
            root.to_path_buf(),
            root.join("bin"),
            release.clone(),
            release.join("bin"),
            release.join("libexec"),
            release.join("runtime/bin"),
            release.join("signed"),
            release.join("verifier"),
            release.join("capsules"),
        ] {
            fs::create_dir_all(&directory).expect("create release fixture directory");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                fs::set_permissions(&directory, fs::Permissions::from_mode(0o700))
                    .expect("make release fixture directory private");
            }
        }

        let mut records = serde_json::Map::new();
        let mut write_fixture = |relative: &str, bytes: &[u8]| {
            let executable = relative == "bin/aos" || relative.starts_with("runtime/bin/");
            let path = release.join(relative);
            let bytes = if relative == "Distro.toml" {
                UNICITY_CE_MANIFEST.as_bytes().to_vec()
            } else {
                bytes.to_vec()
            };
            fs::write(&path, &bytes).expect("write release fixture");
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let mode = if executable { 0o700 } else { 0o600 };
                fs::set_permissions(&path, fs::Permissions::from_mode(mode))
                    .expect("set release fixture mode");
                records.insert(
                    relative.to_owned(),
                    serde_json::json!({
                        "blake3": blake3::Hasher::new()
                            .update(&bytes)
                            .finalize()
                            .to_string(),
                        "mode": mode,
                    }),
                );
            }
            #[cfg(not(unix))]
            records.insert(
                relative.to_owned(),
                serde_json::json!({
                    "blake3": blake3::Hasher::new().update(&bytes).finalize().to_string(),
                    "mode": if executable { 448 } else { 384 },
                }),
            );
        };

        for relative in [
            "Distro.toml",
            "Distro.lock",
            "Distro.sig",
            "README.md",
            "bin/aos",
            "capsule-assets.txt",
            "libexec/install.sh",
            "runtime-compatibility.toml",
            "runtime/bin/astrid",
            "runtime/bin/astrid-build",
            "runtime/bin/astrid-daemon",
            "runtime/bin/astrid-emit",
        ] {
            write_fixture(relative, format!("release-fixture-{relative}").as_bytes());
        }
        let capsule_bytes = b"capsule fixture".as_slice();
        for asset in capsule_assets_from_manifest().expect("read embedded capsule set") {
            write_fixture(&format!("capsules/{asset}"), capsule_bytes);
        }
        fs::copy(release.join("bin/aos"), root.join("bin/aos")).expect("copy activation fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let activation = root.join("bin/aos");
            let mut permissions = fs::symlink_metadata(&activation)
                .expect("read activation fixture metadata")
                .permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(&activation, permissions).expect("make activation executable");
        }

        let manifest = serde_json::json!({
            "schema_version": 2,
            "release_files": records,
        });
        fs::write(
            release.join("release-manifest.json"),
            serde_json::to_vec(&manifest).expect("serialize release manifest"),
        )
        .expect("write release manifest");

        let fixture_target = "x86_64-unknown-linux-gnu";
        let archive_name = format!("unicity-aos-{PRODUCT_VERSION}-{fixture_target}.tar.gz");
        let statement_name = format!("unicity-aos-{PRODUCT_VERSION}-release.toml");
        let bundle_name = format!("{statement_name}.sigstore.json");
        let statement_path = release.join("signed").join(&statement_name);
        let bundle_path = release.join("signed").join(&bundle_name);
        let archive_path = release.join("signed").join(&archive_name);
        let verifier_path = release.join("verifier").join("cosign");
        let reference = root.join("signed-fixture-reference");
        fs::create_dir_all(&reference).expect("create signed fixture reference");

        let cosign = r#"#!/bin/sh
set -eu
bundle=
artifact=
identity=
while [ "$#" -gt 0 ]; do
  case "$1" in
    --bundle) bundle=$2; shift 2 ;;
    --certificate-identity) identity=$2; shift 2 ;;
    -*) shift ;;
    *) artifact=$1; shift ;;
  esac
done
[ -n "$bundle" ] && [ -n "$artifact" ] && [ -n "$identity" ]
cmp "$bundle" "REFERENCE_BUNDLE"
cmp "$artifact" "REFERENCE_STATEMENT"
exit 0
"#
        .replace(
            "REFERENCE_BUNDLE",
            &root.join("reference-bundle").display().to_string(),
        )
        .replace(
            "REFERENCE_STATEMENT",
            &root.join("reference-statement").display().to_string(),
        );
        fs::write(&verifier_path, cosign).expect("write fixture verifier");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut permissions = fs::symlink_metadata(&verifier_path)
                .expect("read verifier fixture metadata")
                .permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(&verifier_path, permissions)
                .expect("make verifier fixture executable");
        }

        let archive_work = root.join(".archive-fixture");
        let archive_root_name = archive_name
            .strip_suffix(".tar.gz")
            .expect("fixture archive suffix");
        let archive_root = archive_work.join(archive_root_name);
        fs::create_dir_all(archive_root.join("bin")).expect("create archive fixture bin");
        fs::create_dir_all(archive_root.join("runtime/bin"))
            .expect("create archive fixture runtime");
        let fixture_home = AosHome::from_root(root);
        fs::write(fixture_home.runtime_daemon_binary(), b"daemon")
            .expect("write baseline daemon fixture");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let daemon = fixture_home.runtime_daemon_binary();
            let mut permissions = fs::symlink_metadata(&daemon)
                .expect("read baseline daemon metadata")
                .permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(&daemon, permissions).expect("make baseline daemon executable");
        }
        fs::copy(release.join("bin/aos"), archive_root.join("bin/aos"))
            .expect("copy release fixture binary");
        for name in RUNTIME_EXECUTABLE_NAMES {
            fs::copy(
                release.join("runtime/bin").join(name),
                archive_root.join("runtime/bin").join(name),
            )
            .expect("copy release fixture runtime");
        }
        let archive_manifest = serde_json::json!({
            "schema_version": 2,
            "product": {"version": PRODUCT_VERSION},
            "target": fixture_target,
            "verifier": {
                "version": RELEASE_VERIFIER_VERSION,
                "asset": "fixture-cosign",
                "sha256": sha256_file(&verifier_path).expect("fixture verifier checksum"),
            },
        });
        fs::write(
            archive_root.join(RELEASE_MANIFEST_FILE),
            serde_json::to_vec(&archive_manifest).expect("serialize archive manifest"),
        )
        .expect("write archive manifest");
        let status = Command::new("/usr/bin/tar")
            .env("COPYFILE_DISABLE", "1")
            .args(["-czf"])
            .arg(&archive_path)
            .arg("-C")
            .arg(&archive_work)
            .arg(archive_root_name)
            .status()
            .expect("create archive fixture");
        assert!(status.success());

        let archive_digest = blake3_file(&archive_path).expect("archive fixture digest");
        let archive_size = fs::symlink_metadata(&archive_path)
            .expect("archive fixture metadata")
            .len();
        let statement = format!(
            r#"schema-version = 1
kind = "aos-release"
product = "unicity-aos-ce"
version = "{PRODUCT_VERSION}"
tag = "{PRODUCT_VERSION}"
source-commit = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
published-at = "2026-07-16T10:00:00Z"
release-workflow-identity = "https://github.com/unicity-aos/aos-ce/.github/workflows/release.yml@refs/tags/{PRODUCT_VERSION}"

[runtime]
release-metadata-available = true
release-metadata-blake3 = "0000000000000000000000000000000000000000000000000000000000000000"

[targets.{fixture_target}]
asset = "{archive_name}"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
blake3 = "{archive_digest}"
sigstore-bundle = "{archive_name}.sigstore.json"
size = {archive_size}
"#
        );
        fs::write(&statement_path, &statement).expect("write statement fixture");
        set_private_file_permissions(&statement_path).expect("set statement fixture mode");
        fs::write(root.join("reference-statement"), statement).expect("write statement reference");
        fs::write(&bundle_path, b"fixture bundle").expect("write bundle fixture");
        set_private_file_permissions(&bundle_path).expect("set bundle fixture mode");
        fs::write(root.join("reference-bundle"), b"fixture bundle")
            .expect("write bundle reference");
        set_private_file_permissions(&archive_path).expect("set archive fixture mode");
        validate_regular_file(&statement_path, false).expect("statement fixture mode");
        validate_regular_file(&bundle_path, false).expect("bundle fixture mode");
        validate_regular_file(&archive_path, false).expect("archive fixture mode");
        release.join("capsules")
    }

    #[cfg(unix)]
    pub(crate) fn refresh_release_record(root: &std::path::Path, relative: &str) {
        use std::os::unix::fs::PermissionsExt;

        let release = root.join("releases").join(env!("CARGO_PKG_VERSION"));
        let path = release.join(relative);
        let bytes = fs::read(&path).expect("read refreshed release fixture");
        let mode = fs::symlink_metadata(&path)
            .expect("read refreshed release metadata")
            .permissions()
            .mode()
            & 0o777;
        let manifest_path = release.join("release-manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).expect("read release manifest"))
                .expect("decode release manifest");
        manifest["release_files"][relative] = serde_json::json!({
            "blake3": blake3::Hasher::new().update(&bytes).finalize().to_string(),
            "mode": mode,
        });
        fs::write(
            &manifest_path,
            serde_json::to_vec(&manifest).expect("serialize release manifest"),
        )
        .expect("write refreshed release manifest");
    }
}
