use crate::support::*;
use std::ffi::OsStr;
use std::fs;
use std::io::Write as _;
use std::os::unix::process::ExitStatusExt;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

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
        fixture.selected_distro().to_string_lossy()
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
        format!("{}\n", fixture.selected_distro().display())
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
fn mcp_serve_preflights_the_signed_release_inventory() {
    let fixture = Fixture::new("mcp-signed-inventory");
    fixture.install_runtime(RECORDING_RUNTIME);
    let statement = fixture
        .home
        .join("releases")
        .join(env!("CARGO_PKG_VERSION"))
        .join("signed")
        .join(format!(
            "unicity-aos-{}-release.toml",
            env!("CARGO_PKG_VERSION")
        ));
    fs::remove_file(statement).expect("remove signed statement");

    let output = fixture
        .command()
        .args(["mcp", "serve"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .and_then(|mut child| {
            child
                .stdin
                .take()
                .expect("MCP bridge stdin")
                .write_all(b"")
                .map(|()| child)
        })
        .expect("start product MCP bridge")
        .wait_with_output()
        .expect("wait for MCP bridge");

    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("bundled runtime inventory preflight failed")
    );
    assert!(!fixture.args.exists());
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
fn runtime_override_cannot_bypass_release_provenance() {
    let fixture = Fixture::new("override-bound");
    fixture.install_runtime(RECORDING_RUNTIME);
    let external = fixture.root.join("external-astrid");
    fs::write(&external, "#!/bin/sh\necho EXTERNAL_RUNTIME_OVERRIDE\n")
        .expect("write external override");
    Fixture::make_executable(&external);

    let output = fixture
        .command()
        .env("UNICITY_AOS_RUNTIME_BIN", &external)
        .args(["doctor"])
        .output()
        .expect("run doctor with an override");

    assert!(output.status.success());
    assert_eq!(
        fs::read_to_string(&fixture.args).expect("read delegated args"),
        "<doctor>\n"
    );
    assert_eq!(
        fs::read_to_string(fixture.root.join("child-home")).expect("read runtime home"),
        format!("{}\n", fixture.home.join("runtime").display())
    );
    assert!(
        output.stdout.is_empty(),
        "the external executable must not run"
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
    for _ in 0..2_000 {
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
