//! Shared end-to-end fixture for the product CLI boundary tests.

use std::fs;
use std::io::Read as _;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

pub(crate) struct Fixture {
    pub(crate) root: PathBuf,
    pub(crate) runtime: PathBuf,
    pub(crate) args: PathBuf,
    pub(crate) bootstrap_args: PathBuf,
    pub(crate) home: PathBuf,
    pub(crate) child_path: PathBuf,
}

pub(crate) const RELEASE_FILES: &[&str] = &[
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
];

pub(crate) const FIXTURE_TARGET: &str = "x86_64-unknown-linux-gnu";

pub(crate) const PRODUCT_DISTRO: &str =
    include_str!("../../../../distros/community/unicity-ce/Distro.toml");

impl Fixture {
    pub(crate) fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "aos-cli-boundary-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create fixture");
        let home = root.join("runtime-home");
        let runtime = home
            .join("releases")
            .join(env!("CARGO_PKG_VERSION"))
            .join("runtime/bin/astrid");
        let args = root.join("args");
        let bootstrap_args = root.join("bootstrap-args");
        let child_path = root.join("child-path");
        let fixture = Self {
            root,
            runtime,
            args,
            bootstrap_args,
            home,
            child_path,
        };
        fixture.install_capsules();
        fixture
    }

    pub(crate) fn install_capsules(&self) {
        let distro: toml::Value =
            include_str!("../../../../distros/community/unicity-ce/Distro.toml")
                .parse()
                .expect("parse embedded distro fixture");
        let release = self.home.join("releases").join(env!("CARGO_PKG_VERSION"));
        for directory in [
            self.root.clone(),
            self.home.clone(),
            self.home.join("bin"),
            self.home.join("releases"),
            release.clone(),
            release.join("bin"),
            release.join("libexec"),
            release.join("runtime/bin"),
            release.join("signed"),
            release.join("verifier"),
            release.join("capsules"),
        ] {
            fs::create_dir_all(&directory).expect("create release fixture");
            let mut permissions = fs::metadata(&directory)
                .expect("fixture directory")
                .permissions();
            permissions.set_mode(0o700);
            fs::set_permissions(&directory, permissions).expect("make fixture directory private");
        }
        let mut records = serde_json::Map::new();
        let mut write_release_file = |relative: &str, bytes: &[u8]| {
            let executable = relative == "bin/aos" || relative.starts_with("runtime/bin/");
            let path = release.join(relative);
            let bytes = if relative == "Distro.toml" {
                PRODUCT_DISTRO.as_bytes().to_vec()
            } else {
                bytes.to_vec()
            };
            fs::write(&path, &bytes).expect("write release fixture");
            let mut permissions = fs::metadata(&path).expect("release metadata").permissions();
            permissions.set_mode(if executable { 0o700 } else { 0o600 });
            fs::set_permissions(&path, permissions).expect("set release mode");
            records.insert(
                relative.to_owned(),
                serde_json::json!({
                    "blake3": blake3::Hasher::new().update(&bytes).finalize().to_string(),
                    "mode": if executable { 0o700 } else { 0o600 },
                }),
            );
        };
        for relative in RELEASE_FILES {
            write_release_file(relative, format!("fixture-{relative}").as_bytes());
        }
        for capsule in distro["capsule"].as_array().expect("capsule entries") {
            let source = capsule["source"].as_str().expect("capsule source");
            let name = Path::new(source).file_name().expect("capsule filename");
            let relative = format!("capsules/{}", name.to_string_lossy());
            write_release_file(&relative, b"fixture capsule");
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
        self.install_signed_inventory(&release);
    }

    pub(crate) fn install_signed_inventory(&self, release: &Path) {
        let target = FIXTURE_TARGET;
        let archive_name = format!("unicity-aos-{}-{target}.tar.gz", env!("CARGO_PKG_VERSION"));
        let statement_name = format!("unicity-aos-{}-release.toml", env!("CARGO_PKG_VERSION"));
        let bundle_name = format!("{statement_name}.sigstore.json");
        let statement = release.join("signed").join(&statement_name);
        let bundle = release.join("signed").join(&bundle_name);
        let archive = release.join("signed").join(&archive_name);
        let verifier = release.join("verifier").join("cosign");
        let reference = self.root.join("signed-reference");
        fs::create_dir_all(&reference).expect("create signed reference");

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
[ "$identity" = "https://github.com/unicity-aos/aos-ce/.github/workflows/release.yml@refs/tags/CARGO_VERSION" ]
"#
        .replace(
            "REFERENCE_BUNDLE",
            &self.root.join("reference-bundle").display().to_string(),
        )
        .replace(
            "REFERENCE_STATEMENT",
            &self.root.join("reference-statement").display().to_string(),
        )
        .replace("CARGO_VERSION", env!("CARGO_PKG_VERSION"));
        fs::write(&verifier, cosign).expect("write fixture verifier");
        Self::set_mode(&verifier, 0o700);

        let archive_work = self.root.join(".archive-fixture");
        let archive_root_name = archive_name
            .strip_suffix(".tar.gz")
            .expect("fixture archive suffix");
        let archive_root = archive_work.join(archive_root_name);
        fs::create_dir_all(archive_root.join("bin")).expect("create archive fixture bin");
        fs::create_dir_all(archive_root.join("runtime/bin"))
            .expect("create archive fixture runtime");
        fs::copy(release.join("bin/aos"), archive_root.join("bin/aos"))
            .expect("copy aos into archive fixture");
        for name in ["astrid", "astrid-build", "astrid-daemon", "astrid-emit"] {
            fs::copy(
                release.join("runtime/bin").join(name),
                archive_root.join("runtime/bin").join(name),
            )
            .expect("copy runtime into archive fixture");
        }
        let verifier_digest = Self::sha256(&verifier);
        let archive_manifest = serde_json::json!({
            "schema_version": 2,
            "product": {"version": env!("CARGO_PKG_VERSION")},
            "target": target,
            "verifier": {
                "version": "v3.1.1",
                "asset": "fixture-cosign",
                "sha256": verifier_digest,
            },
        });
        fs::write(
            archive_root.join("release-manifest.json"),
            serde_json::to_vec(&archive_manifest).expect("serialize archive manifest"),
        )
        .expect("write archive fixture manifest");
        let status = Command::new("/usr/bin/tar")
            .env("COPYFILE_DISABLE", "1")
            .args(["-czf"])
            .arg(&archive)
            .arg("-C")
            .arg(&archive_work)
            .arg(archive_root_name)
            .status()
            .expect("create archive fixture");
        assert!(status.success());

        let archive_digest = Self::blake3(&archive);
        let archive_size = fs::symlink_metadata(&archive)
            .expect("archive fixture")
            .len();
        let statement_text = format!(
            r#"schema-version = 1
kind = "aos-release"
product = "unicity-aos-ce"
version = "{}"
tag = "{}"
source-commit = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
published-at = "2026-07-16T10:00:00Z"
release-workflow-identity = "https://github.com/unicity-aos/aos-ce/.github/workflows/release.yml@refs/tags/{}"

[runtime]
release-metadata-available = true
release-metadata-blake3 = "0000000000000000000000000000000000000000000000000000000000000000"

[targets.{target}]
asset = "{archive_name}"
sha256 = "0000000000000000000000000000000000000000000000000000000000000000"
blake3 = "{archive_digest}"
sigstore-bundle = "{archive_name}.sigstore.json"
size = {archive_size}
"#,
            env!("CARGO_PKG_VERSION"),
            env!("CARGO_PKG_VERSION"),
            env!("CARGO_PKG_VERSION")
        );
        fs::write(&statement, &statement_text).expect("write statement fixture");
        fs::write(self.root.join("reference-statement"), statement_text)
            .expect("write statement reference");
        fs::write(&bundle, b"fixture bundle").expect("write bundle fixture");
        fs::write(self.root.join("reference-bundle"), b"fixture bundle")
            .expect("write bundle reference");
        Self::set_mode(&statement, 0o600);
        Self::set_mode(&bundle, 0o600);
        Self::set_mode(&archive, 0o600);

        fs::copy(release.join("bin/aos"), self.home.join("bin/aos"))
            .expect("copy activation fixture");
        Self::set_mode(self.home.join("bin/aos"), 0o700);
    }

    pub(crate) fn set_mode(path: impl AsRef<Path>, mode: u32) {
        let mut permissions = fs::symlink_metadata(path.as_ref())
            .expect("fixture metadata")
            .permissions();
        permissions.set_mode(mode);
        fs::set_permissions(path.as_ref(), permissions).expect("set fixture mode");
    }

    pub(crate) fn sha256(path: &Path) -> String {
        let output = Command::new("/usr/bin/shasum")
            .args(["-a", "256"])
            .arg(path)
            .output()
            .expect("checksum fixture verifier");
        assert!(output.status.success());
        String::from_utf8_lossy(&output.stdout)
            .split_whitespace()
            .next()
            .expect("fixture checksum")
            .to_owned()
    }

    pub(crate) fn blake3(path: &Path) -> String {
        let mut file = fs::File::open(path).expect("open fixture archive");
        let mut hasher = blake3::Hasher::new();
        let mut buffer = vec![0_u8; 64 * 1024];
        loop {
            let read = file.read(&mut buffer).expect("read fixture archive");
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        hasher.finalize().to_string()
    }

    pub(crate) fn default_capsule_dir(&self) -> PathBuf {
        self.home
            .join("releases")
            .join(env!("CARGO_PKG_VERSION"))
            .join("capsules")
    }

    pub(crate) fn selected_distro(&self) -> PathBuf {
        self.home
            .join("releases")
            .join(env!("CARGO_PKG_VERSION"))
            .join("Distro.toml")
    }

    pub(crate) fn install_runtime(&self, body: &str) {
        fs::write(&self.runtime, body).expect("write fake runtime");
        Self::make_executable(&self.runtime);
        self.refresh_release_record("runtime/bin/astrid");
        let release = self.home.join("releases").join(env!("CARGO_PKG_VERSION"));
        self.install_signed_inventory(&release);
    }

    pub(crate) fn install_daemon(&self, body: &str) {
        let daemon = self.runtime.with_file_name("astrid-daemon");
        fs::write(&daemon, body).expect("write fake daemon");
        Self::make_executable(&daemon);
        self.refresh_release_record("runtime/bin/astrid-daemon");
        let release = self.home.join("releases").join(env!("CARGO_PKG_VERSION"));
        self.install_signed_inventory(&release);
    }

    pub(crate) fn make_executable(path: &Path) {
        let mut permissions = fs::metadata(path).expect("runtime metadata").permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions).expect("make fixture executable");
    }

    pub(crate) fn refresh_release_record(&self, relative: &str) {
        let release = self.home.join("releases").join(env!("CARGO_PKG_VERSION"));
        let path = release.join(relative);
        let bytes = fs::read(&path).expect("read refreshed runtime");
        let mode = fs::metadata(&path)
            .expect("read refreshed mode")
            .permissions()
            .mode();
        let manifest_path = release.join("release-manifest.json");
        let mut manifest: serde_json::Value =
            serde_json::from_slice(&fs::read(&manifest_path).expect("read release manifest"))
                .expect("decode release manifest");
        manifest["release_files"][relative] = serde_json::json!({
            "blake3": blake3::Hasher::new().update(&bytes).finalize().to_string(),
            "mode": mode & 0o777,
        });
        fs::write(
            &manifest_path,
            serde_json::to_vec(&manifest).expect("serialize release manifest"),
        )
        .expect("write refreshed release manifest");
    }

    pub(crate) fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_aos"));
        command
            .env("AOS_HOME", &self.home)
            .env("AOS_TEST_ARGS", &self.args)
            .env("AOS_TEST_BOOTSTRAP_ARGS", &self.bootstrap_args)
            .env("AOS_TEST_HOME", self.root.join("child-home"))
            .env("AOS_TEST_WORKSPACE", self.root.join("child-workspace"))
            .env("AOS_TEST_DISTRO", self.root.join("child-distro"))
            .env(
                "AOS_TEST_CLIENT_CONFIG",
                self.root.join("child-client-config"),
            )
            .env("AOS_TEST_LIFECYCLE", self.root.join("lifecycle"))
            .env("AOS_TEST_APPLY_ARGS", self.root.join("apply-args"))
            .env("AOS_TEST_PATH", &self.child_path);
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

pub(crate) const RECORDING_RUNTIME: &str = r#"#!/bin/sh
record_runtime_state() {
    destination="$1"
    if [ -d "$ASTRID_HOME" ]; then
        find "$ASTRID_HOME" -mindepth 1 -maxdepth 1 -exec basename {} \; | sort > "$destination"
    else
        : > "$destination"
    fi
}

if [ -d "$ASTRID_HOME" ]; then
    record_runtime_state "${AOS_TEST_RUNTIME_STATE_PREFIX:-$AOS_TEST_ARGS}.state"
else
    : > "${AOS_TEST_RUNTIME_STATE_PREFIX:-$AOS_TEST_ARGS}.state"
fi
if [ "$1" = start ]; then
    record_runtime_state "${AOS_TEST_RUNTIME_STATE_PREFIX:-$AOS_TEST_ARGS}.start"
fi
case "$1" in
    start)
        if [ "${AOS_TEST_FOREIGN_PIN:-0}" = 1 ]; then
            mkdir -p "$ASTRID_HOME/trust"
            chmod 700 "$ASTRID_HOME/trust"
            printf 'foreign-key\n' > "$ASTRID_HOME/trust/unicity-ce.pub"
        elif [ "${AOS_TEST_SYMLINK_PIN:-0}" = 1 ]; then
            mkdir -p "$ASTRID_HOME/trust" "$AOS_TEST_PIN_TARGET_DIR"
            chmod 700 "$ASTRID_HOME/trust" "$AOS_TEST_PIN_TARGET_DIR"
            printf 'foreign-key\n' > "$AOS_TEST_PIN_TARGET_DIR/key.pub"
            ln -s "$AOS_TEST_PIN_TARGET_DIR/key.pub" "$ASTRID_HOME/trust/unicity-ce.pub"
        elif [ "${AOS_TEST_NO_MOUNTED_PIN:-0}" != 1 ]; then
            mkdir -p "$ASTRID_HOME/trust"
            chmod 700 "$ASTRID_HOME/trust"
            [ -e "$ASTRID_HOME/trust/unicity-ce.pub" ] ||
                printf 'ed25519:utH537RuOuqKwjGx/pHIUAkKapyqPUhHpZIVDU6Q0FA=\n' \
                    > "$ASTRID_HOME/trust/unicity-ce.pub"
        fi
        ;;
    stop)
        record_runtime_state "${AOS_TEST_RUNTIME_STATE_PREFIX:-$AOS_TEST_ARGS}.stop-before"
        find "$ASTRID_HOME" -mindepth 1 -maxdepth 1 ! -name astrid.volume \
            -exec rm -rf {} +
        [ -f "$ASTRID_HOME/astrid.volume" ] || printf 'packed-volume\n' \
            > "$ASTRID_HOME/astrid.volume"
        ;;
esac
printf '%s\n' "$1" >> "${AOS_TEST_LIFECYCLE:-/dev/null}"
if [ -d "$ASTRID_HOME" ]; then
    record_runtime_state "${AOS_TEST_RUNTIME_STATE_PREFIX:-$AOS_TEST_ARGS}.after"
else
    : > "${AOS_TEST_RUNTIME_STATE_PREFIX:-$AOS_TEST_ARGS}.after"
fi
printf '%s\n' "$0" > "${AOS_TEST_SELF:-/dev/null}"
output="$AOS_TEST_ARGS"
seen_distro=0
for arg in "$@"; do
    printf '<%s>\n' "$arg"
done > "$output"
for arg in "$@"; do
    [ "$arg" = distro ] && seen_distro=1
done
if [ "$seen_distro" -eq 1 ]; then
    cp "$output" "${AOS_TEST_APPLY_ARGS:-/dev/null}"
    record_runtime_state "${AOS_TEST_RUNTIME_STATE_PREFIX:-$AOS_TEST_ARGS}.apply"
fi
if [ "$1" = stop ]; then
    record_runtime_state "${AOS_TEST_RUNTIME_STATE_PREFIX:-$AOS_TEST_ARGS}.stop"
fi
recorded_command="${1:-unknown}"
for argument in "$@"; do
    case "$argument" in
        start|stop|apply) recorded_command="$argument" ;;
    esac
done
if [ -n "${AOS_TEST_RUNTIME_PIN_PREFIX:-}" ]; then
    pin_destination="${AOS_TEST_RUNTIME_PIN_PREFIX}.${recorded_command}"
    if [ -f "$ASTRID_HOME/trust/unicity-ce.pub" ]; then
        cp "$ASTRID_HOME/trust/unicity-ce.pub" "$pin_destination"
    else
        : > "$pin_destination"
    fi
fi
if [ -n "${AOS_TEST_RUN_DIR_PREFIX:-}" ]; then
    printf '%s\n' "$ASTRID_RUN_DIR" > "${AOS_TEST_RUN_DIR_PREFIX}.${recorded_command}"
fi
printf '%s\n' "$ASTRID_HOME" > "$AOS_TEST_HOME"
printf '%s\n' "$ASTRID_WORKSPACE_STATE_DIR" > "$AOS_TEST_WORKSPACE"
printf '%s\n' "$ASTRID_ENFORCED_DISTRO" > "$AOS_TEST_DISTRO"
printf '%s\n' "$ASTRID_CLIENT_CONFIG_PATH" > "$AOS_TEST_CLIENT_CONFIG"
printf '%s\n' "$PATH" > "$AOS_TEST_PATH"
if [ "$1" = start ]; then
    exit "${AOS_TEST_START_EXIT:-${AOS_TEST_EXIT:-0}}"
fi
exit "${AOS_TEST_EXIT:-0}"
"#;

pub(crate) const RECORDING_DAEMON: &str = r#"#!/bin/sh
for arg in "$@"; do
    printf '<%s>\n' "$arg"
done > "$AOS_TEST_ARGS"
printf '%s\n' "$ASTRID_HOME" > "$AOS_TEST_HOME"
printf '%s\n' "$ASTRID_WORKSPACE_STATE_DIR" > "$AOS_TEST_WORKSPACE"
printf '%s\n' "$ASTRID_ENFORCED_DISTRO" > "$AOS_TEST_DISTRO"
printf '%s\n' "$ASTRID_CLIENT_CONFIG_PATH" > "$AOS_TEST_CLIENT_CONFIG"
printf '%s\n' "$ASTRID_DAEMON_LOG_TARGET" > "$AOS_TEST_LOG_TARGET"
exit "${AOS_TEST_EXIT:-0}"
"#;

pub(crate) fn seed_stopped_volume(fixture: &Fixture) {
    fs::create_dir_all(fixture.home.join("runtime")).expect("create runtime home");
    fs::write(fixture.home.join("runtime/astrid.volume"), b"volume-state")
        .expect("create runtime volume");
}

pub(crate) fn seed_active_receipt(fixture: &Fixture) {
    let home = unicity_aos_bootstrap::AosHome::from_root(&fixture.home);
    let selected = fixture.selected_distro();
    let key = unicity_aos_bootstrap::distro_trust::selected_signing_key(&selected)
        .expect("read selected signing key");
    unicity_aos_bootstrap::distro_trust::write_active_receipt(&home, &selected, "alice", &key)
        .expect("seed active apply receipt");
}

pub(crate) fn shell_literal_path(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "'\\''")
}
