//! Read an existing FSKit lease without invoking installation or mount commands.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::{Component, Path, PathBuf};

use astrid_core::PrincipalId;
use astrid_core::kernel_api::{AdminRequestKind, AdminResponseBody};
use astrid_core::storage_provider::{StorageMountId, StorageProviderAccessV1};
use astrid_uplink::admin_client::AdminClient;
use serde::{Deserialize, Serialize};

use super::STATUS_TIMEOUT;
use crate::AosHome;

/// Public navigation metadata only; no callback endpoints or bearer credentials.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub struct MountedVolume {
    mount_id: StorageMountId,
    mountpoint: PathBuf,
    provider: String,
    access: StorageProviderAccessV1,
}

#[derive(Deserialize)]
struct Registry {
    mounts: BTreeMap<String, Record>,
}

#[derive(Deserialize)]
struct Record {
    mount_id: StorageMountId,
    mountpoint: PathBuf,
}

/// The private provider registry locates a lease; only the runtime validates it.
/// This operation never mounts, starts, provisions, or writes a Distro manifest.
pub async fn read(
    home: &AosHome,
    principal: PrincipalId,
    selected: &Path,
) -> Result<MountedVolume, String> {
    if !cfg!(target_os = "macos") {
        return Err("mounted-volume navigation is currently macOS-only".to_owned());
    }
    if !selected.is_absolute()
        || selected
            .components()
            .any(|part| matches!(part, Component::ParentDir | Component::CurDir))
    {
        return Err("select an absolute mount root".to_owned());
    }
    let path = home.run_root().join("providers/fskit-mounts.json");
    let parent = path.parent().ok_or("missing registry parent")?;
    astrid_core::platform_fs::validate_private_directory(parent).map_err(|e| e.to_string())?;
    astrid_core::platform_fs::validate_private_file(&path).map_err(|e| e.to_string())?;
    let mut bytes = Vec::new();
    std::fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 65_536 {
        return Err("mount registry exceeds read limit".to_owned());
    }
    let id = select_record(&bytes, selected)?;
    let mut client = tokio::time::timeout(STATUS_TIMEOUT, AdminClient::connect(principal))
        .await
        .map_err(|_| "mount connection timed out")?
        .map_err(|e| e.to_string())?;
    let response = tokio::time::timeout(
        STATUS_TIMEOUT,
        client.request(AdminRequestKind::StorageMountStatus { mount_id: id }),
    )
    .await
    .map_err(|_| "mount status timed out")?
    .map_err(|e| e.to_string())?;
    match response {
        AdminResponseBody::Success(value) => validate_response(value, id, selected),
        _ => Err("runtime did not authorize the selected mount".to_owned()),
    }
}

fn select_record(bytes: &[u8], selected: &Path) -> Result<StorageMountId, String> {
    let registry: Registry = serde_json::from_slice(bytes).map_err(|e| e.to_string())?;
    let key = selected.to_str().ok_or("mount path is not UTF-8")?;
    let record = registry
        .mounts
        .get(key)
        .ok_or("selected folder is not registered to this AOS runtime")?;
    if record.mountpoint != selected {
        return Err("mount registry path mismatch".to_owned());
    }
    Ok(record.mount_id)
}

fn validate_response(
    value: serde_json::Value,
    id: StorageMountId,
    selected: &Path,
) -> Result<MountedVolume, String> {
    let mount: MountedVolume = serde_json::from_value(value).map_err(|e| e.to_string())?;
    if mount.mount_id != id
        || mount.mountpoint != selected
        || mount.provider != "astrid-storage-provider-fskit"
    {
        return Err("runtime mount identity does not match selection".to_owned());
    }
    Ok(mount)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn registry_only_selects_an_exact_registered_root() {
        let id = StorageMountId::new();
        let bytes = serde_json::to_vec(&json!({"mounts": {"/Volumes/AOS": {
            "mount_id": id, "mountpoint": "/Volumes/AOS"
        }}}))
        .unwrap();
        assert_eq!(
            select_record(&bytes, Path::new("/Volumes/AOS")).unwrap(),
            id
        );
        assert!(select_record(&bytes, Path::new("/Volumes/AOS/home")).is_err());
        assert!(select_record(&bytes, Path::new("/Volumes/Other")).is_err());
    }

    #[test]
    fn runtime_identity_must_match_and_private_fields_are_not_forwarded() {
        let id = StorageMountId::new();
        let value = json!({"mount_id": id, "mountpoint": "/Volumes/AOS",
            "provider": "astrid-storage-provider-fskit", "access": "read-write",
            "callback_path": "private", "lease_token": "private"});
        let mount = validate_response(value.clone(), id, Path::new("/Volumes/AOS")).unwrap();
        assert_eq!(
            serde_json::to_value(mount)
                .unwrap()
                .as_object()
                .unwrap()
                .len(),
            4
        );
        assert!(
            validate_response(
                value.clone(),
                StorageMountId::new(),
                Path::new("/Volumes/AOS")
            )
            .is_err()
        );
        assert!(validate_response(value, id, Path::new("/Volumes/Other")).is_err());
    }
}
