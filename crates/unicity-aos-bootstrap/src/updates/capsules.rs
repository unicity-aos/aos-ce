//! Principal-scoped capsule discovery stays out of the machine-wide cache.
use super::{Inventory, Item, process};
use serde::Deserialize;
use std::{process::Command, time::Duration};
use unicity_aos_bootstrap::AosHome;

#[derive(Deserialize)]
struct RuntimeInventory {
    schema_version: u32,
    principal: String,
    items: Vec<RuntimeItem>,
}
#[derive(Deserialize)]
struct RuntimeItem {
    name: String,
    installed_version: String,
    wasm_hash: Option<String>,
    candidate_version: Option<String>,
    availability: String,
    message: String,
}

pub(super) fn check(principal: &str) -> Result<Inventory, String> {
    let principal =
        astrid_core::PrincipalId::new(principal.to_owned()).map_err(|e| e.to_string())?;
    let binary = std::env::current_exe().map_err(|e| e.to_string())?;
    let owned = process::capture(
        Command::new(&binary).args(["principals", "--json"]),
        Duration::from_secs(30),
    )?;
    let owned: crate::principals::OwnedDiscovery = serde_json::from_slice(&owned)
        .map_err(|_| "Owned-principal discovery is unavailable".to_owned())?;
    if owned.scope != "owned"
        || owned.authority != "discovery"
        || !owned
            .principals
            .iter()
            .any(|p| p.id == principal.as_str() && p.enabled)
    {
        return Err("This principal is not in the local operator's owned directory".into());
    }
    let bytes = process::capture(
        Command::new(binary).args([
            "--principal",
            principal.as_str(),
            "capsule",
            "update",
            "--check",
            "--json",
        ]),
        Duration::from_mins(5),
    )?;
    let runtime: RuntimeInventory = serde_json::from_slice(&bytes)
        .map_err(|_| "The bundled runtime does not support capsule update discovery".to_owned())?;
    if runtime.schema_version != 1 || runtime.principal != principal.as_str() {
        return Err("Capsule update response does not match the selected principal".into());
    }
    let home = AosHome::resolve().map_err(|e| e.to_string())?;
    let locked = if let Ok(verified) = crate::distro_trust::verify_selected_release(&home) {
        std::fs::read(home.release_dir().join("Distro.lock"))
            .ok()
            .filter(|bytes| lock_matches(bytes, &verified.lock_blake3))
            .and_then(|bytes| String::from_utf8(bytes).ok())
            .and_then(|text| text.parse::<toml::Value>().ok())
    } else {
        None
    };
    let items = runtime.items.into_iter().map(|entry| {
        let member = locked.as_ref().and_then(|l| l.get("capsule")).and_then(toml::Value::as_array).and_then(|entries| entries.iter().find(|c| c.get("name").and_then(toml::Value::as_str) == Some(entry.name.as_str())));
        let managed = member.is_some_and(|m| m.get("version").and_then(toml::Value::as_str) == Some(entry.installed_version.as_str()) && entry.wasm_hash.as_ref().is_some_and(|hash| m.get("hash").and_then(toml::Value::as_str) == Some(format!("blake3:{hash}").as_str())));
        let message = if managed { "Matches the verified AOS distribution. Update it together with AOS; no independent override is offered.".into() }
        else if member.is_some() { "This capsule differs from its distribution record. Review its ownership before updating; it is excluded from Update All.".into() }
            else if locked.is_none() { "Distribution ownership could not be verified. This capsule is excluded from automatic updates; repair the signed AOS installation before deciding ownership.".into() }
            else { format!("{} Review with: aos --principal {} capsule update {}. New capabilities or publisher trust require approval.", entry.message, principal, entry.name) };
        Item { id: format!("capsule:{}:{}", principal, entry.name), name: entry.name, installed_version: entry.installed_version,
            candidate_version: if managed { None } else { entry.candidate_version }, availability: if managed { "managed".into() } else { entry.availability },
            verification: if managed { "distribution".into() } else { "unverified".into() }, action: "review".into(), message, artifact_sha256: None, candidate_digest: None }
    }).collect();
    Ok(Inventory {
        schema_version: 1,
        channel: "stable".into(),
        checked_at: Some(super::now()),
        items,
    })
}

fn lock_matches(bytes: &[u8], verified_digest: &str) -> bool {
    format!("blake3:{}", blake3::hash(bytes).to_hex()) == verified_digest
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reread_lock_must_match_the_verified_prefixed_digest() {
        let bytes = b"authenticated lock";
        let digest = format!("blake3:{}", blake3::hash(bytes).to_hex());
        assert!(lock_matches(bytes, &digest));
        assert!(!lock_matches(b"replaced lock", &digest));
        assert!(!lock_matches(bytes, blake3::hash(bytes).to_hex().as_str()));
    }
}
