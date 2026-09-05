//! Native AOS status over the runtime's typed local control operation.

use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io;
use std::time::Duration;

#[cfg(unix)]
use std::os::unix::fs::PermissionsExt;

use astrid_core::PrincipalId;
use astrid_core::kernel_api::{DaemonStatus, KernelRequest, KernelResponse};
use astrid_uplink::KernelClient;
use fs2::FileExt;
use serde::Serialize;

use crate::AosHome;

const STATUS_TIMEOUT: Duration = Duration::from_secs(5);
const RUNTIME_COMPATIBILITY: &str = include_str!("../../../release/runtime-compatibility.toml");

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
            return confirm_stopped(home)
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
/// every transient marker is gone, the runtime lock can be acquired, and any
/// materialized runtime state is an exclusive private volume.
pub fn confirm_stopped(home: &AosHome) -> Result<AosStatus, String> {
    let run_dir = home.runtime_home().join("run");
    for marker in ["system.sock", "system.pid", "system.ready", "system.token"] {
        match fs::symlink_metadata(run_dir.join(marker)) {
            Ok(_) => {
                return Err(format!(
                    "runtime coordination marker {marker} is still present"
                ));
            }
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(format!(
                    "could not inspect runtime marker {marker}: {error}"
                ));
            }
        }
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
            // Hold the singleton lock while inspecting the volume-only layout so
            // a concurrent runtime cannot change the state between the checks.
            validate_stopped_runtime_layout(home)?;
            FileExt::unlock(&lock)
                .map_err(|error| format!("could not release runtime status lock: {error}"))?;
            AosStatus::stopped()
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
            Err("runtime lock is still held".to_owned())
        }
        Err(error) => Err(format!("could not inspect runtime lock state: {error}")),
    }
}

fn validate_stopped_runtime_layout(home: &AosHome) -> Result<(), String> {
    let runtime = home.runtime_home();
    let metadata = match fs::symlink_metadata(&runtime) {
        Ok(metadata) => metadata,
        // A fresh AOS home has no runtime state yet. Preserve its stopped
        // projection until the first volume is materialized.
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(format!("could not inspect runtime state: {error}")),
    };
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err("runtime state must be a real directory".to_owned());
    }

    let mut unexpected = Vec::new();
    let mut volume_path = None;
    for entry in fs::read_dir(&runtime)
        .map_err(|error| format!("could not inspect stopped runtime state: {error}"))?
    {
        let entry =
            entry.map_err(|error| format!("could not enumerate stopped runtime state: {error}"))?;
        let name = entry.file_name();
        if name == OsStr::new("astrid.volume") {
            volume_path = Some(entry.path());
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

    let Some(volume_path) = volume_path else {
        // An empty runtime directory is the other valid pre-first-volume state.
        return Ok(());
    };
    let volume_metadata = fs::symlink_metadata(&volume_path)
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
    #[cfg(unix)]
    {
        let mode = volume_metadata.permissions().mode() & 0o7777;
        if mode != 0o600 {
            return Err(format!(
                "astrid.volume must use private mode 0600 (found {mode:04o})"
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};
    use std::time::{SystemTime, UNIX_EPOCH};

    use astrid_core::kernel_api::DaemonStatus;

    use super::{AosStatus, confirm_stopped};
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

    fn write_private_volume(runtime: &Path) -> PathBuf {
        let volume = runtime.join("astrid.volume");
        fs::write(&volume, b"volume-state").expect("create runtime volume");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;

            let mut permissions = fs::metadata(&volume)
                .expect("inspect runtime volume")
                .permissions();
            permissions.set_mode(0o600);
            fs::set_permissions(&volume, permissions).expect("make runtime volume private");
        }
        volume
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
    fn reports_a_typed_stopped_state_for_an_exclusive_private_volume() {
        let root = temporary_status_home("stopped");
        let home = AosHome::from_root(&root);
        let runtime = home.runtime_home();
        fs::create_dir_all(&runtime).expect("create runtime home");
        write_private_volume(&runtime);

        let status = confirm_stopped(&home).expect("read stopped status");
        assert_eq!(status.state, "stopped");
        assert_eq!(status.pid, 0);
        assert_eq!(status.runtime_version, "0.10.4");

        fs::remove_dir_all(root).expect("remove stopped status fixture");
    }

    #[test]
    fn allows_a_missing_or_empty_runtime_before_the_first_volume() {
        let missing_root = temporary_status_home("missing-runtime");
        let missing_home = AosHome::from_root(&missing_root);
        confirm_stopped(&missing_home).expect("missing runtime is stopped before first volume");
        drop(fs::remove_dir_all(&missing_root));

        let empty_root = temporary_status_home("empty-runtime");
        let empty_home = AosHome::from_root(&empty_root);
        fs::create_dir_all(empty_home.runtime_home()).expect("create empty runtime home");
        confirm_stopped(&empty_home).expect("empty runtime is stopped before first volume");
        fs::remove_dir_all(empty_root).expect("remove empty runtime fixture");
    }

    #[test]
    fn refuses_to_report_stopped_while_the_runtime_lock_is_held() {
        use fs2::FileExt as _;

        let root = temporary_status_home("running");
        let home = AosHome::from_root(&root);
        fs::create_dir_all(home.runtime_home().join("run")).expect("create runtime run dir");
        let lock_path = home.runtime_home().join("run/system.lock");
        fs::write(&lock_path, []).expect("create runtime lock");
        let lock = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&lock_path)
            .expect("open runtime lock");
        lock.try_lock_exclusive().expect("hold runtime lock");

        let error = confirm_stopped(&home).expect_err("held lock must not report stopped");
        assert!(error.contains("runtime lock is still held"));

        fs2::FileExt::unlock(&lock).expect("release runtime lock");
        fs::remove_dir_all(root).expect("remove running status fixture");
    }

    #[cfg(unix)]
    #[test]
    fn stopped_runtime_volume_must_use_private_0600_mode() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = temporary_status_home("volume-mode");
        let home = AosHome::from_root(&root);
        let runtime = home.runtime_home();
        fs::create_dir_all(&runtime).expect("create runtime home");
        let volume = write_private_volume(&runtime);
        let mut permissions = fs::metadata(&volume)
            .expect("inspect runtime volume")
            .permissions();
        permissions.set_mode(0o644);
        fs::set_permissions(&volume, permissions).expect("make runtime volume world-readable");

        let error = confirm_stopped(&home).expect_err("0644 volume must fail");
        assert!(
            error.contains("private"),
            "mode error lacked private: {error}"
        );
        assert!(error.contains("0600"), "mode error lacked 0600: {error}");

        fs::remove_dir_all(root).expect("remove volume mode fixture");
    }

    #[cfg(unix)]
    #[test]
    fn stopped_runtime_volume_must_reject_special_mode_bits() {
        use std::os::unix::fs::PermissionsExt as _;

        let root = temporary_status_home("volume-special-mode");
        let home = AosHome::from_root(&root);
        let runtime = home.runtime_home();
        fs::create_dir_all(&runtime).expect("create runtime home");
        let volume = write_private_volume(&runtime);
        let mut permissions = fs::metadata(&volume)
            .expect("inspect runtime volume")
            .permissions();
        permissions.set_mode(0o1600);
        fs::set_permissions(&volume, permissions).expect("set sticky private volume mode");

        let error = confirm_stopped(&home).expect_err("sticky 0600 volume must fail");
        assert!(
            error.contains("private"),
            "mode error lacked private: {error}"
        );
        assert!(error.contains("0600"), "mode error lacked 0600: {error}");
        assert!(
            error.contains("1600"),
            "mode error lacked special bits: {error}"
        );

        fs::remove_dir_all(root).expect("remove special mode fixture");
    }

    #[test]
    fn stopped_runtime_volume_must_not_be_empty() {
        let root = temporary_status_home("volume-empty");
        let home = AosHome::from_root(&root);
        let runtime = home.runtime_home();
        fs::create_dir_all(&runtime).expect("create runtime home");
        fs::write(runtime.join("astrid.volume"), []).expect("create empty volume");

        let error = confirm_stopped(&home).expect_err("empty volume must fail");
        assert!(error.contains("astrid.volume must not be empty"));

        fs::remove_dir_all(root).expect("remove empty volume fixture");
    }

    #[test]
    fn stopped_runtime_volume_must_be_a_regular_file() {
        let root = temporary_status_home("volume-directory");
        let home = AosHome::from_root(&root);
        let runtime = home.runtime_home();
        fs::create_dir_all(runtime.join("astrid.volume")).expect("create volume directory");

        let error = confirm_stopped(&home).expect_err("volume directory must fail");
        assert!(error.contains("astrid.volume must be a regular file"));

        fs::remove_dir_all(root).expect("remove volume directory fixture");
    }

    #[cfg(unix)]
    #[test]
    fn stopped_runtime_volume_must_not_be_a_symlink() {
        use std::os::unix::fs::symlink;

        let root = temporary_status_home("volume-symlink");
        let home = AosHome::from_root(&root);
        let runtime = home.runtime_home();
        fs::create_dir_all(&runtime).expect("create runtime home");
        let target = root.join("volume-target");
        fs::write(&target, b"volume-state").expect("create volume target");
        symlink(&target, runtime.join("astrid.volume")).expect("create volume symlink");

        let error = confirm_stopped(&home).expect_err("volume symlink must fail");
        assert!(error.contains("astrid.volume must not be a symlink"));

        fs::remove_dir_all(root).expect("remove volume symlink fixture");
    }

    #[test]
    fn stopped_runtime_must_contain_only_the_volume() {
        let root = temporary_status_home("stopped-layout");
        let home = AosHome::from_root(&root);
        let runtime = home.runtime_home();
        fs::create_dir_all(&runtime).expect("create runtime home");
        write_private_volume(&runtime);
        fs::create_dir_all(runtime.join("run")).expect("create residual run directory");

        let error = confirm_stopped(&home).expect_err("residual runtime state must fail");
        assert!(error.contains("unexpected state: run"));

        fs::remove_dir_all(root).expect("remove layout fixture");
    }
}
