//! Principal-scoped metadata snapshots, never private copies of capsule bytes.

use astrid_sdk::{SysError, kv, runtime};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::BTreeMap;

const SNAPSHOT_KEY: &str = "system.loaded_inventory";

#[derive(Debug, Deserialize, Serialize)]
pub(super) struct Snapshot {
    pub principal: String,
    pub observed_at: String,
    pub capsules: BTreeMap<String, Value>,
}

#[derive(Deserialize)]
struct LoadedView {
    capsules: Vec<LoadedCapsule>,
}

#[derive(Deserialize)]
struct LoadedCapsule {
    principal: String,
    name: String,
    meta: Value,
}

fn error(message: impl Into<String>) -> SysError {
    SysError::ApiError(message.into())
}

impl Snapshot {
    fn from_event(payload: Value, principal: &str, observed_at: String) -> Result<Self, SysError> {
        let view: LoadedView = serde_json::from_value(payload)?;
        let mut capsules = BTreeMap::new();
        for entry in view.capsules {
            if entry.principal != principal {
                return Err(error("capsule inventory belongs to a different principal"));
            }
            if entry.name.is_empty() || entry.name.contains(['/', '\\']) || entry.name == ".." {
                return Err(error("invalid capsule name in runtime inventory"));
            }
            let mut meta = entry.meta;
            // Descriptors are large, live broker data. Inspection needs only
            // the installed metadata, including its shared WASM hash.
            if let Some(object) = meta.as_object_mut() {
                object.remove("tools");
                if object.is_empty() {
                    // A successful live describe does not prove that the
                    // runtime could read installed metadata.
                    meta = Value::Null;
                }
            }
            if capsules.insert(entry.name, meta).is_some() {
                return Err(error("duplicate capsule in runtime inventory"));
            }
        }
        Ok(Self {
            principal: principal.to_owned(),
            observed_at,
            capsules,
        })
    }

    pub(super) fn metadata(&self, name: &str) -> Result<&Value, SysError> {
        let meta = self.capsules.get(name).ok_or_else(|| {
            error(format!(
                "capsule '{name}' is not in this principal's loaded snapshot"
            ))
        })?;
        if meta.as_object().is_none_or(serde_json::Map::is_empty) {
            return Err(error(format!(
                "runtime metadata is unavailable for capsule '{name}'"
            )));
        }
        Ok(meta)
    }
}

pub(super) fn receive(payload: Value) -> Result<(), SysError> {
    let caller = runtime::caller()?;
    let principal = caller
        .principal
        .ok_or_else(|| error("inventory event has no principal"))?;
    let snapshot = Snapshot::from_event(payload, &principal, caller.timestamp)?;
    // The kernel scopes KV by invocation principal and capsule. Never persist
    // shared executable bytes or use a principal supplied by tool arguments.
    // Snapshot broadcasts use the dispatcher's ordered per-(capsule, principal)
    // consumer; it awaits this handler before processing the next event.
    kv::set_json(SNAPSHOT_KEY, &snapshot)
}

pub(super) fn load() -> Result<Snapshot, SysError> {
    let caller = runtime::caller()?;
    let snapshot: Snapshot = kv::get_json_opt(SNAPSHOT_KEY)?
        .ok_or_else(|| error("runtime capsule inventory has not been observed yet"))?;
    if caller.principal.as_deref() != Some(snapshot.principal.as_str()) {
        return Err(error(
            "capsule inventory principal does not match this invocation",
        ));
    }
    Ok(snapshot)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn references_shared_bytes_without_private_capsule_directories() {
        let snapshot = Snapshot::from_event(json!({"capsules":[{
            "principal":"alice", "name":"aos-fs",
            "meta":{"version":"1.0.0", "wasm_hash":"shared-hash", "tools":[{"name":"read_file"}]}
        }]}), "alice", "now".into()).unwrap();
        assert_eq!(
            snapshot.metadata("aos-fs").unwrap()["wasm_hash"],
            "shared-hash"
        );
        assert!(snapshot.metadata("aos-fs").unwrap().get("tools").is_none());
        assert!(snapshot.metadata("missing").is_err());
    }

    #[test]
    fn rejects_other_principals_and_malformed_or_duplicate_inventory() {
        for payload in [
            json!({}),
            json!({"capsules":[{"principal":"bob","name":"cap","meta":{}}]}),
            json!({"capsules":[{"principal":"alice","name":"../cap","meta":{}}]}),
            json!({"capsules":[{"principal":"alice","name":"cap","meta":{}},
                                {"principal":"alice","name":"cap","meta":{}}]}),
        ] {
            assert!(Snapshot::from_event(payload, "alice", "now".into()).is_err());
        }
    }

    #[test]
    fn empty_snapshot_removes_previous_entries_but_unavailable_metadata_is_not_health() {
        let empty = Snapshot::from_event(json!({"capsules":[]}), "alice", "now".into()).unwrap();
        assert!(empty.capsules.is_empty());
        let missing = Snapshot::from_event(
            json!({"capsules":[{
                "principal":"alice","name":"broken","meta":null
            }]}),
            "alice",
            "now".into(),
        )
        .unwrap();
        assert!(missing.metadata("broken").is_err());
    }

    #[test]
    fn live_tools_without_installed_metadata_are_not_health() {
        // The runtime can discover tools even when reading meta.json fails.
        for meta in [json!({"tools":[{"name":"read_file"}]}), json!({})] {
            let snapshot = Snapshot::from_event(
                json!({"capsules":[{
                    "principal":"alice", "name":"broken", "meta":meta
                }]}),
                "alice",
                "now".into(),
            )
            .unwrap();
            assert!(snapshot.metadata("broken").is_err());
        }
    }
}
