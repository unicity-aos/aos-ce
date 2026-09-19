#![cfg(unix)]

use std::fs;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

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
            // Do not launch the developer's installed menu-bar app from a
            // protocol fixture. This child does not run Cargo or rustup.
            .env("HOME", &self.root)
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
fn socket_approval_returns_pending_and_resumes_once_after_human_decision() {
    use serde_json::{Value, json};
    use std::os::unix::net::UnixListener;
    let fixture = Fixture::new("pending-native-approval");
    fixture.install_runtime(r#"#!/usr/bin/env python3
import sys, json, os
for line in sys.stdin:
    request = json.loads(line)
    if request.get('method') != 'tools/call':
        continue
    with open(os.environ['AOS_TEST_ARGS'], 'a') as log:
        log.write(json.dumps(request) + '\n')
    if 'requestState' in request['params']:
        assert request['params']['requestState'] == 'test-state'
        assert request['params']['inputResponses']['consent']['content']['allow'] is True
        result = {'content':[{'type':'text','text':'original-operation-complete'}], 'isError':False}
    else:
        result = {'resultType':'input_required','requestState':'test-state','inputRequests':{'consent':{'method':'elicitation/create','params':{'message':'Allow operation?', 'requestedSchema':{'type':'object','properties':{'allow':{'type':'boolean'}},'required':['allow']}}}}}
    print(json.dumps({'jsonrpc':'2.0','id':request['id'],'result':result}), flush=True)
"#);
    // Darwin's Unix socket path limit is shorter than its default TMPDIR.
    let private = PathBuf::from("/tmp").join(format!(
        "aos-approval-{}-{}",
        std::process::id(),
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    ));
    struct SocketDirectory(PathBuf);
    impl Drop for SocketDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _socket_directory = SocketDirectory(private.clone());
    fs::create_dir(&private).unwrap();
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700)).unwrap();
    let socket = fs::canonicalize(private).unwrap().join("approval.sock");
    let listener = UnixListener::bind(&socket).unwrap();
    listener.set_nonblocking(true).unwrap();
    fs::set_permissions(&socket, fs::Permissions::from_mode(0o600)).unwrap();
    let (human_tx, human_rx) = mpsc::channel();
    let ui = std::thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut stream = loop {
            if let Ok((stream, _)) = listener.accept() {
                break stream;
            }
            assert!(Instant::now() < deadline, "presenter did not connect");
            std::thread::sleep(Duration::from_millis(5));
        };
        stream
            .set_read_timeout(Some(Duration::from_secs(5)))
            .unwrap();
        let mut line = String::new();
        BufReader::new(stream.try_clone().unwrap())
            .read_line(&mut line)
            .unwrap();
        let request: Value = serde_json::from_str(&line).unwrap();
        human_rx.recv_timeout(Duration::from_secs(5)).unwrap();
        writeln!(
            stream,
            "{}",
            json!({"version":1,"id":request["id"],"selected":0})
        )
        .unwrap();
    });
    let mut child = fixture
        .command()
        .args([
            "mcp",
            "serve",
            "--interaction",
            "native",
            "--interaction-socket",
            socket.to_str().unwrap(),
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::inherit())
        .spawn()
        .unwrap();
    let mut input = child.stdin.take().unwrap();
    let output = child.stdout.take().unwrap();
    let (tx, rx) = mpsc::channel();
    let reader = std::thread::spawn(move || {
        for line in BufReader::new(output).lines() {
            if tx.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    writeln!(input, "{}", json!({"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"fs.write","arguments":{"path":"original"}}})).unwrap();
    let first: Value =
        serde_json::from_str(&rx.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
    assert_eq!(first["id"], 7);
    let pending = &first["result"]["structuredContent"];
    assert_eq!(pending["status"], "awaiting_approval");
    assert_eq!(pending["tool"], "fs.write");
    let instructions = pending["message"].as_str().unwrap();
    assert!(instructions.contains("existing SSH connection first"));
    assert!(
        !instructions.contains("aos console"),
        "custom socket must not route to another surface"
    );
    let status_call = json!({"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"aos_approval_status","arguments":{"approval_id":pending["approval_id"]}}});
    writeln!(input, "{status_call}").unwrap();
    let before: Value =
        serde_json::from_str(&rx.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
    assert_eq!(
        before["result"]["structuredContent"]["status"],
        "awaiting_approval"
    );
    human_tx.send(()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        writeln!(input, "{status_call}").unwrap();
        let response: Value =
            serde_json::from_str(&rx.recv_timeout(Duration::from_secs(5)).unwrap()).unwrap();
        if response["result"]["structuredContent"]["status"] == "completed" {
            assert_eq!(
                response["result"]["structuredContent"]["outcome"]["result"]["content"][0]["text"],
                "original-operation-complete"
            );
            break;
        }
        assert!(Instant::now() < deadline);
        std::thread::sleep(Duration::from_millis(20));
    }
    drop(input);
    assert!(wait_for_child(&mut child, Duration::from_secs(5)).success());
    reader.join().unwrap();
    ui.join().unwrap();
    let calls = fs::read_to_string(&fixture.args).unwrap();
    assert_eq!(
        calls.lines().count(),
        2,
        "one original invocation and exactly one continuation; status never forwards"
    );
}

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
        vec!["mcp", "serve", "--interaction-socket"],
        vec!["mcp", "serve", "--interaction-timeout"],
        vec!["mcp", "serve", "--max-in-flight-calls"],
        vec!["mcp", "serve", "--max-input-rounds"],
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
fn serve_rejects_out_of_range_native_adapter_limits_before_starting_the_runtime() {
    let fixture = Fixture::new("invalid-limits");
    fixture.install_runtime(RECORDING_RUNTIME);

    for arguments in [
        vec!["mcp", "serve", "--interaction-timeout", "0"],
        vec!["mcp", "serve", "--interaction-timeout", "301"],
        vec!["mcp", "serve", "--max-in-flight-calls", "0"],
        vec!["mcp", "serve", "--max-in-flight-calls", "1025"],
        vec!["mcp", "serve", "--max-input-rounds", "0"],
        vec!["mcp", "serve", "--max-input-rounds", "65"],
    ] {
        let output = fixture
            .command()
            .args(&arguments)
            .output()
            .expect("run invalid MCP bridge limits");
        assert_eq!(output.status.code(), Some(2), "{arguments:?}");
        assert!(
            !fixture.args.exists(),
            "invalid adapter limits must not reach runtime: {arguments:?}"
        );
    }
}

#[test]
fn serve_rejects_interaction_socket_without_native_mode() {
    let fixture = Fixture::new("socket-mode");
    fixture.install_runtime(RECORDING_RUNTIME);
    let output = fixture
        .command()
        .args([
            "mcp",
            "serve",
            "--interaction",
            "client",
            "--interaction-socket",
            "/tmp/aos-tray.sock",
        ])
        .output()
        .expect("run MCP bridge with invalid socket mode");
    assert!(!output.status.success(), "bridge exited {}", output.status);
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(
        stderr.contains("--interaction native"),
        "stderr must explain the native-only socket flag, got {stderr:?}"
    );
    assert!(
        !fixture.args.exists(),
        "invalid interaction-socket must not start the runtime"
    );
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
fn serve_preserves_unchanged_initialize_request_bytes() {
    let fixture = Fixture::new("unchanged-initialize-request");
    fixture.install_runtime(
        r#"#!/bin/sh
cat > "$AOS_TEST_PAYLOAD"
cat "$AOS_TEST_PAYLOAD"
"#,
    );
    let payload_path = fixture.root.join("payload");
    let payload = b"  {\"method\":\"initialize\",\"params\":{\"clientInfo\":{\"version\":\"1\",\"name\":\"test\"},\"capabilities\":{\"elicitation\":{\"form\":{}}},\"protocolVersion\":\"2025-11-25\"},\"id\":1,\"jsonrpc\":\"2.0\"}  \n";

    let mut child = fixture
        .command()
        .env("AOS_TEST_PAYLOAD", &payload_path)
        .args(["mcp", "serve", "--interaction", "client"])
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
        .expect("write initialize frame");
    let output = child.wait_with_output().expect("relay initialize frame");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, payload);
    assert_eq!(
        fs::read(&payload_path).expect("read runtime input"),
        payload
    );
}

#[test]
fn serve_preserves_unchanged_initialize_response_bytes() {
    let fixture = Fixture::new("unchanged-initialize-response");
    fixture.install_runtime(
        r#"#!/bin/sh
IFS= read -r line || exit 91
printf '%s\n' '  {"result":{"capabilities":{"roots":{}},"protocolVersion":"2025-11-25"},"id":1,"jsonrpc":"2.0"}  '
"#,
    );
    let request = br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"test","version":"1"}}}
"#;
    let expected_response = b"  {\"result\":{\"capabilities\":{\"roots\":{}},\"protocolVersion\":\"2025-11-25\"},\"id\":1,\"jsonrpc\":\"2.0\"}  \n";

    let mut child = fixture
        .command()
        .args(["mcp", "serve", "--interaction", "client"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start MCP bridge");
    child
        .stdin
        .take()
        .expect("bridge stdin")
        .write_all(request)
        .expect("write initialize request");
    let output = child.wait_with_output().expect("relay initialize response");

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(output.stdout, expected_response);
}

#[test]
fn serve_newline_frames_transformed_initialize_for_line_reading_runtime() {
    let fixture = Fixture::new("transformed-newline");
    fixture.install_runtime(
        r#"#!/bin/sh
IFS= read -r line || exit 91
printf '%s\n' "$line" > "$AOS_TEST_FRAME"
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{}}'
"#,
    );
    let frame_path = fixture.root.join("transformed-frame");
    let mut child = fixture
        .command()
        .env("AOS_TEST_FRAME", &frame_path)
        .args(["mcp", "serve"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start MCP bridge");
    let mut stdin = child.stdin.take().expect("bridge stdin");
    let stdout = child.stdout.take().expect("bridge stdout");
    let (sender, receiver) = mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(stdout)
            .read_line(&mut line)
            .map(|bytes| (bytes, line));
        let _ = sender.send(result);
    });

    stdin
        .write_all(
            br#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-11-25","capabilities":{"roots":{}},"clientInfo":{"name":"test","version":"1"}}}
"#,
        )
        .expect("write initialize frame");
    let response = match receiver.recv_timeout(Duration::from_secs(5)) {
        Ok(Ok((bytes, line))) => {
            assert!(bytes > 0, "line-reading runtime returned an empty frame");
            line
        }
        Ok(Err(error)) => panic!("read transformed response: {error}"),
        Err(mpsc::RecvTimeoutError::Timeout) => {
            drop(stdin);
            let _ = wait_for_child(&mut child, Duration::from_secs(5));
            panic!("line-reading runtime did not receive transformed initialize frame");
        }
        Err(mpsc::RecvTimeoutError::Disconnected) => {
            drop(stdin);
            let _ = wait_for_child(&mut child, Duration::from_secs(5));
            panic!("line-reading runtime probe disconnected before receiving a frame");
        }
    };
    drop(stdin);
    let status = wait_for_child(&mut child, Duration::from_secs(5));
    assert!(
        status.success(),
        "bridge failed after forwarding transformed initialize: {status}"
    );
    let transformed = fs::read_to_string(&frame_path).expect("read transformed frame");
    let transformed: serde_json::Value =
        serde_json::from_str(transformed.trim()).expect("transformed initialize JSON");
    assert_eq!(transformed["method"], "initialize");
    assert!(
        transformed
            .pointer("/params/capabilities/elicitation/form")
            .is_some(),
        "initialize must be transformed before forwarding"
    );
    assert_eq!(response, "{\"jsonrpc\":\"2.0\",\"id\":1,\"result\":{}}\n");
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

fn wait_for_child(child: &mut Child, timeout: Duration) -> ExitStatus {
    let deadline = Instant::now() + timeout;
    loop {
        match child.try_wait().expect("probe bridge process") {
            Some(status) => return status,
            None if Instant::now() >= deadline => {
                child.kill().expect("terminate stalled bridge process");
                let _ = child.wait();
                panic!("bridge process did not exit within {timeout:?}");
            }
            None => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}

fn shell_literal_path(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "'\\''")
}

#[test]
fn deny_interaction_replies_to_runtime_not_mcp_host() {
    assert_local_interaction_returns_to_runtime("deny");
}

#[test]
fn unsupported_native_interaction_cancels_to_runtime_not_mcp_host() {
    // Free-form secrets are refused before any platform UI is opened.
    assert_local_interaction_returns_to_runtime("native");
}

fn assert_local_interaction_returns_to_runtime(mode: &str) {
    let fixture = Fixture::new(mode);
    fixture.install_runtime(
        r#"#!/bin/sh
IFS= read -r request || exit 90
printf '%s\n' '{"jsonrpc":"2.0","id":"native-decision","method":"elicitation/create","params":{"mode":"form","message":"Test only","requestedSchema":{"type":"object","properties":{"secret":{"type":"string","format":"password"}}}}}'
IFS= read -r answer || exit 91
printf '%s\n' "$answer" > "$AOS_TEST_ARGS"
printf '%s\n' '{"jsonrpc":"2.0","id":1,"result":{"receivedDecision":true}}'
"#,
    );
    let mut child = fixture
        .command()
        .args(["mcp", "serve", "--interaction", mode])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("start isolated bridge");
    let mut input = child.stdin.take().expect("bridge input");
    let output = child.stdout.take().expect("bridge output");
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(output).read_line(&mut line).map(|_| line);
        let _ = sender.send(result);
    });
    input
        .write_all(
            br#"{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"fs.read","arguments":{"path":"/tmp/report"}}}"#,
        )
        .expect("send tool request");
    input.write_all(b"\n").expect("newline");
    let observed = receiver.recv_timeout(Duration::from_secs(5));
    drop(input);
    let status = wait_for_child(&mut child, Duration::from_secs(5));
    reader.join().expect("reader completed");
    let line = observed.expect("bounded reply").expect("read reply");
    let host_reply: serde_json::Value = serde_json::from_str(&line).expect("host JSON");
    assert_eq!(
        host_reply,
        serde_json::json!({"jsonrpc":"2.0", "id":1, "result":{"receivedDecision":true}}),
        "the host must receive the runtime's tool result, never the local consent reply"
    );
    assert!(status.success(), "bridge exited {status}");
    let runtime_reply: serde_json::Value =
        serde_json::from_slice(&fs::read(&fixture.args).expect("runtime received answer"))
            .expect("runtime JSON");
    assert_eq!(
        runtime_reply,
        serde_json::json!({"jsonrpc":"2.0", "id":"native-decision", "result":{"action":"cancel"}})
    );
}

#[test]
fn deny_mrtr_resumes_original_call_to_runtime_not_mcp_host() {
    assert_mrtr_resume_returns_to_runtime(
        "deny",
        r#"{"type":"object","properties":{"grant":{"type":"boolean"}},"required":["grant"]}"#,
        false,
    );
}

#[test]
fn unsupported_native_mrtr_cancels_to_runtime_not_mcp_host() {
    assert_mrtr_resume_returns_to_runtime(
        "native",
        r#"{"type":"object","properties":{"secret":{"type":"string","format":"password"}}}"#,
        true,
    );
}

#[test]
fn client_mode_forwards_input_required_to_host() {
    let fixture = Fixture::new("client-mrtr-bytepass");
    fixture.install_runtime(
        r#"#!/bin/sh
IFS= read -r request || exit 90
printf '%s\n' "$request" > "$AOS_TEST_ARGS"
printf '%s\n' '{"jsonrpc":"2.0","id":7,"result":{"resultType":"input_required","requestState":"opaque-token","inputRequests":{"astrid-consent":{"method":"elicitation/create","params":{"mode":"form","message":"Allow this capsule to continue?","requestedSchema":{"type":"object","properties":{"grant":{"type":"boolean"}},"required":["grant"]}}}}}}'
"#,
    );
    let mut child = fixture
        .command()
        .args(["mcp", "serve", "--interaction", "client"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("start isolated bridge");
    let mut input = child.stdin.take().expect("bridge input");
    let output = child.stdout.take().expect("bridge output");
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(output).read_line(&mut line).map(|_| line);
        let _ = sender.send(result);
    });
    input
        .write_all(
            br#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"fs.read","arguments":{"path":"/tmp/report"},"_meta":{"io.modelcontextprotocol/protocolVersion":"2025-11-25","io.modelcontextprotocol/clientCapabilities":{"elicitation":{}}}}}"#,
        )
        .expect("send tool request");
    input.write_all(b"\n").expect("newline");
    let observed = receiver.recv_timeout(Duration::from_secs(5));
    drop(input);
    let status = wait_for_child(&mut child, Duration::from_secs(5));
    reader.join().expect("reader completed");
    let line = observed.expect("bounded reply").expect("read reply");
    let host_reply: serde_json::Value = serde_json::from_str(&line).expect("host JSON");
    assert_eq!(
        host_reply["result"]["resultType"], "input_required",
        "client mode must bytepass input_required to the host"
    );
    assert_eq!(host_reply["result"]["requestState"], "opaque-token");
    assert!(status.success(), "bridge exited {status}");
    let forwarded = serde_json::from_slice::<serde_json::Value>(
        &fs::read(&fixture.args).expect("runtime received host tools/call"),
    )
    .expect("forwarded JSON");
    assert_eq!(forwarded["method"], "tools/call");
    assert_eq!(forwarded["params"]["name"], "fs.read");
    assert!(
        forwarded["params"].get("inputResponses").is_none(),
        "client mode must not resume input_required to the runtime"
    );
}

fn assert_mrtr_resume_returns_to_runtime(mode: &str, schema: &str, advertise_form: bool) {
    let fixture = Fixture::new(&format!("mrtr-{mode}"));
    let schema_literal = schema.replace('\'', r"'\''");
    fixture.install_runtime(&format!(
        r#"#!/bin/sh
IFS= read -r request || exit 90
printf '%s\n' '{{"jsonrpc":"2.0","id":7,"result":{{"resultType":"input_required","requestState":"opaque-token","inputRequests":{{"astrid-consent":{{"method":"elicitation/create","params":{{"mode":"form","message":"Allow this capsule to continue?","requestedSchema":{schema}}}}}}}}}}}'
IFS= read -r resume || exit 91
printf '%s\n' "$resume" > "$AOS_TEST_ARGS"
printf '%s\n' '{{"jsonrpc":"2.0","id":7,"result":{{"content":[{{"type":"text","text":"done"}}]}}}}'
"#,
        schema = schema_literal
    ));
    let mut child = fixture
        .command()
        .args(["mcp", "serve", "--interaction", mode])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("start isolated bridge");
    let mut input = child.stdin.take().expect("bridge input");
    let output = child.stdout.take().expect("bridge output");
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(output).read_line(&mut line).map(|_| line);
        let _ = sender.send(result);
    });
    input
        .write_all(
            br#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"fs.read","arguments":{"path":"/tmp/report"},"extra":"keep-me","_meta":{"io.modelcontextprotocol/protocolVersion":"2025-11-25","io.modelcontextprotocol/clientCapabilities":{"elicitation":{}}}}}"#,
        )
        .expect("send tool request");
    input.write_all(b"\n").expect("newline");
    let observed = receiver.recv_timeout(Duration::from_secs(5));
    drop(input);
    let status = wait_for_child(&mut child, Duration::from_secs(5));
    reader.join().expect("reader completed");
    let line = observed.expect("bounded reply").expect("read reply");
    let host_reply: serde_json::Value = serde_json::from_str(&line).expect("host JSON");
    assert_eq!(
        host_reply,
        serde_json::json!({"jsonrpc":"2.0","id":7,"result":{"content":[{"type":"text","text":"done"}]}}),
        "the host must receive the runtime's final tool result, never input_required"
    );
    assert!(status.success(), "bridge exited {status}");
    let resume: serde_json::Value =
        serde_json::from_slice(&fs::read(&fixture.args).expect("runtime received resume"))
            .expect("resume JSON");
    assert_eq!(resume["method"], "tools/call");
    assert_eq!(resume["id"], 7);
    assert_eq!(resume["params"]["name"], "fs.read");
    assert_eq!(resume["params"]["arguments"]["path"], "/tmp/report");
    assert_eq!(resume["params"]["extra"], "keep-me");
    assert_eq!(resume["params"]["requestState"], "opaque-token");
    assert_eq!(
        resume["params"]["inputResponses"]["astrid-consent"],
        serde_json::json!({ "action": "cancel" })
    );
    assert_eq!(
        resume["params"]["_meta"]["io.modelcontextprotocol/protocolVersion"],
        "2025-11-25"
    );
    let form =
        resume["params"]["_meta"]["io.modelcontextprotocol/clientCapabilities"]["elicitation"]
            .get("form");
    if advertise_form {
        assert!(
            form.is_some_and(serde_json::Value::is_object),
            "native must advertise per-request form"
        );
    } else {
        assert!(form.is_none(), "deny must not invent per-request form");
    }
}

#[test]
fn duplicate_tools_call_id_fails_closed_without_reaching_runtime() {
    let fixture = Fixture::new("mrtr-duplicate-id");
    let second_path = fixture.root.join("second-call");
    fixture.install_runtime(
        r#"#!/bin/sh
IFS= read -r request || exit 90
printf '%s\n' "$request" > "$AOS_TEST_ARGS"
IFS= read -r second
if [ -n "$second" ]; then
  printf '%s\n' "$second" > "$AOS_TEST_SECOND"
  exit 92
fi
exit 0
"#,
    );
    let mut child = fixture
        .command()
        .env("AOS_TEST_SECOND", &second_path)
        .args(["mcp", "serve", "--interaction", "native"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("start isolated bridge");
    let mut input = child.stdin.take().expect("bridge input");
    let mut output = child.stdout.take().expect("bridge output");
    let mut stderr = child.stderr.take().expect("bridge stderr");
    input
        .write_all(
            br#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"fs.read","arguments":{"path":"/tmp/report"},"_meta":{"io.modelcontextprotocol/protocolVersion":"2025-11-25","io.modelcontextprotocol/clientCapabilities":{"elicitation":{}}}}}"#,
        )
        .expect("send first tools/call");
    input.write_all(b"\n").expect("newline");
    input.flush().expect("flush first tools/call");
    let first = wait_for_file(&fixture.args, Duration::from_secs(5));
    let first: serde_json::Value = serde_json::from_slice(&first).expect("first tools/call JSON");
    assert_eq!(first["params"]["name"], "fs.read");
    input
        .write_all(
            br#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"fs.write","arguments":{"path":"/etc/passwd"},"_meta":{"io.modelcontextprotocol/protocolVersion":"2025-11-25","io.modelcontextprotocol/clientCapabilities":{"elicitation":{}}}}}"#,
        )
        .expect("send duplicate tools/call");
    input.write_all(b"\n").expect("newline");
    input.flush().expect("flush duplicate tools/call");
    let status = wait_for_child(&mut child, Duration::from_secs(5));
    drop(input);
    let mut host_out = String::new();
    let _ = output.read_to_string(&mut host_out);
    let mut err = String::new();
    stderr.read_to_string(&mut err).expect("read bridge stderr");
    assert!(
        !status.success(),
        "duplicate id must fail the connection, got {status}"
    );
    assert!(
        err.contains("refusing to forward tools/call"),
        "bridge must fail closed with an explicit error, stderr={err:?}"
    );
    assert!(
        err.contains("already in flight"),
        "duplicate id error should name the in-flight conflict, stderr={err:?}"
    );
    assert!(
        !second_path.exists(),
        "duplicate tools/call must not reach the runtime"
    );
    assert!(
        host_out.trim().is_empty(),
        "host must not receive a resumed or rewritten second call, got {host_out:?}"
    );
}

#[test]
fn cancelled_tools_call_does_not_resume_after_forget() {
    let fixture = Fixture::new("mrtr-cancel");
    let cancel_path = fixture.root.join("cancel");
    let resume_path = fixture.root.join("resume");
    let emitted_path = fixture.root.join("emitted");
    fixture.install_runtime(
        r#"#!/bin/sh
IFS= read -r request || exit 90
printf '%s\n' "$request" > "$AOS_TEST_ARGS"
IFS= read -r cancel || exit 91
printf '%s\n' "$cancel" > "$AOS_TEST_CANCEL"
printf '%s\n' '{"jsonrpc":"2.0","id":7,"result":{"resultType":"input_required","requestState":"opaque-token","inputRequests":{"astrid-consent":{"method":"elicitation/create","params":{"mode":"form","message":"Allow this capsule to continue?","requestedSchema":{"type":"object","properties":{"grant":{"type":"boolean"}},"required":["grant"]}}}}}}'
printf 'emitted\n' > "$AOS_TEST_EMITTED"
IFS= read -r resume
if [ -n "$resume" ]; then
  printf '%s\n' "$resume" > "$AOS_TEST_RESUME"
  exit 92
fi
exit 0
"#,
    );
    let mut child = fixture
        .command()
        .env("AOS_TEST_CANCEL", &cancel_path)
        .env("AOS_TEST_RESUME", &resume_path)
        .env("AOS_TEST_EMITTED", &emitted_path)
        .args(["mcp", "serve", "--interaction", "deny"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("start isolated bridge");
    let mut input = child.stdin.take().expect("bridge input");
    let output = child.stdout.take().expect("bridge output");
    let (sender, receiver) = mpsc::sync_channel(1);
    let reader = std::thread::spawn(move || {
        let mut line = String::new();
        let result = BufReader::new(output).read_line(&mut line).map(|_| line);
        let _ = sender.send(result);
    });
    input
        .write_all(
            br#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"fs.read","arguments":{"path":"/tmp/report"}}}"#,
        )
        .expect("send tools/call");
    input.write_all(b"\n").expect("newline");
    input.flush().expect("flush tools/call");
    let first = wait_for_file(&fixture.args, Duration::from_secs(5));
    let first: serde_json::Value = serde_json::from_slice(&first).expect("tools/call JSON");
    assert_eq!(first["params"]["name"], "fs.read");
    input
        .write_all(
            br#"{"jsonrpc":"2.0","method":"notifications/cancelled","params":{"requestId":7}}"#,
        )
        .expect("send cancel");
    input.write_all(b"\n").expect("newline");
    input.flush().expect("flush cancel");
    let cancel = wait_for_file(&cancel_path, Duration::from_secs(5));
    let cancel: serde_json::Value = serde_json::from_slice(&cancel).expect("cancel JSON");
    assert_eq!(cancel["method"], "notifications/cancelled");
    assert_eq!(cancel["params"]["requestId"], 7);
    let _ = wait_for_file(&emitted_path, Duration::from_secs(5));
    let observed = receiver.recv_timeout(Duration::from_secs(1));
    drop(input);
    let status = wait_for_child(&mut child, Duration::from_secs(5));
    reader.join().expect("reader completed");
    assert!(
        observed.is_err(),
        "cancelled input_required must not reach the host or emit local consent, got {observed:?}"
    );
    assert!(status.success(), "bridge exited {status}");
    assert!(
        !resume_path.exists(),
        "cancelled tools/call must not be resumed to the runtime"
    );
}

#[test]
fn exhausted_input_rounds_error_the_host_and_reclaim_capacity() {
    let fixture = Fixture::new("mrtr-exhausted-rounds");
    let cancel_path = fixture.root.join("cancel");
    let second_path = fixture.root.join("second");
    fixture.install_runtime(
        r#"#!/bin/sh
IFS= read -r request || exit 90
printf '%s\n' "$request" > "$AOS_TEST_ARGS"
printf '%s\n' '{"jsonrpc":"2.0","id":7,"result":{"resultType":"input_required","requestState":"round-a","inputRequests":{"astrid-consent":{"method":"elicitation/create","params":{"mode":"form","message":"Allow this capsule to continue?","requestedSchema":{"type":"object","properties":{"grant":{"type":"boolean"}},"required":["grant"]}}}}}}'
IFS= read -r _resume_a || exit 91
printf '%s\n' '{"jsonrpc":"2.0","id":7,"result":{"resultType":"input_required","requestState":"round-b","inputRequests":{"astrid-consent":{"method":"elicitation/create","params":{"mode":"form","message":"Allow this capsule to continue?","requestedSchema":{"type":"object","properties":{"grant":{"type":"boolean"}},"required":["grant"]}}}}}}'
IFS= read -r _resume_b || exit 92
printf '%s\n' '{"jsonrpc":"2.0","id":7,"result":{"resultType":"input_required","requestState":"round-c","inputRequests":{"astrid-consent":{"method":"elicitation/create","params":{"mode":"form","message":"Allow this capsule to continue?","requestedSchema":{"type":"object","properties":{"grant":{"type":"boolean"}},"required":["grant"]}}}}}}'
IFS= read -r cancel || exit 93
printf '%s\n' "$cancel" > "$AOS_TEST_CANCEL"
IFS= read -r second || exit 94
printf '%s\n' "$second" > "$AOS_TEST_SECOND"
printf '%s\n' '{"jsonrpc":"2.0","id":8,"result":{"content":[{"type":"text","text":"next"}]}}'
"#,
    );
    let mut child = fixture
        .command()
        .env("AOS_TEST_CANCEL", &cancel_path)
        .env("AOS_TEST_SECOND", &second_path)
        .args([
            "mcp",
            "serve",
            "--interaction",
            "deny",
            "--max-in-flight-calls",
            "1",
            "--max-input-rounds",
            "2",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("start isolated bridge");
    let mut input = child.stdin.take().expect("bridge input");
    let output = child.stdout.take().expect("bridge output");
    let (sender, receiver) = mpsc::sync_channel(4);
    let reader = std::thread::spawn(move || {
        let mut reader = BufReader::new(output);
        loop {
            let mut line = String::new();
            match reader.read_line(&mut line) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    if sender.send(line).is_err() {
                        break;
                    }
                }
            }
        }
    });
    input
        .write_all(
            br#"{"jsonrpc":"2.0","id":7,"method":"tools/call","params":{"name":"fs.read","arguments":{"path":"/tmp/report"}}}"#,
        )
        .expect("send first tools/call");
    input.write_all(b"\n").expect("newline");
    input.flush().expect("flush first tools/call");
    let first = wait_for_file(&fixture.args, Duration::from_secs(5));
    let first: serde_json::Value = serde_json::from_slice(&first).expect("first tools/call JSON");
    assert_eq!(first["params"]["name"], "fs.read");
    let error_line = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("host must receive a terminal error for the exhausted call");
    let host_error: serde_json::Value =
        serde_json::from_str(&error_line).expect("exhausted-call host JSON");
    assert_eq!(host_error["id"], 7);
    assert_eq!(host_error["error"]["code"], -32603);
    assert_eq!(
        host_error["error"]["message"],
        "tracked tools/call exceeded the native input round limit"
    );
    let cancel = wait_for_file(&cancel_path, Duration::from_secs(5));
    let cancel: serde_json::Value = serde_json::from_slice(&cancel).expect("cancel JSON");
    assert_eq!(cancel["method"], "notifications/cancelled");
    assert_eq!(cancel["params"]["requestId"], 7);
    input
        .write_all(
            br#"{"jsonrpc":"2.0","id":8,"method":"tools/call","params":{"name":"fs.write","arguments":{"path":"/tmp/other"}}}"#,
        )
        .expect("send second tools/call");
    input.write_all(b"\n").expect("newline");
    input.flush().expect("flush second tools/call");
    let second = wait_for_file(&second_path, Duration::from_secs(5));
    let second: serde_json::Value =
        serde_json::from_slice(&second).expect("second tools/call JSON");
    assert_eq!(second["id"], 8);
    assert_eq!(second["params"]["name"], "fs.write");
    let result_line = receiver
        .recv_timeout(Duration::from_secs(5))
        .expect("reclaimed slot must deliver the next host result");
    drop(input);
    let status = wait_for_child(&mut child, Duration::from_secs(5));
    reader.join().expect("reader completed");
    let host_result: serde_json::Value =
        serde_json::from_str(&result_line).expect("second-call host JSON");
    assert_eq!(
        host_result,
        serde_json::json!({"jsonrpc":"2.0","id":8,"result":{"content":[{"type":"text","text":"next"}]}})
    );
    assert!(status.success(), "bridge exited {status}");
}

fn wait_for_file(path: &Path, timeout: Duration) -> Vec<u8> {
    let deadline = Instant::now() + timeout;
    loop {
        match fs::read(path) {
            Ok(bytes) if !bytes.is_empty() => return bytes,
            _ if Instant::now() >= deadline => {
                panic!("did not observe {} within {timeout:?}", path.display())
            }
            _ => std::thread::sleep(Duration::from_millis(10)),
        }
    }
}
