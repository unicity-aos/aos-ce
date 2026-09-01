#![cfg(unix)]

use std::fs;
use std::io::Write as _;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

struct Fixture {
    root: PathBuf,
    runtime: PathBuf,
    args: PathBuf,
    home: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Self {
        let root = std::env::temp_dir().join(format!(
            "aos-mcp-serve-{name}-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ));
        fs::create_dir_all(&root).expect("create fixture");
        let fixture = Self {
            root: root.clone(),
            runtime: root.join("fake-runtime"),
            args: root.join("runtime-args"),
            home: root.join("runtime-home"),
        };
        fs::create_dir_all(&fixture.home).expect("create runtime home");
        fixture
    }

    fn install_runtime(&self, body: &str) {
        fs::write(&self.runtime, body).expect("write fake runtime");
        let mut permissions = fs::metadata(&self.runtime)
            .expect("runtime metadata")
            .permissions();
        permissions.set_mode(0o700);
        fs::set_permissions(&self.runtime, permissions).expect("make runtime executable");
    }

    fn command(&self) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_aos"));
        command
            .env("AOS_HOME", &self.root)
            .env("UNICITY_AOS_RUNTIME_BIN", &self.runtime)
            .env("AOS_TEST_ARGS", &self.args)
            .env("AOS_TEST_HOME", self.root.join("home-marker"))
            .env("AOS_TEST_WORKSPACE", self.root.join("workspace-marker"))
            .env("AOS_TEST_PWD", self.root.join("pwd-marker"));
        command
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

const RECORDING_RUNTIME: &str = r#"#!/bin/sh
for arg in "$@"; do
    printf '<%s>\n' "$arg"
done > "$AOS_TEST_ARGS"
printf '%s\n' "$ASTRID_HOME" > "$AOS_TEST_HOME"
printf '%s\n' "$ASTRID_WORKSPACE_STATE_DIR" > "$AOS_TEST_WORKSPACE"
printf '%s\n' "$PWD" > "$AOS_TEST_PWD"
exit "${AOS_TEST_EXIT:-0}"
"#;

#[test]
fn serve_forwards_host_arguments_exactly_and_separates_home_from_workspace() {
    let fixture = Fixture::new("argv");
    fixture.install_runtime(RECORDING_RUNTIME);
    let workspace = fixture.root.join("project wörkspace ✨");
    fs::create_dir_all(&workspace).expect("create Unicode workspace");

    let output = fixture
        .command()
        .args([
            "--principal",
            "grok-code",
            "mcp",
            "serve",
            "--interaction",
            "native",
            "--workspace",
            workspace.to_str().expect("Unicode path"),
            "--request-timeout",
            "1d5m",
        ])
        .output()
        .expect("run MCP bridge");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let expected = format!(
        "<--principal>\n<grok-code>\n<mcp>\n<serve>\n<--workspace>\n<{}>\n<--request-timeout>\n<1d5m>\n",
        workspace.display()
    );
    assert_eq!(
        fs::read_to_string(&fixture.args).expect("read runtime argv"),
        expected
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("home-marker")).expect("read runtime home"),
        format!("{}\n", fixture.root.join("runtime").display())
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("workspace-marker")).expect("read state dir"),
        ".aos\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("pwd-marker")).expect("read runtime cwd"),
        format!("{}\n", std::env::current_dir().expect("test cwd").display())
    );
}

#[test]
fn serve_rejects_flags_without_values_before_starting_the_runtime() {
    let fixture = Fixture::new("invalid-argv");
    fixture.install_runtime(RECORDING_RUNTIME);

    for arguments in [
        vec!["mcp", "serve", "--workspace"],
        vec!["mcp", "serve", "--request-timeout"],
    ] {
        let output = fixture
            .command()
            .args(&arguments)
            .output()
            .expect("run invalid MCP bridge");
        assert_eq!(output.status.code(), Some(2));
        assert!(
            !fixture.args.exists(),
            "invalid argv must not reach runtime"
        );
    }
}

#[test]
fn serve_preserves_raw_frames_and_keeps_stderr_isolated() {
    let fixture = Fixture::new("bytes");
    fixture.install_runtime(
        r#"#!/bin/sh
cat > "$AOS_TEST_PAYLOAD"
cat "$AOS_TEST_PAYLOAD"
echo 'runtime diagnostics only' >&2
"#,
    );
    let payload_path = fixture.root.join("payload");
    let payload = b"\xff non-JSON frame\n   {\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}   \n";

    let mut child = fixture
        .command()
        .env("AOS_TEST_PAYLOAD", &payload_path)
        .args(["mcp", "serve"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start MCP bridge");
    child
        .stdin
        .take()
        .expect("bridge stdin")
        .write_all(payload)
        .expect("write raw frames");
    let output = child.wait_with_output().expect("relay raw frames");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, payload);
    assert_eq!(
        String::from_utf8_lossy(&output.stderr),
        "runtime diagnostics only\n"
    );
    assert_eq!(
        fs::read(&payload_path).expect("read runtime input"),
        payload
    );
}

#[test]
fn serve_preserves_child_numeric_and_signal_termination() {
    let fixture = Fixture::new("child-exit");
    fixture.install_runtime(
        r#"#!/bin/sh
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}'
exit "${AOS_TEST_EXIT:-0}"
"#,
    );

    let output = fixture
        .command()
        .env("AOS_TEST_EXIT", "23")
        .args(["mcp", "serve"])
        .output()
        .expect("run bridge with failing runtime");
    assert_eq!(output.status.code(), Some(23));
    assert!(String::from_utf8_lossy(&output.stderr).contains("exited with"));

    let signal_fixture = Fixture::new("child-signal");
    signal_fixture.install_runtime(
        r#"#!/bin/sh
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}'
kill -TERM "$$"
"#,
    );
    let output = signal_fixture
        .command()
        .args(["mcp", "serve"])
        .output()
        .expect("run bridge with signalled runtime");
    assert_eq!(output.status.code(), Some(143));
    assert!(String::from_utf8_lossy(&output.stderr).contains("exited with"));
}

#[test]
fn parent_termination_kills_and_reaps_the_runtime_child() {
    let fixture = Fixture::new("parent-interrupt");
    let pid_path = fixture.root.join("runtime-pid");
    fixture.install_runtime(&format!(
        "#!/bin/sh\nprintf '%s\\n' \"$$\" > '{}'\nexec sleep 30\n",
        shell_literal_path(&pid_path)
    ));

    let bridge = fixture
        .command()
        .args(["mcp", "serve"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start MCP bridge");
    let runtime_pid = wait_for_runtime_pid(&pid_path);
    let bridge_pid = bridge.id().to_string();

    let termination = Command::new("kill")
        .args(["-TERM", &bridge_pid])
        .status()
        .expect("signal bridge process");
    assert!(termination.success(), "send SIGTERM to bridge");
    let output = bridge
        .wait_with_output()
        .expect("wait for interrupted bridge");
    assert_eq!(output.status.code(), Some(143));
    assert!(
        !process_exists(&runtime_pid),
        "runtime child must not orphan"
    );
}

fn wait_for_runtime_pid(path: &Path) -> String {
    for _ in 0..1_000 {
        if let Ok(pid) = fs::read_to_string(path) {
            let pid = pid.trim();
            if !pid.is_empty() {
                return pid.to_owned();
            }
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("runtime child did not publish its pid");
}

fn process_exists(pid: &str) -> bool {
    Command::new("kill")
        .args(["-0", pid])
        .output()
        .expect("probe runtime child")
        .status
        .success()
}

fn shell_literal_path(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "'\\''")
}
