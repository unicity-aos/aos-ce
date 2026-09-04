use crate::support::*;
use std::fs;
use std::time::{Duration, Instant};

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

    let ready_marker = fixture.home.join("run/system.ready");
    let runtime_volume = fixture.home.join("runtime/astrid.volume");
    fs::create_dir_all(fixture.home.join("runtime")).expect("create runtime home");
    fs::create_dir_all(ready_marker.parent().expect("runtime run directory"))
        .expect("create runtime run directory");
    fs::write(&ready_marker, []).expect("create runtime ready marker");
    let marker_to_remove = ready_marker.clone();
    let shutdown = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        fs::remove_file(&marker_to_remove).expect("remove runtime ready marker");
        fs::write(runtime_volume, b"volume-state").expect("create runtime volume");
        fs::remove_dir(marker_to_remove.parent().expect("runtime run directory"))
            .expect("remove runtime run directory");
    });
    seed_active_receipt(&fixture);

    let output = fixture
        .command()
        .args(["--future-runtime-global", "future-value", "stop"])
        .output()
        .expect("run inherited stop");
    shutdown.join().expect("finish runtime shutdown");

    assert!(
        output.status.success(),
        "status={:?}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        fs::read_to_string(&fixture.args).expect("read delegated stop args"),
        "<--future-runtime-global>\n<future-value>\n<stop>\n"
    );
    assert!(!ready_marker.exists());
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

    let ready_marker = fixture.home.join("run/system.ready");
    let runtime_volume = fixture.home.join("runtime/astrid.volume");
    fs::create_dir_all(fixture.home.join("runtime")).expect("create runtime home");
    fs::create_dir_all(ready_marker.parent().expect("runtime run directory"))
        .expect("create runtime run directory");
    fs::write(&ready_marker, []).expect("create runtime ready marker");
    let marker_to_remove = ready_marker.clone();
    let shutdown = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(200));
        fs::remove_file(&marker_to_remove).expect("remove runtime ready marker");
        fs::write(runtime_volume, b"volume-state").expect("create runtime volume");
        fs::remove_dir(marker_to_remove.parent().expect("runtime run directory"))
            .expect("remove runtime run directory");
    });
    seed_active_receipt(&fixture);

    let output = fixture
        .command()
        .arg("stop")
        .output()
        .expect("run successful inherited stop");
    shutdown.join().expect("finish runtime shutdown");

    assert!(
        output.status.success(),
        "status={:?}, stderr={}",
        output.status,
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(
        String::from_utf8(output.stdout).expect("utf8 stop output"),
        "runtime stop complete\n"
    );
    assert!(output.stderr.is_empty());
    assert!(!ready_marker.exists());
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

    let token = fixture.home.join("run/system.token");
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
        started.elapsed() < Duration::from_secs(30),
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

    let gateway = fixture.home.join("run/mcp-gateway.ready");
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
        "confirmation must not hide a stale marker"
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

    let gateway = fixture.home.join("run/mcp-gateway.sock");
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
    seed_stopped_volume(&fixture);
    seed_active_receipt(&fixture);

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
fn inherited_stop_fails_closed_on_a_tampered_release_runtime() {
    let fixture = Fixture::new("tampered-stop");
    fixture.install_runtime("#!/bin/sh\nexit 0\n");
    fs::write(&fixture.runtime, "#!/bin/sh\necho STOP_TAMPERED_EXECUTED\n")
        .expect("tamper release runtime");

    let output = fixture.command().args(["stop"]).output().expect("run stop");

    assert_eq!(output.status.code(), Some(1));
    assert!(
        String::from_utf8_lossy(&output.stderr)
            .contains("release executable does not match the authenticated archive")
    );
    assert!(!fixture.args.exists());
}
