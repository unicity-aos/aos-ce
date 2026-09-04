//! Signed release inventory validation and authenticated archive checks.

use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::aos_home::AosHome;
use crate::fs_validation::{
    create_private_dir, validate_activation_file, validate_private_dir, validate_regular_file,
};
use crate::{
    PRODUCT_VERSION, RELEASE_ISSUER, RELEASE_MANIFEST_FILE, RELEASE_REPOSITORY,
    RELEASE_VERIFIER_NAME, RELEASE_VERIFIER_VERSION, RUNTIME_EXECUTABLE_NAMES,
};

impl AosHome {
    /// Validate the release-owned runtime and product assets before dispatch.
    ///
    /// The authority is the persisted release statement and bundle. After
    /// authenticating the persisted archive, selected executables are compared
    /// byte-for-byte against that archive before any runtime process starts.
    ///
    /// # Errors
    /// Returns an error when the signed release inventory, pinned verifier,
    /// authenticated archive, runtime executable, capsule asset, or product
    /// manifest is missing or invalid.
    pub fn ensure_runtime_available(&self) -> io::Result<()> {
        let binary = self.runtime_binary();
        self.ensure_runtime_executable(&binary, "runtime")?;
        self.validate_signed_release_inventory()?;
        self.ensure_selected_distribution()
    }

    fn validate_signed_release_inventory(&self) -> io::Result<()> {
        let release = self.release_dir();
        validate_private_dir(&release)?;

        let signed_dir = self.release_statement_dir();
        let verifier_dir = self.release_verifier_dir();
        validate_private_dir(&signed_dir)?;
        validate_private_dir(&verifier_dir)?;

        let statement_path = self.release_statement_path();
        let bundle_path = self.release_statement_bundle_path();
        let verifier_path = self.release_verifier_path();
        validate_regular_file(&statement_path, false)?;
        validate_regular_file(&bundle_path, false)?;
        validate_regular_file(&verifier_path, true)?;

        let expected_signed_entries = vec![
            bundle_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| invalid_release_inventory("signed bundle path is invalid"))?
                .to_owned(),
            statement_path
                .file_name()
                .and_then(|name| name.to_str())
                .ok_or_else(|| invalid_release_inventory("signed statement path is invalid"))?
                .to_owned(),
        ];
        let archive_name = {
            let entries = signed_dir_entries(&signed_dir)?;
            let mut archives: Vec<_> = entries
                .iter()
                .filter(|name| name.ends_with(".tar.gz"))
                .cloned()
                .collect();
            if archives.len() != 1 {
                return Err(invalid_release_inventory(
                    "signed release inventory must contain exactly one authenticated archive",
                ));
            }
            let archive_name = archives.remove(0);
            let mut expected = expected_signed_entries;
            expected.push(archive_name.clone());
            expected.sort();
            if entries != expected {
                return Err(invalid_release_inventory(
                    "signed release inventory contains unexpected entries",
                ));
            }
            archive_name
        };
        if signed_dir_entries(&verifier_dir)?.as_slice() != [RELEASE_VERIFIER_NAME] {
            return Err(invalid_release_inventory(
                "release verifier inventory contains unexpected entries",
            ));
        }

        let statement_bytes = fs::read(&statement_path)?;
        let statement: toml::Value = toml::from_str(&String::from_utf8_lossy(&statement_bytes))
            .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if statement
            .get("schema-version")
            .and_then(toml::Value::as_integer)
            != Some(1)
            || toml_string(&statement, "kind")? != "aos-release"
            || toml_string(&statement, "product")? != "unicity-aos-ce"
            || toml_string(&statement, "version")? != PRODUCT_VERSION
            || toml_string(&statement, "tag")? != PRODUCT_VERSION
        {
            return Err(invalid_release_inventory(
                "signed statement does not select this AOS release",
            ));
        }
        let expected_identity = format!(
            "https://github.com/{RELEASE_REPOSITORY}/.github/workflows/release.yml@refs/tags/{PRODUCT_VERSION}"
        );
        if toml_string(&statement, "release-workflow-identity")? != expected_identity {
            return Err(invalid_release_inventory(
                "signed statement release identity does not match this AOS release",
            ));
        }
        let runtime_value = statement
            .get("runtime")
            .ok_or_else(|| invalid_release_inventory("signed statement has no runtime record"))?;
        let runtime = runtime_value
            .as_table()
            .ok_or_else(|| invalid_release_inventory("signed statement has no runtime record"))?;
        if runtime
            .get("release-metadata-available")
            .and_then(toml::Value::as_bool)
            != Some(true)
            || !is_blake3(toml_string(runtime_value, "release-metadata-blake3")?)
        {
            return Err(invalid_release_inventory(
                "signed statement runtime metadata inventory is invalid",
            ));
        }

        let (_, target_record) = selected_target_and_record(&statement, &archive_name)?;
        let asset = toml_string(target_record, "asset")?;
        let target_bundle = toml_string(target_record, "sigstore-bundle")?;
        let expected_target_bundle = format!("{asset}.sigstore.json");
        if asset != archive_name || target_bundle != expected_target_bundle {
            return Err(invalid_release_inventory(
                "signed statement target bundle names do not match",
            ));
        }
        if !is_sha256(toml_string(target_record, "sha256")?)
            || !is_blake3(toml_string(target_record, "blake3")?)
        {
            return Err(invalid_release_inventory(
                "signed statement archive digest is invalid",
            ));
        }

        let archive_path = signed_dir.join(&archive_name);
        validate_regular_file(&archive_path, false)?;
        let metadata = fs::symlink_metadata(&archive_path)?;
        let expected_size = target_record
            .get("size")
            .and_then(toml::Value::as_integer)
            .filter(|size| *size > 0)
            .ok_or_else(|| invalid_release_inventory("signed statement archive size is invalid"))?;
        if metadata.len()
            != u64::try_from(expected_size).map_err(|_| {
                invalid_release_inventory("signed statement archive size is invalid")
            })?
        {
            return Err(invalid_release_inventory(
                "authenticated archive size does not match the signed statement",
            ));
        }
        if blake3_file(&archive_path)? != toml_string(target_record, "blake3")? {
            return Err(invalid_release_inventory(
                "authenticated archive digest does not match the signed statement",
            ));
        }

        let prefix = format!("unicity-aos-{PRODUCT_VERSION}-");
        let target = archive_name
            .strip_prefix(&prefix)
            .and_then(|name| name.strip_suffix(".tar.gz"))
            .filter(|target| !target.is_empty())
            .ok_or_else(|| invalid_release_inventory("authenticated archive name is invalid"))?;
        let expected_root = archive_name
            .strip_suffix(".tar.gz")
            .unwrap_or(&archive_name);
        list_archive(&archive_path, expected_root)?;

        let verification = release.join(format!(
            ".inventory-verify-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?
                .as_nanos()
        ));
        if fs::symlink_metadata(&verification).is_ok() {
            return Err(invalid_release_inventory(
                "stale release inventory verification state exists",
            ));
        }
        let result = self.verify_authenticated_archive(
            &verification,
            &archive_path,
            expected_root,
            target,
            &statement,
            &verifier_path,
            &statement_path,
            &bundle_path,
        );
        let _ = fs::remove_dir_all(&verification);
        result
    }

    #[allow(clippy::too_many_arguments)]
    fn verify_authenticated_archive(
        &self,
        verification: &Path,
        archive_path: &Path,
        expected_root: &str,
        target: &str,
        statement: &toml::Value,
        verifier_path: &Path,
        statement_path: &Path,
        bundle_path: &Path,
    ) -> io::Result<()> {
        create_private_dir(verification)?;
        let status = Command::new("/usr/bin/tar")
            .args(["-xzf"])
            .arg(archive_path)
            .arg("-C")
            .arg(verification)
            .status()?;
        if !status.success() {
            return Err(invalid_release_inventory(
                "authenticated archive cannot be extracted for inventory comparison",
            ));
        }
        let authenticated = validate_archive_tree(verification, expected_root)?;

        let manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(authenticated.join(RELEASE_MANIFEST_FILE))?)
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
        if manifest
            .get("schema_version")
            .and_then(serde_json::Value::as_i64)
            != Some(2)
            || manifest
                .pointer("/product/version")
                .and_then(serde_json::Value::as_str)
                != Some(PRODUCT_VERSION)
            || manifest.get("target").and_then(serde_json::Value::as_str) != Some(target)
        {
            return Err(invalid_release_inventory(
                "authenticated archive inventory does not describe this release",
            ));
        }
        let verifier_record = manifest
            .get("verifier")
            .and_then(serde_json::Value::as_object)
            .ok_or_else(|| {
                invalid_release_inventory("authenticated archive has no verifier inventory")
            })?;
        if verifier_record
            .get("version")
            .and_then(serde_json::Value::as_str)
            != Some(RELEASE_VERIFIER_VERSION)
            || verifier_record
                .get("asset")
                .and_then(serde_json::Value::as_str)
                .is_none()
            || !is_sha256(
                verifier_record
                    .get("sha256")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or_default(),
            )
        {
            return Err(invalid_release_inventory(
                "authenticated archive verifier inventory is invalid",
            ));
        }
        let expected_verifier_digest = verifier_record
            .get("sha256")
            .and_then(serde_json::Value::as_str)
            .expect("validated verifier digest");
        let verifier_digest = sha256_file(verifier_path).map_err(|error| {
            io::Error::new(error.kind(), "could not checksum persisted verifier")
        })?;
        if verifier_digest != expected_verifier_digest {
            return Err(invalid_release_inventory(
                "persisted Sigstore verifier does not match the authenticated release inventory",
            ));
        }

        let expected_identity = format!(
            "https://github.com/{RELEASE_REPOSITORY}/.github/workflows/release.yml@refs/tags/{PRODUCT_VERSION}"
        );
        let output = Command::new(verifier_path)
            .arg("verify-blob")
            .arg("--bundle")
            .arg(bundle_path)
            .arg("--certificate-oidc-issuer")
            .arg(RELEASE_ISSUER)
            .arg("--certificate-identity")
            .arg(&expected_identity)
            .arg("--use-signed-timestamps")
            .arg(statement_path)
            .output()
            .map_err(|error| {
                io::Error::new(error.kind(), "could not execute persisted verifier")
            })?;
        if !output.status.success() {
            return Err(io::Error::other(format!(
                "persisted release statement failed Sigstore verification: {}",
                String::from_utf8_lossy(&output.stderr).trim()
            )));
        }
        if toml_string(statement, "release-workflow-identity")? != expected_identity {
            return Err(invalid_release_inventory(
                "verified statement does not carry the exact release identity",
            ));
        }

        let activation = self.activation_binary();
        validate_activation_file(&activation)?;
        if fs::read(&activation)? != fs::read(authenticated.join("bin/aos"))? {
            return Err(invalid_release_inventory(
                "active launcher does not match the authenticated archive",
            ));
        }
        compare_inventory_file(
            &self.release_bin_dir().join("aos"),
            &authenticated.join("bin/aos"),
            true,
        )?;
        for name in RUNTIME_EXECUTABLE_NAMES {
            compare_inventory_file(
                &self.release_runtime_bin_dir().join(name),
                &authenticated.join("runtime/bin").join(name),
                true,
            )?;
        }
        Ok(())
    }

    pub(crate) fn ensure_runtime_executable(&self, binary: &Path, label: &str) -> io::Result<()> {
        let metadata = fs::symlink_metadata(binary).map_err(|error| {
            if error.kind() == io::ErrorKind::NotFound {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!(
                        "bundled {label} executable not found at {}",
                        binary.display()
                    ),
                )
            } else {
                error
            }
        })?;
        if !metadata.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "bundled {label} executable must be a regular file at {}",
                    binary.display()
                ),
            ));
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if metadata.permissions().mode() & 0o111 == 0 {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!(
                        "bundled {label} executable is not executable at {}",
                        binary.display()
                    ),
                ));
            }
        }
        Ok(())
    }
}

fn invalid_release_inventory(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

fn is_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn is_blake3(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
}

fn toml_string<'a>(value: &'a toml::Value, key: &str) -> io::Result<&'a str> {
    value.get(key).and_then(toml::Value::as_str).ok_or_else(|| {
        invalid_release_inventory(&format!("signed release statement field is invalid: {key}"))
    })
}

pub(crate) fn sha256_file(path: &Path) -> io::Result<String> {
    #[cfg(target_os = "macos")]
    let output = Command::new("/usr/bin/shasum")
        .args(["-a", "256"])
        .arg(path)
        .output()?;
    #[cfg(not(target_os = "macos"))]
    let output = Command::new("/usr/bin/sha256sum").arg(path).output()?;
    if !output.status.success() {
        return Err(io::Error::other("could not checksum the release verifier"));
    }
    let digest = String::from_utf8_lossy(&output.stdout)
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_owned();
    if !is_sha256(&digest) {
        return Err(io::Error::other("release verifier checksum is invalid"));
    }
    Ok(digest)
}

pub(crate) fn blake3_file(path: &Path) -> io::Result<String> {
    let mut file = fs::File::open(path)?;
    let mut hasher = blake3::Hasher::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let read = file.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        hasher.update(&buffer[..read]);
    }
    Ok(hasher.finalize().to_string())
}

fn signed_dir_entries(dir: &Path) -> io::Result<Vec<String>> {
    let mut entries = Vec::new();
    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| invalid_release_inventory("signed release entry is not valid UTF-8"))?;
        entries.push(name);
    }
    entries.sort();
    Ok(entries)
}

fn selected_target_and_record<'a>(
    statement: &'a toml::Value,
    archive_name: &str,
) -> io::Result<(&'a str, &'a toml::Value)> {
    let targets = statement
        .get("targets")
        .and_then(toml::Value::as_table)
        .ok_or_else(|| invalid_release_inventory("signed release has no target inventory"))?;
    let mut selected = None;
    for (target, record) in targets {
        if toml_string(record, "asset")? == archive_name {
            if selected.is_some() {
                return Err(invalid_release_inventory(
                    "signed release selects multiple target archives",
                ));
            }
            selected = Some((target.as_str(), record));
        }
    }
    selected.ok_or_else(|| {
        invalid_release_inventory("persisted archive is absent from the signed statement")
    })
}

fn validate_archive_tree(extracted: &Path, expected_root: &str) -> io::Result<PathBuf> {
    let mut roots = Vec::new();
    for entry in fs::read_dir(extracted)? {
        let entry = entry?;
        roots.push(entry.path());
    }
    if roots.len() != 1
        || roots[0].file_name().and_then(|name| name.to_str()) != Some(expected_root)
    {
        return Err(invalid_release_inventory(
            "authenticated archive root is invalid",
        ));
    }
    let root = roots.remove(0);
    fn walk(directory: &Path) -> io::Result<()> {
        for entry in fs::read_dir(directory)? {
            let entry = entry?;
            let metadata = fs::symlink_metadata(entry.path())?;
            if metadata.is_dir() {
                walk(&entry.path())?;
            } else if !metadata.is_file() {
                return Err(invalid_release_inventory(
                    "authenticated archive contains a non-regular file",
                ));
            }
        }
        Ok(())
    }
    walk(&root)?;
    Ok(root)
}

fn list_archive(archive: &Path, expected_root: &str) -> io::Result<()> {
    let output = Command::new("/usr/bin/tar")
        .arg("-tzf")
        .arg(archive)
        .output()?;
    if !output.status.success() {
        return Err(invalid_release_inventory(
            "authenticated archive cannot be inspected",
        ));
    }
    let listing = String::from_utf8_lossy(&output.stdout);
    for name in listing.lines() {
        let name = name.strip_suffix('/').unwrap_or(name);
        if name.is_empty() || name.starts_with('/') {
            return Err(invalid_release_inventory(
                "authenticated archive contains an unsafe path",
            ));
        }
        if name == expected_root {
            continue;
        }
        let Some(relative) = name.strip_prefix(&format!("{expected_root}/")) else {
            return Err(invalid_release_inventory(
                "authenticated archive contains an unexpected root",
            ));
        };
        if relative
            .split('/')
            .any(|component| component.is_empty() || component == "." || component == "..")
        {
            return Err(invalid_release_inventory(
                "authenticated archive contains an unsafe path",
            ));
        }
    }
    Ok(())
}

fn compare_inventory_file(
    release_file: &Path,
    authenticated_file: &Path,
    executable: bool,
) -> io::Result<()> {
    validate_regular_file(release_file, executable)?;
    if fs::read(release_file)? != fs::read(authenticated_file)? {
        return Err(invalid_release_inventory(&format!(
            "release executable does not match the authenticated archive: {}",
            release_file.display()
        )));
    }
    Ok(())
}

pub(crate) fn validate_path_entry(path: &Path, variable: &str) -> io::Result<()> {
    std::env::join_paths(std::iter::once(path))
        .map(drop)
        .map_err(|_| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{variable} cannot contain a platform PATH separator"),
            )
        })
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::aos_home::{runtime_binary_name, runtime_daemon_binary_name};
    use crate::test_fixtures::fixtures::*;
    use std::io::ErrorKind;
    use std::path::PathBuf;
    #[test]
    fn runtime_executes_from_the_exact_product_release() {
        let home = AosHome::from_root("/tmp/unicity-aos-test");
        assert_eq!(home.root(), PathBuf::from("/tmp/unicity-aos-test"));
        assert_eq!(
            home.runtime_home(),
            PathBuf::from("/tmp/unicity-aos-test/runtime")
        );
        assert_eq!(home.run_root(), PathBuf::from("/tmp/unicity-aos-test/run"));
        assert_eq!(
            home.release_dir(),
            PathBuf::from(format!(
                "/tmp/unicity-aos-test/releases/{}",
                env!("CARGO_PKG_VERSION")
            ))
        );
        assert_eq!(
            home.runtime_binary(),
            home.release_runtime_bin_dir().join(runtime_binary_name())
        );
        assert_eq!(
            home.runtime_daemon_binary(),
            home.release_runtime_bin_dir()
                .join(runtime_daemon_binary_name())
        );
    }

    #[test]
    fn mutable_runtime_copy_cannot_substitute_for_a_missing_release_binary() {
        let fixture = temporary_home();
        let home = AosHome::from_root(&fixture);
        let mutable_binary = home.runtime_home().join("bin").join(runtime_binary_name());
        fs::create_dir_all(mutable_binary.parent().expect("mutable binary parent"))
            .expect("create mutable runtime bin");
        fs::write(&mutable_binary, b"mutable compatibility copy")
            .expect("write mutable compatibility copy");
        install_capsule_fixtures(home.root());
        fs::remove_file(home.runtime_binary()).expect("remove release runtime");

        let error = home
            .ensure_runtime_available()
            .expect_err("mutable compatibility copy must not satisfy release lookup");
        assert_eq!(error.kind(), ErrorKind::NotFound);
        assert!(
            error.to_string().contains(
                home.release_runtime_bin_dir()
                    .join(runtime_binary_name())
                    .to_string_lossy()
                    .as_ref()
            )
        );
        fs::remove_dir_all(fixture).expect("remove mutable runtime fixture");
    }

    #[test]
    fn tampered_release_runtime_fails_inventory_validation() {
        let fixture = temporary_home();
        let home = AosHome::from_root(&fixture);
        install_capsule_fixtures(home.root());
        fs::write(home.runtime_binary(), b"tampered runtime bytes")
            .expect("tamper release runtime");

        let error = home
            .ensure_runtime_available()
            .expect_err("tampered release runtime must fail before launch");
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(
            error
                .to_string()
                .contains("does not match the authenticated archive")
        );
        fs::remove_dir_all(fixture).expect("remove tamper fixture");
    }

    #[test]
    fn replaced_signed_inventory_state_fails_before_launch() {
        let fixture = temporary_home();
        let home = AosHome::from_root(&fixture);
        install_capsule_fixtures(home.root());
        let archive = fs::read(
            home.release_statement_dir()
                .join("unicity-aos-2026.1.3-x86_64-unknown-linux-gnu.tar.gz"),
        )
        .expect("read archive fixture");
        let statement_path = home.release_statement_path();
        let statement = fs::read(&statement_path).expect("read statement fixture");
        let bundle_path = home.release_statement_bundle_path();
        let bundle = fs::read(&bundle_path).expect("read bundle fixture");
        let verifier_path = home.release_verifier_path();
        let verifier = fs::read(&verifier_path).expect("read verifier fixture");
        let activation = home.activation_binary();
        let active_launcher = fs::read(&activation).expect("read active launcher");

        let mismatched_version = String::from_utf8_lossy(&statement).replace(
            &format!("version = \"{PRODUCT_VERSION}\""),
            "version = \"9.9.9\"",
        );
        fs::write(&statement_path, mismatched_version).expect("replace version");
        let error = home
            .ensure_runtime_available()
            .expect_err("statement version mismatch must fail before launch");
        assert!(
            error
                .to_string()
                .contains("does not select this AOS release")
        );
        fs::write(&statement_path, &statement).expect("restore statement");

        let replaced_archive = vec![b'x'; archive.len()];
        fs::write(
            home.release_statement_dir()
                .join("unicity-aos-2026.1.3-x86_64-unknown-linux-gnu.tar.gz"),
            replaced_archive,
        )
        .expect("replace archive");
        let error = home
            .ensure_runtime_available()
            .expect_err("replaced archive must fail before launch");
        assert!(error.to_string().contains("archive digest"));
        fs::write(
            home.release_statement_dir()
                .join("unicity-aos-2026.1.3-x86_64-unknown-linux-gnu.tar.gz"),
            archive,
        )
        .expect("restore archive");

        let replaced_statement = String::from_utf8_lossy(&statement)
            .replace("a".repeat(40).as_str(), "b".repeat(40).as_str());
        fs::write(&statement_path, replaced_statement).expect("replace statement");
        let error = home
            .ensure_runtime_available()
            .expect_err("replaced statement must fail Sigstore verification");
        assert!(error.to_string().contains("failed Sigstore verification"));
        fs::write(&statement_path, statement).expect("restore statement");

        fs::write(&bundle_path, b"replaced bundle").expect("replace bundle");
        let error = home
            .ensure_runtime_available()
            .expect_err("replaced bundle must fail Sigstore verification");
        assert!(error.to_string().contains("failed Sigstore verification"));
        fs::write(&bundle_path, bundle).expect("restore bundle");

        fs::write(&verifier_path, b"replaced verifier").expect("replace verifier");
        let error = home
            .ensure_runtime_available()
            .expect_err("replaced verifier must fail its authenticated checksum");
        assert!(error.to_string().contains("persisted Sigstore verifier"));
        fs::write(&verifier_path, verifier).expect("restore verifier");

        fs::write(&activation, b"replaced active launcher").expect("replace active launcher");
        let error = home
            .ensure_runtime_available()
            .expect_err("replaced active launcher must fail before launch");
        assert!(
            error
                .to_string()
                .contains("active launcher does not match the authenticated archive")
        );
        fs::write(&activation, active_launcher).expect("restore active launcher");

        fs::write(home.release_manifest_path(), b"{}\n").expect("replace local cache");
        home.ensure_runtime_available()
            .expect("local cache must not authorize or veto authenticated execution");

        fs::remove_dir_all(fixture).expect("remove replacement fixture");
    }

    #[test]
    fn missing_signed_inventory_fails_before_launch() {
        let fixture = temporary_home();
        let home = AosHome::from_root(&fixture);
        install_capsule_fixtures(home.root());
        fs::remove_file(home.release_statement_path()).expect("remove statement");

        let error = home
            .ensure_runtime_available()
            .expect_err("missing signed statement must fail before launch");
        assert_eq!(error.kind(), ErrorKind::NotFound);
        fs::remove_dir_all(fixture).expect("remove missing inventory fixture");
    }

    #[cfg(unix)]
    #[test]
    fn symlinked_release_runtime_fails_before_inventory_hashing() {
        use std::os::unix::fs::symlink;

        let fixture = temporary_home();
        let home = AosHome::from_root(&fixture);
        install_capsule_fixtures(home.root());
        let binary = home.runtime_binary();
        fs::remove_file(&binary).expect("remove release runtime");
        symlink(fixture.join("outside"), &binary).expect("symlink release runtime");

        let error = home
            .ensure_runtime_available()
            .expect_err("symlinked release runtime must fail before launch");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
        fs::remove_dir_all(fixture).expect("remove symlink fixture");
    }

    #[test]
    fn foreground_daemon_rejects_a_nonexecutable_inventory_record() {
        let fixture = temporary_home();
        let home = AosHome::from_root(&fixture);
        install_capsule_fixtures(home.root());
        let runtime_bin = home.release_runtime_bin_dir();
        fs::create_dir_all(&runtime_bin).expect("create runtime bin");
        fs::write(home.runtime_daemon_binary(), b"daemon").expect("write daemon fixture");
        refresh_release_record(home.root(), "runtime/bin/astrid-daemon");
        let daemon = home.runtime_daemon_binary();
        use std::os::unix::fs::PermissionsExt;
        let mut permissions = fs::metadata(&daemon)
            .expect("read non-executable daemon fixture")
            .permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(&daemon, permissions).expect("clear daemon executable bits");
        refresh_release_record(home.root(), "runtime/bin/astrid-daemon");

        let error = home
            .foreground_daemon_command(None, false)
            .expect_err("non-executable daemon must fail before spawn");

        assert_eq!(error.kind(), ErrorKind::PermissionDenied);
        assert!(
            error
                .to_string()
                .contains("release inventory entry has unexpected permissions")
        );
        fs::remove_dir_all(fixture).expect("remove foreground daemon fixture");
    }

    #[cfg(unix)]
    #[test]
    fn foreground_daemon_rejects_a_rewritten_binary_without_an_inventory_update() {
        use std::os::unix::fs::PermissionsExt;

        let fixture = temporary_home();
        let home = AosHome::from_root(&fixture);
        install_capsule_fixtures(home.root());
        fs::write(home.runtime_daemon_binary(), b"tampered daemon").expect("tamper daemon fixture");
        {
            let daemon = home.runtime_daemon_binary();
            let mut permissions = fs::metadata(&daemon)
                .expect("read daemon fixture metadata")
                .permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(&daemon, permissions).expect("make daemon fixture executable");
        }

        let error = home
            .foreground_daemon_command(None, false)
            .expect_err("rewritten daemon bytes must fail before spawn");

        assert_eq!(error.kind(), ErrorKind::InvalidData);
        assert!(
            error
                .to_string()
                .contains("does not match the authenticated archive")
        );
        fs::remove_dir_all(fixture).expect("remove foreground daemon fixture");
    }
}
