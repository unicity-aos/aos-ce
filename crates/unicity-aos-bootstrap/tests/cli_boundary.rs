#![cfg(unix)]

use std::ffi::OsStr;
use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use astrid_crypto::KeyPair;
use serde::Serialize;

struct Fixture {
    root: PathBuf,
    runtime: PathBuf,
    args: PathBuf,
    bootstrap_args: PathBuf,
    home: PathBuf,
    child_path: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "aos-cli-boundary-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create fixture");
        let runtime = root.join("fake-runtime");
        let args = root.join("args");
        let bootstrap_args = root.join("bootstrap-args");
        let home = root.join("runtime-home");
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

    fn install_capsules(&self) {
        let distro: toml::Value = include_str!("../../../distros/community/unicity-ce/Distro.toml")
            .parse()
            .expect("parse embedded distro fixture");
        let directory = self
            .home
            .join("releases")
            .join(env!("CARGO_PKG_VERSION"))
            .join("capsules");
        fs::create_dir_all(&directory).expect("create capsule fixture");
        for capsule in distro["capsule"].as_array().expect("capsule entries") {
            let source = capsule["source"].as_str().expect("capsule source");
            let name = Path::new(source).file_name().expect("capsule filename");
            fs::write(directory.join(name), b"fixture capsule").expect("write capsule fixture");
        }
    }

    fn default_capsule_dir(&self) -> PathBuf {
        self.home
            .join("releases")
            .join(env!("CARGO_PKG_VERSION"))
            .join("capsules")
    }

    fn release_dir(&self) -> PathBuf {
        self.home.join("releases").join(env!("CARGO_PKG_VERSION"))
    }

    fn install_signed_distro(&self) {
        let keypair = KeyPair::from_secret_key(&[7_u8; 32]).expect("fixture signing key");
        let signing_pubkey = format!("ed25519:{}", keypair.export_public_key().to_base64());
        let embedded = include_str!("../../../distros/community/unicity-ce/Distro.toml");
        let original_pubkey = "ed25519:utH537RuOuqKwjGx/pHIUAkKapyqPUhHpZIVDU6Q0FA=";
        let manifest = embedded.replace(original_pubkey, &signing_pubkey);
        let manifest_hash = format!("blake3:{}", blake3::hash(manifest.as_bytes()).to_hex());
        let lock = FixtureLock {
            schema_version: 1,
            distro: FixtureLockMeta {
                id: "unicity-ce".to_owned(),
                version: env!("CARGO_PKG_VERSION").to_owned(),
                resolved_at: "2026-01-01T00:00:00Z".to_owned(),
            },
            capsules: Vec::new(),
            manifest_hash: Some(manifest_hash),
        };
        let lock_bytes = toml::to_string_pretty(&lock)
            .expect("serialize fixture Distro.lock")
            .into_bytes();
        let canonical_lock = serde_json::to_vec(&lock).expect("canonicalize fixture Distro.lock");
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"astrid-distro-lock-sig-v1\0");
        hasher.update(&canonical_lock);
        let signature = keypair.sign(hasher.finalize().as_bytes()).to_hex();
        let release = self.release_dir();
        fs::write(release.join("Distro.toml"), manifest).expect("write fixture Distro.toml");
        fs::write(release.join("Distro.lock"), lock_bytes).expect("write fixture Distro.lock");
        fs::write(release.join("Distro.sig"), format!("{signature}\n"))
            .expect("write fixture Distro.sig");
        for name in ["Distro.toml", "Distro.lock", "Distro.sig"] {
            let path = release.join(name);
            let mut permissions = fs::metadata(&path)
                .expect("inspect fixture distro member")
                .permissions();
            permissions.set_mode(0o600);
            fs::set_permissions(path, permissions).expect("make fixture distro member private");
        }
    }

    fn install_runtime(&self, body: &str) {
        fs::write(&self.runtime, body).expect("write fake runtime");
        Self::make_executable(&self.runtime);
    }

    fn install_daemon(&self, body: &str) {
        let daemon = self.runtime.with_file_name("astrid-daemon");
        fs::write(&daemon, body).expect("write fake daemon");
        Self::make_executable(&daemon);
    }

    fn make_executable(path: &Path) {
        let mut permissions = fs::metadata(path).expect("runtime metadata").permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(path, permissions).expect("make fixture executable");
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_aos"));
        command
            .env("AOS_HOME", &self.home)
            .env("UNICITY_AOS_RUNTIME_BIN", &self.runtime)
            .env("AOS_TEST_ARGS", &self.args)
            .env("AOS_TEST_BOOTSTRAP_ARGS", &self.bootstrap_args)
            .env("AOS_TEST_HOME", self.root.join("child-home"))
            .env("AOS_TEST_WORKSPACE", self.root.join("child-workspace"))
            .env("AOS_TEST_DISTRO", self.root.join("child-distro"))
            .env("AOS_TEST_PATH", &self.child_path);
        command
            .env("AOS_TEST_START", self.root.join("start-args"))
            .env("AOS_TEST_APPLY", self.root.join("apply-args"))
            .env("AOS_TEST_STOP", self.root.join("stop-args"))
            .env("AOS_TEST_APPLY_DISTRO", self.root.join("apply-distro"));
        command
    }
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
struct FixtureLock {
    schema_version: u32,
    distro: FixtureLockMeta,
    #[serde(rename = "capsule")]
    capsules: Vec<FixtureCapsule>,
    #[serde(skip_serializing_if = "Option::is_none")]
    manifest_hash: Option<String>,
}

#[derive(Serialize)]
#[serde(rename_all = "kebab-case")]
struct FixtureLockMeta {
    id: String,
    version: String,
    resolved_at: String,
}

#[derive(Serialize)]
struct FixtureCapsule {
    name: String,
    version: String,
    source: String,
    hash: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    resolved_ref: Option<String>,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

const RECORDING_RUNTIME: &str = r#"#!/bin/sh
if [ "$1" = "start" ]; then
    for arg in "$@"; do
        printf '<%s>\n' "$arg"
    done | tee "$AOS_TEST_ARGS" > "$AOS_TEST_START"
    mkdir -p "$ASTRID_HOME/trust"
    exit "${AOS_TEST_EXIT:-0}"
elif [ "$1" = "--principal" ] && [ "$3" = "distro" ] && [ "$4" = "apply" ]; then
    for arg in "$@"; do
        printf '<%s>\n' "$arg"
    done | tee "$AOS_TEST_ARGS" > "$AOS_TEST_APPLY"
    printf '%s\n' "$ASTRID_ENFORCED_DISTRO" > "$AOS_TEST_APPLY_DISTRO"
    exit "${AOS_TEST_APPLY_EXIT:-${AOS_TEST_EXIT:-0}}"
elif [ "$1" = "stop" ]; then
    for arg in "$@"; do
        printf '<%s>\n' "$arg"
    done | tee "$AOS_TEST_ARGS" > "$AOS_TEST_STOP"
    if [ "${AOS_TEST_KEEP_TRUST:-0}" = "1" ]; then
        find "$ASTRID_HOME" -mindepth 1 -maxdepth 1 ! -name astrid.volume ! -name trust -exec rm -rf {} +
    else
        find "$ASTRID_HOME" -mindepth 1 -maxdepth 1 ! -name astrid.volume -exec rm -rf {} +
    fi
    printf 'volume-state\n' > "$ASTRID_HOME/astrid.volume"
    chmod 600 "$ASTRID_HOME/astrid.volume"
    exit "${AOS_TEST_EXIT:-0}"
elif [ "$1" = "--principal" ] && [ "$2" = "default" ] && [ "$3" = "init" ]; then
    output="$AOS_TEST_BOOTSTRAP_ARGS"
else
    output="$AOS_TEST_ARGS"
fi
for arg in "$@"; do
    printf '<%s>\n' "$arg"
done > "$output"
printf '%s\n' "$ASTRID_HOME" > "$AOS_TEST_HOME"
printf '%s\n' "$ASTRID_WORKSPACE_STATE_DIR" > "$AOS_TEST_WORKSPACE"
printf '%s\n' "$ASTRID_ENFORCED_DISTRO" > "$AOS_TEST_DISTRO"
printf '%s\n' "$PATH" > "$AOS_TEST_PATH"
exit "${AOS_TEST_EXIT:-0}"
"#;

const RECORDING_DAEMON: &str = r#"#!/bin/sh
for arg in "$@"; do
    printf '<%s>\n' "$arg"
done > "$AOS_TEST_ARGS"
printf '%s\n' "$ASTRID_HOME" > "$AOS_TEST_HOME"
printf '%s\n' "$ASTRID_WORKSPACE_STATE_DIR" > "$AOS_TEST_WORKSPACE"
printf '%s\n' "$ASTRID_ENFORCED_DISTRO" > "$AOS_TEST_DISTRO"
printf '%s\n' "$ASTRID_DAEMON_LOG_TARGET" > "$AOS_TEST_LOG_TARGET"
exit "${AOS_TEST_EXIT:-0}"
"#;

#[test]
fn foreground_daemon_replaces_aos_with_the_persistent_product_runtime() {
    let fixture = Fixture::new("foreground-daemon");
    fixture.install_daemon(RECORDING_DAEMON);
    let workspace = fixture.root.join("workspace");
    let log_target = fixture.root.join("log-target");

    let output = fixture
        .command()
        .env("AOS_TEST_EXIT", "23")
        .env("AOS_TEST_LOG_TARGET", &log_target)
        .args([
            OsStr::new("daemon"),
            OsStr::new("foreground"),
            OsStr::new("--workspace"),
            workspace.as_os_str(),
            OsStr::new("--verbose"),
        ])
        .output()
        .expect("run foreground daemon");

    assert_eq!(
        output.status.code(),
        Some(23),
        "the daemon must directly own the supervisor-visible exit status"
    );
    assert_eq!(
        fs::read_to_string(&fixture.args).expect("read daemon args"),
        format!("<--workspace>\n<{}>\n<--verbose>\n", workspace.display())
    );
    assert!(
        !fs::read_to_string(&fixture.args)
            .expect("read daemon args")
            .contains("--ephemeral")
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("child-home"))
            .expect("read runtime home")
            .trim(),
        fixture.home.join("runtime").to_string_lossy()
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("child-workspace"))
            .expect("read workspace state")
            .trim(),
        ".aos"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("child-distro"))
            .expect("read enforced distro")
            .trim(),
        fixture
            .home
            .join("distributions/unicity-ce/Distro.toml")
            .to_string_lossy()
    );
    assert_eq!(
        fs::read_to_string(&log_target)
            .expect("read daemon log target")
            .trim(),
        "stderr"
    );
}

#[test]
fn unowned_root_passes_through_with_argv_home_and_exit_code() {
    let fixture = Fixture::new("passthrough");
    fixture.install_runtime(RECORDING_RUNTIME);

    let output = fixture
        .command()
        .env("AOS_TEST_EXIT", "37")
        .args(["doctor", "--json", "space value", "$(not-a-shell)"])
        .output()
        .expect("run aos");

    assert_eq!(output.status.code(), Some(37));
    assert_eq!(
        fs::read_to_string(&fixture.args).expect("read delegated args"),
        "<doctor>\n<--json>\n<space value>\n<$(not-a-shell)>\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("child-home")).expect("read runtime home"),
        format!("{}\n", fixture.home.join("runtime").display())
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("child-workspace")).expect("read workspace"),
        ".aos\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("child-distro")).expect("read distro"),
        format!(
            "{}\n",
            fixture
                .home
                .join("distributions/unicity-ce/Distro.toml")
                .display()
        )
    );
    let child_path = fs::read_to_string(&fixture.child_path).expect("read child PATH");
    assert_eq!(
        std::env::split_paths(OsStr::new(child_path.trim())).next(),
        fixture.runtime.parent().map(Path::to_path_buf)
    );
}

#[test]
fn product_mcp_bridge_adds_local_form_support_and_rebrands_the_server() {
    let fixture = Fixture::new("product-mcp");
    fixture.install_runtime(
        r#"#!/bin/sh
printf '<%s>\n' "$@" > "$AOS_TEST_ARGS"
IFS= read -r initialize
printf '%s\n' "$initialize" > "$AOS_TEST_BOOTSTRAP_ARGS"
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"protocolVersion":"2025-11-25","capabilities":{"tools":{}},"serverInfo":{"name":"astrid","version":"0.10.4"}}}'
while IFS= read -r _line; do :; done
"#,
    );

    let mut child = fixture
        .command()
        .args(["--principal", "grok-code", "mcp", "serve"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("start product MCP bridge");
    let mut stdin = child.stdin.take().expect("bridge stdin");
    writeln!(
        stdin,
        "{}",
        serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "initialize",
            "params": {
                "protocolVersion": "2025-11-25",
                "capabilities": {},
                "clientInfo": { "name": "grok", "version": "1" }
            }
        })
    )
    .expect("write initialize");
    drop(stdin);
    let output = child.wait_with_output().expect("wait for MCP bridge");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(&fixture.args).expect("read runtime args"),
        "<--principal>\n<grok-code>\n<mcp>\n<serve>\n"
    );
    let forwarded: serde_json::Value = serde_json::from_str(
        fs::read_to_string(&fixture.bootstrap_args)
            .expect("read forwarded initialize")
            .trim(),
    )
    .expect("forwarded initialize JSON");
    assert!(
        forwarded
            .pointer("/params/capabilities/elicitation/form")
            .is_some()
    );
    let response: serde_json::Value =
        serde_json::from_slice(&output.stdout).expect("product initialize response");
    assert_eq!(response["result"]["serverInfo"]["name"], "unicity-aos");
    assert_eq!(response["result"]["serverInfo"]["title"], "Unicity AOS");
}

#[test]
fn leading_runtime_globals_on_unowned_roots_pass_through_exactly() {
    let fixture = Fixture::new("leading-global-passthrough");
    fixture.install_runtime(RECORDING_RUNTIME);

    let output = fixture
        .command()
        .args(["--principal", "alice", "doctor", "--json"])
        .output()
        .expect("run inherited command with a leading global");

    assert!(output.status.success());
    assert_eq!(
        fs::read_to_string(&fixture.args).expect("read delegated args"),
        "<--principal>\n<alice>\n<doctor>\n<--json>\n"
    );
}

#[test]
fn inherited_stop_succeeds_only_after_the_runtime_is_confirmed_stopped() {
    let fixture = Fixture::new("confirmed-stop");
    fixture.install_runtime(
        r#"#!/bin/sh
for arg in "$@"; do
    echo "<$arg>"
done > "$AOS_TEST_ARGS"
echo 'error: connection lost waiting on astrid.v1.response.shutdown.test: connection lost: connection closed before astrid.v1.response.shutdown.test' >&2
exit 1
"#,
    );

    let ready_marker = fixture.home.join("runtime/run/system.ready");
    fs::create_dir_all(ready_marker.parent().expect("runtime run directory"))
        .expect("create runtime run directory");
    fs::write(&ready_marker, []).expect("create runtime ready marker");
    let marker_to_remove = ready_marker.clone();
    let run_dir = fixture.home.join("runtime/run");
    let volume = fixture.home.join("runtime/astrid.volume");
    let shutdown = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));

        // The runtime must finish materializing its private volume before the
        // readiness marker is removed, so AOS cannot report a partial stop.
        fs::write(&volume, b"volume-state").expect("create private runtime volume");
        let mut permissions = fs::metadata(&volume)
            .expect("inspect private runtime volume")
            .permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(&volume, permissions).expect("make runtime volume private");
        fs::remove_file(marker_to_remove).expect("remove runtime ready marker");
        fs::remove_dir_all(run_dir).expect("remove runtime run directory");
    });

    let output = fixture
        .command()
        .args(["--future-runtime-global", "future-value", "stop"])
        .output()
        .expect("run inherited stop");
    shutdown.join().expect("finish runtime shutdown");

    assert!(output.status.success());
    assert_eq!(
        fs::read_to_string(&fixture.args).expect("read delegated stop args"),
        "<--future-runtime-global>\n<future-value>\n<stop>\n"
    );
    assert!(!ready_marker.exists());
    let runtime = fixture.home.join("runtime");
    let entries: Vec<_> = fs::read_dir(&runtime)
        .expect("read stopped runtime state")
        .map(|entry| {
            entry
                .expect("read stopped runtime entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(entries, vec!["astrid.volume"]);
    let volume = runtime.join("astrid.volume");
    let metadata = fs::symlink_metadata(&volume).expect("inspect stopped runtime volume");
    assert!(metadata.is_file());
    assert!(!metadata.file_type().is_symlink());
    assert!(metadata.len() > 0);
    assert_eq!(metadata.permissions().mode() & 0o7777, 0o600);
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8 stop output"),
        "Unicity AOS stopped.\n"
    );
}

#[test]
fn inherited_exit_zero_stop_waits_for_confirmation() {
    let fixture = Fixture::new("confirmed-zero-stop");
    fixture.install_runtime(
        r#"#!/bin/sh
echo 'runtime stop complete'
exit 0
"#,
    );

    let ready_marker = fixture.home.join("runtime/run/system.ready");
    fs::create_dir_all(ready_marker.parent().expect("runtime run directory"))
        .expect("create runtime run directory");
    fs::write(&ready_marker, []).expect("create runtime ready marker");
    let marker_to_remove = ready_marker.clone();
    let run_dir = fixture.home.join("runtime/run");
    let volume = fixture.home.join("runtime/astrid.volume");
    let shutdown = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        fs::write(&volume, b"volume-state").expect("create private runtime volume");
        let mut permissions = fs::metadata(&volume)
            .expect("inspect private runtime volume")
            .permissions();
        permissions.set_mode(0o600);
        fs::set_permissions(&volume, permissions).expect("make runtime volume private");
        fs::remove_file(marker_to_remove).expect("remove runtime ready marker");
        fs::remove_dir_all(run_dir).expect("remove runtime run directory");
    });

    let output = fixture
        .command()
        .arg("stop")
        .output()
        .expect("run successful inherited stop");
    shutdown.join().expect("finish runtime shutdown");

    assert!(output.status.success());
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8 stop output"),
        "runtime stop complete\n"
    );
    assert!(output.stderr.is_empty());
    assert!(!ready_marker.exists());
    let runtime = fixture.home.join("runtime");
    let entries: Vec<_> = fs::read_dir(&runtime)
        .expect("read stopped runtime state")
        .map(|entry| {
            entry
                .expect("read stopped runtime entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(entries, vec!["astrid.volume"]);
}

#[test]
fn inherited_exit_zero_stop_fails_while_the_runtime_token_remains() {
    let fixture = Fixture::new("unconfirmed-zero-stop");
    fixture.install_runtime(
        r#"#!/bin/sh
echo 'runtime claimed stop complete'
exit 0
"#,
    );

    let token = fixture.home.join("runtime/run/system.token");
    fs::create_dir_all(token.parent().expect("runtime run directory"))
        .expect("create runtime run directory");
    fs::write(&token, b"stale token").expect("create stale runtime token");

    let started = Instant::now();
    let output = fixture
        .command()
        .arg("stop")
        .output()
        .expect("run unconfirmed inherited stop");

    assert_eq!(output.status.code(), Some(1));
    assert!(
        started.elapsed() < Duration::from_secs(10),
        "stop confirmation must remain bounded"
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8 stop output"),
        "runtime claimed stop complete\n"
    );
    let stderr = String::from_utf8(output.stderr).expect("utf8 stop error");
    assert!(stderr.contains("aos: shutdown confirmation failed:"));
    assert!(stderr.contains("system.token"));
    assert!(token.exists(), "confirmation must not hide a stale marker");
}

#[test]
fn inherited_stop_preserves_the_primary_failure_before_confirmation_failure() {
    let fixture = Fixture::new("primary-and-confirmation-failure");
    fixture.install_runtime(
        r#"#!/bin/sh
echo 'primary runtime stop failure' >&2
exit 23
"#,
    );

    let gateway = fixture.home.join("runtime/run/mcp-gateway.ready");
    fs::create_dir_all(gateway.parent().expect("runtime run directory"))
        .expect("create runtime run directory");
    fs::write(&gateway, b"stale gateway").expect("create stale gateway marker");

    let output = fixture
        .command()
        .arg("stop")
        .output()
        .expect("run failed inherited stop");

    assert_eq!(output.status.code(), Some(23));
    let stderr = String::from_utf8(output.stderr).expect("utf8 stop error");
    let primary = stderr
        .find("primary runtime stop failure")
        .expect("primary failure must be retained");
    let confirmation = stderr
        .find("aos: shutdown confirmation failed:")
        .expect("confirmation failure must be reported separately");
    assert!(primary < confirmation);
    assert!(stderr.contains("mcp-gateway.ready"));
    assert!(
        gateway.exists(),
        "confirmation must not hide a stale gateway marker"
    );
}

#[test]
fn expected_disconnect_is_not_suppressed_when_confirmation_fails() {
    let fixture = Fixture::new("disconnect-and-confirmation-failure");
    fixture.install_runtime(
        r#"#!/bin/sh
echo 'error: connection lost waiting on astrid.v1.response.shutdown.test: connection lost: connection closed before astrid.v1.response.shutdown.test' >&2
exit 1
"#,
    );

    let gateway = fixture.home.join("runtime/run/mcp-gateway.sock");
    fs::create_dir_all(gateway.parent().expect("runtime run directory"))
        .expect("create runtime run directory");
    fs::write(&gateway, b"stale gateway endpoint").expect("create stale gateway endpoint");

    let output = fixture
        .command()
        .arg("stop")
        .output()
        .expect("run disconnected inherited stop");

    assert_eq!(output.status.code(), Some(1));
    let stderr = String::from_utf8(output.stderr).expect("utf8 stop error");
    let disconnect = stderr
        .find("connection lost waiting on astrid.v1.response.shutdown.test")
        .expect("disconnect must remain visible when confirmation fails");
    let confirmation = stderr
        .find("aos: shutdown confirmation failed:")
        .expect("confirmation failure must be reported separately");
    assert!(disconnect < confirmation);
    assert!(stderr.contains("mcp-gateway.sock"));
    assert!(
        gateway.exists(),
        "confirmation must not hide a stale gateway endpoint"
    );
}

#[test]
fn inherited_stop_does_not_mask_other_runtime_failures() {
    let fixture = Fixture::new("failed-stop");
    fixture.install_runtime(
        r#"#!/bin/sh
echo 'invalid stop argument' >&2
exit 2
"#,
    );

    let output = fixture
        .command()
        .args(["stop", "--invalid"])
        .output()
        .expect("run rejected inherited stop");

    assert_eq!(output.status.code(), Some(2));
    assert_eq!(
        String::from_utf8(output.stderr).expect("utf8 stop error"),
        "invalid stop argument\n"
    );
}

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
    let contract: toml::Value = include_str!("../../../release/runtime-command-surface.toml")
        .parse()
        .expect("parse runtime command surface");
    let roots = contract["roots"].as_table().expect("root classifications");
    let direct_roots = ["inherited", "hidden-inherited"]
        .into_iter()
        .flat_map(|bucket| roots[bucket].as_array().expect("root classification list"))
        .map(|root| root.as_str().expect("runtime root string"));

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

#[test]
fn product_default_init_delegates_grants_without_inventing_a_target() {
    let fixture = Fixture::new("init-default");
    fixture.install_runtime(RECORDING_RUNTIME);

    let status = fixture
        .command()
        .args(["init"])
        .status()
        .expect("run product init");
    assert!(status.success());
    let args = fs::read_to_string(&fixture.args).expect("read init args");
    assert_eq!(args, "<init>\n<--grant-capsules>\n");
    assert_eq!(
        fs::read_to_string(&fixture.bootstrap_args).expect("read bootstrap args"),
        "<--principal>\n<default>\n<init>\n<--target-principal>\n<default>\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("child-distro")).expect("read enforced distro"),
        format!(
            "{}\n",
            fixture
                .home
                .join("distributions/unicity-ce/Distro.toml")
                .display()
        )
    );
}

#[test]
fn product_init_stops_before_runtime_dispatch_when_system_fleet_init_fails() {
    let fixture = Fixture::new("init-bootstrap-failure");
    fixture.install_runtime(RECORDING_RUNTIME);

    let output = fixture
        .command()
        .env("AOS_TEST_EXIT", "42")
        .arg("init")
        .output()
        .expect("run product init with a failing bootstrap installer");

    assert!(!output.status.success());
    assert!(fixture.bootstrap_args.exists());
    assert!(!fixture.args.exists());
    assert!(
        String::from_utf8(output.stderr)
            .expect("utf8 stderr")
            .contains("bundled CE system-fleet initializer exited")
    );
}

#[test]
fn product_non_default_init_delegates_principal_and_capsule_grants() {
    let fixture = Fixture::new("init-principal");
    fixture.install_runtime(RECORDING_RUNTIME);

    let status = fixture
        .command()
        .args([
            "init",
            "--target-principal",
            "alice",
            "--yes",
            "--var",
            "model=gpt-5",
        ])
        .status()
        .expect("run product init for a non-default principal");
    assert!(status.success());
    assert_eq!(
        fs::read_to_string(&fixture.args).expect("read non-default init args"),
        "<init>\n<--target-principal>\n<alice>\n<--yes>\n<--var>\n<model=gpt-5>\n<--grant-capsules>\n"
    );
    assert_eq!(
        fs::read_to_string(&fixture.bootstrap_args).expect("read bootstrap args"),
        "<--principal>\n<default>\n<init>\n<--target-principal>\n<default>\n<--yes>\n<--var>\n<model=gpt-5>\n"
    );
}

#[test]
fn offline_init_keeps_the_runtime_offline_flag_and_uses_only_local_capsules() {
    let fixture = Fixture::new("init-offline");
    fixture.install_runtime(RECORDING_RUNTIME);

    let status = fixture
        .command()
        .args(["init", "--offline"])
        .status()
        .expect("run offline product init");
    assert!(status.success());
    assert_eq!(
        fs::read_to_string(&fixture.args).expect("read offline args"),
        "<init>\n<--offline>\n<--grant-capsules>\n"
    );
    assert_eq!(
        fs::read_to_string(&fixture.bootstrap_args).expect("read bootstrap args"),
        "<--principal>\n<default>\n<init>\n<--target-principal>\n<default>\n<--offline>\n"
    );
    let manifest_path = fixture.home.join("distributions/unicity-ce/Distro.toml");
    let manifest: toml::Value = fs::read_to_string(manifest_path)
        .expect("read materialized manifest")
        .parse()
        .expect("parse materialized manifest");
    let capsules = manifest["capsule"].as_array().expect("capsule entries");
    let embedded: toml::Value = include_str!("../../../distros/community/unicity-ce/Distro.toml")
        .parse()
        .expect("parse embedded distro fixture");
    assert_eq!(
        capsules.len(),
        embedded["capsule"]
            .as_array()
            .expect("embedded capsule entries")
            .len()
    );
    let expected_root = fixture
        .home
        .join("releases")
        .join(env!("CARGO_PKG_VERSION"))
        .join("capsules")
        .canonicalize()
        .expect("canonical capsule root");
    assert!(capsules.iter().all(|capsule| {
        let source = Path::new(capsule["source"].as_str().expect("source"));
        source.is_absolute() && source.parent() == Some(expected_root.as_path())
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
    let manifest: toml::Value =
        fs::read_to_string(fixture.home.join("distributions/unicity-ce/Distro.toml"))
            .expect("read materialized override manifest")
            .parse()
            .expect("parse materialized override manifest");
    let canonical = custom.canonicalize().expect("canonical custom capsules");
    assert!(
        manifest["capsule"]
            .as_array()
            .expect("capsules")
            .iter()
            .all(
                |capsule| Path::new(capsule["source"].as_str().expect("source")).parent()
                    == Some(canonical.as_path())
            )
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
fn product_init_preserves_authenticated_operator_and_separate_target() {
    let fixture = Fixture::new("init-operator-target");
    fixture.install_runtime(RECORDING_RUNTIME);

    let status = fixture
        .command()
        .args([
            "--principal",
            "operator",
            "init",
            "--target-principal",
            "alice",
            "--yes",
        ])
        .status()
        .expect("run product init with an explicit operator");

    assert!(status.success());
    assert_eq!(
        fs::read_to_string(&fixture.args).expect("read operator init args"),
        "<--principal>\n<operator>\n<init>\n<--target-principal>\n<alice>\n<--yes>\n<--grant-capsules>\n"
    );
}

#[test]
fn product_init_rejects_caller_distro_selection() {
    let fixture = Fixture::new("init-distro-override");
    fixture.install_runtime(RECORDING_RUNTIME);

    let output = fixture
        .command()
        .args(["init", "--distro=other"])
        .output()
        .expect("run protected init");
    assert_eq!(output.status.code(), Some(2));
    assert!(!fixture.args.exists());
    assert!(
        String::from_utf8(output.stderr)
            .expect("utf8 stderr")
            .contains("unexpected argument '--distro'")
    );
}

#[test]
fn unsupported_leading_globals_cannot_bypass_product_roots() {
    let fixture = Fixture::new("leading-global");
    fixture.install_runtime(RECORDING_RUNTIME);

    for args in [
        vec!["--format", "json", "init"],
        vec!["-p", "prompt text", "init"],
        vec!["--principal", "alice", "update"],
    ] {
        let output = fixture
            .command()
            .args(args)
            .output()
            .expect("run protected product root");

        assert_eq!(output.status.code(), Some(2));
        assert!(!fixture.args.exists());
    }
}

#[test]
fn explicit_principal_is_accepted_in_either_product_status_position() {
    let fixture = Fixture::new("principal-status");
    fixture.install_runtime(RECORDING_RUNTIME);

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
    assert!(
        !fixture.args.exists(),
        "product status must not delegate to the runtime binary"
    );

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
fn product_distro_apply_rejects_arbitrary_paths_and_unsigned_bypasses() {
    let fixture = Fixture::new("owned-distro");
    fixture.install_runtime(RECORDING_RUNTIME);

    for args in [
        vec!["distro", "apply", "https://example.invalid/other.toml"],
        vec!["--principal", "operator", "distro", "apply", "other"],
        vec![
            "distro",
            "apply",
            "--principal",
            "operator",
            "--yes",
            "--accept-new-key",
        ],
        vec![
            "distro",
            "apply",
            "--principal",
            "operator",
            "--yes",
            "--allow-unsigned",
        ],
    ] {
        let output = fixture
            .command()
            .args(args)
            .output()
            .expect("run protected distro command");

        assert_eq!(output.status.code(), Some(2));
        assert!(!fixture.args.exists());
        let stderr = String::from_utf8(output.stderr).expect("utf8 stderr");
        assert!(stderr.contains("unexpected argument"), "stderr: {stderr}");
        assert!(!stderr.contains("astrid distro"));
    }
}

#[test]
fn product_distro_apply_requires_an_explicit_principal_before_dispatch() {
    let fixture = Fixture::new("distro-principal-required");
    fixture.install_runtime(RECORDING_RUNTIME);
    fixture.install_signed_distro();

    let output = fixture
        .command()
        .args(["distro", "apply", "--yes"])
        .output()
        .expect("run distro apply without a principal");

    assert_eq!(output.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&output.stderr).contains("requires an explicit"));
    assert!(!fixture.root.join("start-args").exists());
    assert!(!fixture.root.join("apply-args").exists());
}

#[test]
fn signed_distro_apply_stops_to_a_volume_and_writes_a_bound_receipt() {
    let fixture = Fixture::new("distro-apply-success");
    fixture.install_runtime(RECORDING_RUNTIME);
    fixture.install_signed_distro();

    let output = fixture
        .command()
        .args([
            "distro",
            "apply",
            "--principal",
            "operator",
            "--yes",
            "--offline",
            "--var",
            "model=fixture",
        ])
        .output()
        .expect("run signed distro apply");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("start-args")).expect("read start args"),
        "<start>\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("apply-args")).expect("read apply args"),
        format!(
            "<--principal>\n<operator>\n<distro>\n<apply>\n<--yes>\n<{}>\n<--offline>\n<--var>\n<model=fixture>\n",
            fixture.release_dir().join("Distro.toml").display()
        )
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("apply-distro")).expect("read selected distro"),
        format!("{}\n", fixture.release_dir().join("Distro.toml").display())
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("stop-args")).expect("read stop args"),
        "<stop>\n"
    );

    let runtime = fixture.home.join("runtime");
    let entries: Vec<_> = fs::read_dir(&runtime)
        .expect("read stopped runtime")
        .map(|entry| {
            entry
                .expect("read stopped runtime entry")
                .file_name()
                .to_string_lossy()
                .into_owned()
        })
        .collect();
    assert_eq!(entries, vec!["astrid.volume"]);
    let volume = fs::symlink_metadata(runtime.join("astrid.volume")).expect("inspect volume");
    assert!(volume.is_file());
    assert!(!volume.file_type().is_symlink());
    assert!(volume.len() > 0);
    assert_eq!(volume.permissions().mode() & 0o7777, 0o600);
    assert!(!runtime.join("trust").exists());
    assert!(!runtime.join("run").exists());

    let receipt_path = fixture.home.join("receipts").join("unicity-ce.active.json");
    let receipt: serde_json::Value =
        serde_json::from_slice(&fs::read(&receipt_path).expect("read distro apply receipt"))
            .expect("parse distro apply receipt");
    assert_eq!(receipt["distro_id"], "unicity-ce");
    assert_eq!(receipt["distro_version"], env!("CARGO_PKG_VERSION"));
    assert_eq!(receipt["principal"], "operator");
    assert_eq!(receipt["astrid_runtime_version"], "0.10.4");
    assert!(receipt["signing_pubkey"].as_str().unwrap().len() > 8);
    assert!(
        receipt["manifest_blake3"]
            .as_str()
            .unwrap()
            .starts_with("blake3:")
    );
    let receipt_metadata = fs::symlink_metadata(&receipt_path).expect("inspect receipt");
    assert_eq!(receipt_metadata.permissions().mode() & 0o7777, 0o600);
}

#[test]
fn signed_distro_apply_requires_the_bundled_signature_before_start() {
    let fixture = Fixture::new("distro-apply-missing-signature");
    fixture.install_runtime(RECORDING_RUNTIME);
    fixture.install_signed_distro();
    fs::remove_file(fixture.release_dir().join("Distro.sig")).expect("remove fixture signature");

    let output = fixture
        .command()
        .args(["distro", "apply", "--principal", "operator", "--yes"])
        .output()
        .expect("run unsigned distro apply");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("Distro.sig"));
    assert!(!fixture.root.join("start-args").exists());
    assert!(!fixture.root.join("apply-args").exists());
}

#[test]
fn failed_distro_apply_still_stops_and_does_not_write_a_receipt() {
    let fixture = Fixture::new("distro-apply-failure");
    fixture.install_runtime(RECORDING_RUNTIME);
    fixture.install_signed_distro();

    let output = fixture
        .command()
        .env("AOS_TEST_APPLY_EXIT", "23")
        .args(["distro", "apply", "--principal", "operator", "--yes"])
        .output()
        .expect("run failing distro apply");
    assert_eq!(output.status.code(), Some(23));
    assert!(fixture.root.join("start-args").exists());
    assert!(fixture.root.join("apply-args").exists());
    assert!(fixture.root.join("stop-args").exists());
    assert!(
        !fixture
            .home
            .join("receipts")
            .join("unicity-ce.active.json")
            .exists()
    );
    assert!(!fixture.home.join("runtime/trust").exists());
}

#[test]
fn successful_distro_apply_refuses_a_leftover_runtime_trust_projection() {
    let fixture = Fixture::new("distro-apply-leftover-trust");
    fixture.install_runtime(RECORDING_RUNTIME);
    fixture.install_signed_distro();

    let output = fixture
        .command()
        .env("AOS_TEST_KEEP_TRUST", "1")
        .args(["distro", "apply", "--principal", "operator", "--yes"])
        .output()
        .expect("run distro apply with leftover trust");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("shutdown"));
    assert!(fixture.home.join("runtime/trust/unicity-ce.pub").exists());
    assert!(
        !fixture
            .home
            .join("receipts")
            .join("unicity-ce.active.json")
            .exists()
    );
}

#[test]
fn signed_distro_apply_refuses_a_manifest_key_swap_before_start() {
    let fixture = Fixture::new("distro-apply-key-swap");
    fixture.install_runtime(RECORDING_RUNTIME);
    fixture.install_signed_distro();
    let manifest_path = fixture.release_dir().join("Distro.toml");
    let manifest = fs::read_to_string(&manifest_path)
        .expect("read fixture manifest")
        .lines()
        .map(|line| {
            if line.trim_start().starts_with("pubkey =") {
                "pubkey = \"ed25519:AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=\""
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    fs::write(manifest_path, manifest).expect("swap fixture signing key");

    let output = fixture
        .command()
        .args(["distro", "apply", "--principal", "operator", "--yes"])
        .output()
        .expect("run key-swapped distro apply");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("manifest-hash"));
    assert!(!fixture.root.join("start-args").exists());
    assert!(!fixture.root.join("apply-args").exists());
}

#[test]
fn signed_distro_apply_refuses_a_receipt_principal_mismatch_before_dispatch() {
    let fixture = Fixture::new("distro-apply-receipt-mismatch");
    fixture.install_runtime(RECORDING_RUNTIME);
    fixture.install_signed_distro();

    let first = fixture
        .command()
        .args(["distro", "apply", "--principal", "operator", "--yes"])
        .status()
        .expect("run initial signed distro apply");
    assert!(first.success());
    fs::remove_file(fixture.root.join("start-args")).expect("reset start marker");
    fs::remove_file(fixture.root.join("apply-args")).expect("reset apply marker");

    let output = fixture
        .command()
        .args(["distro", "apply", "--principal", "other", "--yes"])
        .output()
        .expect("run mismatched repeat distro apply");
    assert!(!output.status.success());
    assert!(String::from_utf8_lossy(&output.stderr).contains("before dispatch"));
    assert!(!fixture.root.join("start-args").exists());
    assert!(!fixture.root.join("apply-args").exists());
}

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

#[test]
fn native_status_reports_stopped_without_invoking_the_runtime_cli() {
    let fixture = Fixture::new("status");
    fixture.install_runtime(RECORDING_RUNTIME);

    for args in [vec!["status"], vec!["status", "--json"]] {
        let output = fixture
            .command()
            .args(args)
            .output()
            .expect("run aos status");

        assert!(output.status.success());
        assert!(!fixture.args.exists());
        assert!(output.stderr.is_empty());
        let stdout = String::from_utf8(output.stdout).expect("utf8 stdout");
        assert!(stdout.contains("stopped"));
        assert!(stdout.contains("0.10.4"));
    }
}

#[test]
fn unix_passthrough_preserves_signal_termination() {
    let fixture = Fixture::new("signal");
    let ready = fixture.root.join("ready");
    fixture.install_runtime(&format!(
        "#!/bin/sh\nprintf '%s\\n' \"$$\" > '{}'\nexec sleep 30\n",
        shell_literal_path(&ready)
    ));

    let mut child = fixture
        .command()
        .arg("wait")
        .spawn()
        .expect("spawn inherited command");
    for _ in 0..1_000 {
        if ready.exists() {
            break;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(ready.exists(), "runtime must replace the aos process");
    assert_eq!(
        fs::read_to_string(&ready)
            .expect("read runtime pid")
            .trim()
            .parse::<u32>()
            .expect("parse runtime pid"),
        child.id(),
        "the runtime script must retain the aos process id"
    );

    child.kill().expect("terminate delegated runtime");
    let status = child.wait().expect("wait for delegated runtime");
    assert_eq!(status.signal(), Some(9));
}

fn shell_literal_path(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "'\\''")
}
