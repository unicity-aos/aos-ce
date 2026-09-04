//! Native AOS status over the runtime's typed local control operation.

use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io;
use std::time::Duration;

use astrid_core::PrincipalId;
use astrid_core::kernel_api::{DaemonStatus, KernelRequest, KernelResponse};
use astrid_uplink::KernelClient;
use fs2::FileExt;
use serde::Serialize;

use crate::AosHome;
use crate::distro_trust;

const STATUS_TIMEOUT: Duration = Duration::from_secs(5);
const RUNTIME_COMPATIBILITY: &str = include_str!("../../../release/runtime-compatibility.toml");
const COORDINATION_MARKERS: [&str; 6] = [
    "system.sock",
    "system.pid",
    "system.ready",
    "system.token",
    "mcp-gateway.sock",
    "mcp-gateway.ready",
];

/// Product status derived from the typed runtime status response.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct AosStatus {
    pub state: &'static str,
    pub pid: u32,
    pub uptime_secs: u64,
    pub runtime_version: String,
    pub ephemeral: bool,
    pub connected_clients: u32,
    pub loaded_capsules: Vec<String>,
}

impl From<DaemonStatus> for AosStatus {
    fn from(status: DaemonStatus) -> Self {
        Self {
            state: "running",
            pid: status.pid,
            uptime_secs: status.uptime_secs,
            runtime_version: status.version,
            ephemeral: status.ephemeral,
            connected_clients: status.connected_clients,
            loaded_capsules: status.loaded_capsules,
        }
    }
}

impl AosStatus {
    fn stopped() -> Result<Self, String> {
        let compatibility = RUNTIME_COMPATIBILITY
            .parse::<toml::Value>()
            .map_err(|error| format!("embedded runtime compatibility is invalid: {error}"))?;
        let runtime_version = compatibility
            .get("runtime")
            .and_then(|runtime| runtime.get("version"))
            .and_then(toml::Value::as_str)
            .ok_or_else(|| "embedded runtime compatibility has no runtime version".to_owned())?
            .to_owned();
        Ok(Self {
            state: "stopped",
            pid: 0,
            uptime_secs: 0,
            runtime_version,
            ephemeral: false,
            connected_clients: 0,
            loaded_capsules: Vec::new(),
        })
    }
}

/// Read status through the typed authenticated local control client using the
/// single-user compatibility principal.
pub async fn read(home: &AosHome) -> Result<AosStatus, String> {
    read_for_principal(home, PrincipalId::default()).await
}

/// Read status through the typed authenticated local control client as
/// `principal`.
pub async fn read_for_principal(
    home: &AosHome,
    principal: PrincipalId,
) -> Result<AosStatus, String> {
    let connection = tokio::time::timeout(STATUS_TIMEOUT, KernelClient::connect(principal))
        .await
        .map_err(|_| "connection timed out".to_owned())
        .and_then(|result| {
            result.map_err(|error| format!("could not connect to the local runtime: {error}"))
        });
    let mut client = match connection {
        Ok(client) => client,
        Err(connection_error) => {
            return reopen_stopped(home)
                .await
                .map_err(|state_error| format!("{connection_error}; {state_error}"));
        }
    };

    let response = tokio::time::timeout(STATUS_TIMEOUT, client.request(KernelRequest::GetStatus))
        .await
        .map_err(|_| "status request timed out".to_owned())?
        .map_err(|error| format!("status request failed: {error}"))?;

    match response {
        KernelResponse::Status(status) => Ok(status.into()),
        KernelResponse::Error(error) => Err(error),
        _ => Err("runtime returned an unexpected status response".to_owned()),
    }
}

/// Confirm that the runtime has released its coordination state and singleton
/// lock.
///
/// This is stricter than a missing socket: a shutdown is complete only after
/// every transient marker is gone and the runtime lock can be acquired.
pub fn confirm_stopped(home: &AosHome) -> Result<AosStatus, String> {
    let status = confirm_stopped_projection(home)?;
    distro_trust::validate_active_receipt(home, &home.selected_distro_path())
        .map_err(|error| format!("active distribution receipt is not GO: {error}"))?;
    Ok(status)
}

/// Confirm runtime shutdown before the product receipt is rewritten.
pub fn confirm_stopped_projection(home: &AosHome) -> Result<AosStatus, String> {
    let run_dir = home.run_root();
    let mut present = Vec::new();
    for marker in COORDINATION_MARKERS {
        match fs::symlink_metadata(run_dir.join(marker)) {
            Ok(_) => present.push(marker),
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "could not inspect runtime marker {marker}: {error}"
                ));
            }
        }
    }
    if !present.is_empty() {
        return Err(format!(
            "runtime coordination marker(s) still present: {}",
            present.join(", ")
        ));
    }

    let lock_path = run_dir.join("system.lock");
    let lock_metadata = match fs::symlink_metadata(&lock_path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            validate_stopped_runtime_layout(home)?;
            return AosStatus::stopped();
        }
        Err(error) => return Err(format!("could not inspect runtime lock: {error}")),
    };
    if lock_metadata.file_type().is_symlink() || !lock_metadata.is_file() {
        return Err("runtime lock is not a real regular file".to_owned());
    }
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(&lock_path)
        .map_err(|error| format!("could not open runtime lock: {error}"))?;
    match lock.try_lock_exclusive() {
        Ok(()) => {
            FileExt::unlock(&lock)
                .map_err(|error| format!("could not release runtime status lock: {error}"))?;
            validate_stopped_runtime_layout(home)?;
            AosStatus::stopped()
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
            Err("runtime lock is still held".to_owned())
        }
        Err(error) => Err(format!("could not inspect runtime lock state: {error}")),
    }
}

/// Bounded wait for the runtime projection to return to volume-only state.
pub fn wait_for_stopped_projection(home: &AosHome) -> Result<(), String> {
    const ATTEMPTS: usize = 100;
    const INTERVAL: Duration = Duration::from_millis(50);
    let mut last = "runtime did not reach a stopped state".to_owned();
    for attempt in 0..ATTEMPTS {
        match confirm_stopped_projection(home) {
            Ok(_) => return Ok(()),
            Err(error) => last = error,
        }
        if attempt + 1 < ATTEMPTS {
            std::thread::sleep(INTERVAL);
        }
    }
    Err(last)
}

/// Reopen a volume-only home through the real runtime lifecycle.
///
/// A structurally valid volume can still lack the runtime's ACTIVE receipt or
/// have arbitrary unadmitted host files. The only honest stopped GO is to let
/// the exact runtime perform its normal mount/admission checks and clean stop.
async fn reopen_stopped(home: &AosHome) -> Result<AosStatus, String> {
    confirm_stopped(home)?;
    run_lifecycle(home, &["start"]).await?;
    stop_and_confirm(home).await?;
    confirm_stopped(home)
}

async fn stop_and_confirm(home: &AosHome) -> Result<(), String> {
    run_lifecycle(home, &["stop"]).await?;
    tokio::task::spawn_blocking({
        let home = home.clone();
        move || wait_for_stopped_projection(&home)
    })
    .await
    .map_err(|error| format!("stopped-state wait failed: {error}"))?
}

async fn run_lifecycle(home: &AosHome, args: &[&str]) -> Result<(), String> {
    let home = home.clone();
    let args = args.iter().map(OsString::from).collect::<Vec<_>>();
    let status = tokio::task::spawn_blocking(move || home.run_runtime_lifecycle(args))
        .await
        .map_err(|error| format!("runtime lifecycle join failed: {error}"))?
        .map_err(|error| format!("runtime lifecycle failed to start: {error}"))?;
    if status.success() {
        return Ok(());
    }
    Err(format!(
        "runtime lifecycle command exited with {}",
        status.code().unwrap_or(1)
    ))
}

fn validate_stopped_runtime_layout(home: &AosHome) -> Result<(), String> {
    let runtime = home.runtime_home();
    let metadata = match fs::symlink_metadata(&runtime) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err("stopped runtime state is missing".to_owned());
        }
        Err(error) => return Err(format!("could not inspect runtime state: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("runtime state must be a real directory".to_owned());
    }

    let mut unexpected = Vec::new();
    let mut volume_entry = None;
    for entry in fs::read_dir(runtime)
        .map_err(|error| format!("could not inspect stopped runtime state: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("could not enumerate stopped runtime state: {error}"))?;
        let name = entry.file_name();
        if name.as_encoded_bytes() == b"astrid.volume" {
            volume_entry = Some(entry);
        } else {
            unexpected.push(name.to_string_lossy().into_owned());
        }
    }
    if !unexpected.is_empty() {
        unexpected.sort();
        return Err(format!(
            "stopped runtime contains unexpected state: {}",
            unexpected.join(", ")
        ));
    }

    let volume = match volume_entry {
        Some(volume) => volume,
        None => return Err("stopped runtime is missing its astrid.volume state".to_owned()),
    };
    let volume_metadata = volume
        .metadata()
        .map_err(|error| format!("could not inspect astrid.volume: {error}"))?;
    if volume_metadata.file_type().is_symlink() {
        return Err("astrid.volume must not be a symlink".to_owned());
    }
    if !volume_metadata.is_file() {
        return Err("astrid.volume must be a regular file".to_owned());
    }
    if volume_metadata.len() == 0 {
        return Err("astrid.volume must not be empty".to_owned());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    use astrid_core::kernel_api::DaemonStatus;

    use super::{AosStatus, confirm_stopped, confirm_stopped_projection};
    use crate::AosHome;

    fn temporary_status_home(case: &str) -> PathBuf {
        std::env::temp_dir().join(format!(
            "unicity-aos-{case}-status-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ))
    }

    #[test]
    fn maps_typed_runtime_status_to_product_status() {
        let status = AosStatus::from(DaemonStatus {
            pid: 42,
            uptime_secs: 90,
            version: "0.9.4".to_owned(),
            ephemeral: false,
            connected_clients: 3,
            connections_by_principal: Vec::new(),
            loaded_capsules: vec!["agents".to_owned(), "session".to_owned()],
        });

        assert_eq!(status.state, "running");
        assert_eq!(status.pid, 42);
        assert_eq!(status.runtime_version, "0.9.4");
        assert_eq!(status.loaded_capsules, ["agents", "session"]);
    }

    #[test]
    fn json_has_aos_owned_field_names() {
        let status = AosStatus {
            state: "running",
            pid: 7,
            uptime_secs: 8,
            runtime_version: "0.9.4".to_owned(),
            ephemeral: false,
            connected_clients: 1,
            loaded_capsules: vec!["agents".to_owned()],
        };

        let value = serde_json::to_value(status).expect("serialize status");
        assert_eq!(value["state"], "running");
        assert_eq!(value["runtime_version"], "0.9.4");
        assert!(value.get("astrid").is_none());
    }

    #[test]
    fn reports_a_typed_stopped_state_when_the_runtime_lock_is_available() {
        let root = temporary_status_home("stopped");
        let home = AosHome::from_root(&root);
        fs::create_dir_all(home.runtime_home()).expect("create runtime home");
        fs::write(home.runtime_home().join("astrid.volume"), b"volume-state")
            .expect("create runtime volume");

        let status = confirm_stopped_projection(&home).expect("read stopped status");
        assert_eq!(status.state, "stopped");
        assert_eq!(status.pid, 0);
        assert_eq!(status.runtime_version, "0.10.4");

        fs::remove_dir_all(root).expect("remove stopped status fixture");
    }

    #[test]
    fn refuses_to_report_stopped_while_the_runtime_lock_is_held() {
        use fs2::FileExt as _;

        let root = temporary_status_home("running");
        let home = AosHome::from_root(&root);
        fs::create_dir_all(home.run_root()).expect("create runtime run dir");
        let lock_path = home.run_root().join("system.lock");
        fs::write(&lock_path, []).expect("create runtime lock");
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .expect("open runtime lock");
        lock.try_lock_exclusive().expect("hold runtime lock");

        let error =
            confirm_stopped_projection(&home).expect_err("held lock must not report stopped");
        assert!(error.contains("runtime lock is still held"));

        fs2::FileExt::unlock(&lock).expect("release runtime lock");
        fs::remove_dir_all(root).expect("remove running status fixture");
    }

    #[test]
    fn stopped_runtime_must_contain_only_the_volume() {
        let root = temporary_status_home("stopped-layout");
        let home = AosHome::from_root(&root);
        let runtime = home.runtime_home();
        fs::create_dir_all(&runtime).expect("create runtime home");
        fs::write(runtime.join("astrid.volume"), b"volume-state").expect("create runtime volume");
        fs::create_dir_all(runtime.join("run")).expect("create residual run directory");

        let error =
            confirm_stopped_projection(&home).expect_err("residual runtime state must fail");
        assert!(error.contains("unexpected state: run"));

        fs::remove_dir_all(runtime.join("run")).expect("remove residual");
        confirm_stopped_projection(&home).expect("exact volume-only runtime is stopped");
        fs::remove_dir_all(root).expect("remove layout fixture");
    }

    #[test]
    fn refuses_to_report_stopped_while_any_coordination_marker_remains() {
        for marker in super::COORDINATION_MARKERS {
            let root = temporary_status_home(marker);
            let home = AosHome::from_root(&root);
            let run_dir = home.run_root();
            fs::create_dir_all(&run_dir).expect("create runtime run dir");
            fs::write(run_dir.join(marker), []).expect("create coordination marker");

            let error =
                confirm_stopped_projection(&home).expect_err("marker must prevent stopped status");
            assert!(error.contains(marker), "{marker} was not named in: {error}");

            fs::remove_dir_all(root).expect("remove marker fixture");
        }
    }

    #[test]
    fn reports_every_remaining_system_and_gateway_marker() {
        let root = temporary_status_home("combined-markers");
        let home = AosHome::from_root(&root);
        let run_dir = home.run_root();
        fs::create_dir_all(&run_dir).expect("create runtime run dir");
        for marker in super::COORDINATION_MARKERS {
            fs::write(run_dir.join(marker), []).expect("create coordination marker");
        }

        let error =
            confirm_stopped_projection(&home).expect_err("markers must prevent stopped status");
        for marker in super::COORDINATION_MARKERS {
            assert!(error.contains(marker), "{marker} was not named in: {error}");
        }

        fs::remove_dir_all(root).expect("remove combined marker fixture");
    }

    #[test]
    fn stopped_runtime_requires_a_missing_state_to_fail_closed() {
        let root = temporary_status_home("missing-runtime");
        let home = AosHome::from_root(&root);

        let error = confirm_stopped(&home).expect_err("missing runtime must not look stopped");
        assert!(error.contains("stopped runtime state is missing"));

        drop(fs::remove_dir_all(root));
    }

    #[test]
    fn stopped_runtime_requires_the_volume_state_entry() {
        let root = temporary_status_home("empty-runtime");
        let home = AosHome::from_root(&root);
        fs::create_dir_all(home.runtime_home()).expect("create empty runtime home");

        let error = confirm_stopped(&home).expect_err("empty runtime must not look stopped");
        assert!(error.contains("missing its astrid.volume state"));

        fs::remove_dir_all(root).expect("remove empty runtime fixture");
    }

    #[test]
    fn stopped_runtime_volume_must_be_a_regular_file() {
        let root = temporary_status_home("volume-type");
        let home = AosHome::from_root(&root);
        fs::create_dir_all(home.runtime_home().join("astrid.volume"))
            .expect("create volume directory");

        let error = confirm_stopped(&home).expect_err("volume directory must fail");
        assert!(error.contains("astrid.volume must be a regular file"));

        fs::remove_dir_all(root).expect("remove volume type fixture");
    }

    #[test]
    fn stopped_runtime_volume_must_not_be_empty() {
        let root = temporary_status_home("volume-empty");
        let home = AosHome::from_root(&root);
        fs::create_dir_all(home.runtime_home()).expect("create runtime home");
        fs::write(home.runtime_home().join("astrid.volume"), []).expect("create empty volume");

        let error = confirm_stopped(&home).expect_err("empty volume must fail");
        assert!(error.contains("astrid.volume must not be empty"));

        fs::remove_dir_all(root).expect("remove empty volume fixture");
    }

    #[cfg(unix)]
    #[test]
    fn stopped_runtime_volume_must_not_be_a_symlink() {
        use std::os::unix::fs::symlink;

        let root = temporary_status_home("volume-symlink");
        let home = AosHome::from_root(&root);
        let runtime = home.runtime_home();
        fs::create_dir_all(&runtime).expect("create runtime home");
        fs::create_dir_all(&root).expect("create fixture root");
        fs::write(root.join("volume-target"), b"volume-state").expect("create volume target");
        symlink(root.join("volume-target"), runtime.join("astrid.volume"))
            .expect("create volume symlink");

        let error = confirm_stopped(&home).expect_err("volume symlink must fail");
        assert!(error.contains("astrid.volume must not be a symlink"));

        fs::remove_dir_all(root).expect("remove volume symlink fixture");
    }
}
