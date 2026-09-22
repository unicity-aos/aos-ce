//! Oracle owns release verification and host registration. AOS only consumes
//! its installed, signed snapshot's updater; it never executes a fetched script.
use super::{Item, process};
use serde::Deserialize;
use std::{fs, path::PathBuf, process::Command, time::Duration};
use unicity_aos_bootstrap::AosHome;

#[derive(Deserialize)]
#[serde(rename_all = "kebab-case")]
struct Receipt {
    schema_version: u32,
    oracle_version: String,
    host: String,
    source: String,
    plugin_snapshot: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Metadata {
    schema_version: u32,
    kind: String,
    version: String,
    plugin_blake3: String,
    verification: String,
}

fn installed(home: &AosHome, host: &str) -> Result<(String, PathBuf), String> {
    if !["codex", "claude", "grok"].contains(&host) {
        return Err("Unknown Oracle host".into());
    }
    let root = home.root().join("extensions/oracles");
    let registration = root.join(host).join("PluginRegistration.toml");
    let registered = registration.try_exists().map_err(|e| e.to_string())?;
    let path = if registered {
        registration
    } else {
        root.join(host).join("current/Receipt.toml")
    };
    let canonical = path.canonicalize().map_err(|e| e.to_string())?;
    use std::io::Read;
    let mut bytes = Vec::new();
    fs::File::open(&canonical)
        .map_err(|e| e.to_string())?
        .take(16_385)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 16_384 {
        return Err("Oracle receipt is too large".into());
    }
    let receipt: Receipt = toml::from_str(std::str::from_utf8(&bytes).map_err(|e| e.to_string())?)
        .map_err(|e| e.to_string())?;
    semver::Version::parse(&receipt.oracle_version).map_err(|e| e.to_string())?;
    if receipt.schema_version != 1
        || receipt.host != host
        || receipt.source != "release"
        || receipt.plugin_snapshot != format!("../../../plugins/{}", receipt.oracle_version)
        || (!registered
            && canonical
                != root
                    .join(host)
                    .join("releases")
                    .join(&receipt.oracle_version)
                    .join("Receipt.toml")
                    .canonicalize()
                    .map_err(|e| e.to_string())?)
    {
        return Err("Oracle receipt does not identify a signed installed host snapshot".into());
    }
    let script = root
        .join("plugins")
        .join(&receipt.oracle_version)
        .join("install.sh");
    // Only `current` is a designed symlink. Resolve the home once (macOS /tmp
    // itself is an alias), then reject aliases beneath the installation root.
    let canonical_home = home.root().canonicalize().map_err(|e| e.to_string())?;
    let expected_root = canonical_home.join("extensions/oracles");
    if root.canonicalize().map_err(|e| e.to_string())? != expected_root
        || (registered && canonical != expected_root.join(host).join("PluginRegistration.toml"))
        || script.parent().and_then(|p| p.canonicalize().ok())
            != Some(expected_root.join("plugins").join(&receipt.oracle_version))
    {
        return Err("Oracle installation contains aliased directories".into());
    }
    Ok((receipt.oracle_version, script))
}

fn command(script: &std::path::Path) -> Result<Command, String> {
    let metadata = fs::symlink_metadata(script).map_err(|_| "This Oracle snapshot predates in-app updates. Rerun the public Oracle installer to upgrade it.".to_owned())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err("Oracle updater must be a regular installed file".into());
    }
    let mut command = Command::new("sh");
    command.arg(script);
    // GUI updates use the canonical production repository, not an ambient
    // development assets directory or repository override inherited by a shell.
    command
        .env_remove("AOS_ORACLE_ASSETS")
        .env_remove("AOS_ORACLES_REPO")
        .env_remove("AOS_ORACLES_VERSION");
    Ok(command)
}

fn discover(script: &std::path::Path) -> Result<Metadata, String> {
    let bytes = process::capture(
        command(script)?.args(["--check", "--json"]),
        Duration::from_mins(5),
    )?;
    let metadata: Metadata = serde_json::from_slice(&bytes)
        .map_err(|_| "Oracle updater returned invalid metadata".to_owned())?;
    semver::Version::parse(&metadata.version).map_err(|e| e.to_string())?;
    if metadata.schema_version != 1
        || metadata.kind != "oracle"
        || metadata.verification != "metadata"
        || metadata.plugin_blake3.len() != 64
        || !metadata
            .plugin_blake3
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err("Invalid verified Oracle metadata".into());
    }
    Ok(metadata)
}

pub(super) fn check(home: &AosHome) -> Vec<Item> {
    let mut items = Vec::new();
    let mut latest: Option<Result<Metadata, String>> = None;
    for host in ["codex", "claude", "grok"] {
        if !home
            .root()
            .join("extensions/oracles")
            .join(host)
            .join("PluginRegistration.toml")
            .exists()
            && !home
                .root()
                .join("extensions/oracles")
                .join(host)
                .join("current")
                .exists()
        {
            continue;
        }
        let mut item = Item {
            id: format!("oracle:{host}"),
            name: format!("Oracle for {host}"),
            installed_version: "unknown".into(),
            candidate_version: None,
            availability: "unknown".into(),
            verification: "none".into(),
            action: "none".into(),
            message: String::new(),
            artifact_sha256: None,
            candidate_digest: None,
        };
        match installed(home, host) {
            Ok((version, script)) => {
                item.installed_version = version.clone();
                if let Err(message) = command(&script) {
                    item.availability = "unsupported".into();
                    item.message = message;
                } else {
                    let result = latest.get_or_insert_with(|| discover(&script));
                    match result {
                        Ok(metadata) => {
                            let newer = semver::Version::parse(&metadata.version).ok()
                                > semver::Version::parse(&version).ok();
                            item.candidate_version = Some(metadata.version.clone());
                            item.availability = if newer {
                                "available"
                            } else if version == metadata.version {
                                "current"
                            } else {
                                "ahead"
                            }
                            .into();
                            item.action = if newer { "apply" } else { "none" }.into();
                            item.verification = "metadata".into();
                            // This field is reserved for SHA-256; Oracle's
                            // authenticated BLAKE3 identity is kept separately.
                            item.candidate_digest =
                                Some(format!("blake3:{}", metadata.plugin_blake3));
                            item.message = "Installed plugin snapshot; activation in an existing coding session is not observable. Updating registers the new snapshot and may require a new session.".into();
                        }
                        Err(message) => {
                            item.availability = "failed".into();
                            item.message = message.clone();
                        }
                    }
                }
            }
            Err(message) => {
                item.availability = "failed".into();
                item.message = message;
            }
        }
        items.push(item);
    }
    items
}

pub(super) fn apply(home: &AosHome, item: &Item) -> Result<(), String> {
    let host = item
        .id
        .strip_prefix("oracle:")
        .ok_or_else(|| "Not an Oracle selection".to_owned())?;
    let (installed_version, script) = installed(home, host)?;
    if installed_version != item.installed_version {
        return Err("Oracle installation changed; check again".into());
    }
    let current = discover(&script)?;
    if semver::Version::parse(&current.version).map_err(|e| e.to_string())?
        <= semver::Version::parse(&installed_version).map_err(|e| e.to_string())?
    {
        return Err(
            "Oracle candidate is not newer than the registered version; check again".into(),
        );
    }
    let digest = format!("blake3:{}", current.plugin_blake3);
    if item.candidate_version.as_deref() != Some(&current.version)
        || item.candidate_digest.as_deref() != Some(&digest)
    {
        return Err("Oracle release changed since discovery; check again".into());
    }
    process::run(
        command(&script)?
            .args([
                "--plugins-only",
                "--no-install-aos",
                "--yes",
                "--host",
                host,
                "--oracle-version",
                &current.version,
            ])
            .env("AOS_EXPECTED_PLUGIN_BLAKE3", current.plugin_blake3),
        Duration::from_mins(15),
    )?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_is_distinct_from_pack_and_rejects_aliased_snapshot() {
        let temp =
            std::env::temp_dir().join(format!("aos-oracle-registration-{}", uuid::Uuid::new_v4()));
        let home = AosHome::from_root(&temp);
        let root = temp.join("extensions/oracles");
        fs::create_dir_all(root.join("codex")).unwrap();
        fs::create_dir_all(root.join("plugins/2026.9.3")).unwrap();
        fs::write(root.join("plugins/2026.9.3/install.sh"), "#!/bin/sh\n").unwrap();
        fs::write(root.join("codex/PluginRegistration.toml"), "schema-version = 1\noracle-version = \"2026.9.3\"\nhost = \"codex\"\nsource = \"release\"\nplugin-snapshot = \"../../../plugins/2026.9.3\"\n").unwrap();
        assert_eq!(installed(&home, "codex").unwrap().0, "2026.9.3");
        assert!(!root.join("codex/current").exists());
        #[cfg(unix)]
        {
            fs::rename(root.join("plugins/2026.9.3"), root.join("plugins/alias")).unwrap();
            std::os::unix::fs::symlink("alias", root.join("plugins/2026.9.3")).unwrap();
            assert!(installed(&home, "codex").is_err());
        }
        fs::remove_dir_all(temp).unwrap();
    }
}
