//! One update inventory shared by the native and terminal Command Centers.
//! Cached discovery is presentation, never installation authority.
mod capsules;
mod oracle;
mod process;

use clap::{Args, Subcommand};
use fs2::FileExt;
use serde::{Deserialize, Serialize};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, ExitCode},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use unicity_aos_bootstrap::AosHome;

#[derive(Args)]
pub(crate) struct Arguments {
    #[command(subcommand)]
    command: Action,
}

#[derive(Subcommand)]
enum Action {
    /// Refresh an opted-in channel at most daily; safe for background UI use.
    Refresh {
        #[arg(long)]
        json: bool,
    },
    /// Check capsules visible to one owned principal without starting the runtime.
    Capsules {
        #[arg(long)]
        principal: String,
        #[arg(long)]
        json: bool,
    },
    /// Read the last check immediately, without network or runtime startup.
    List {
        #[arg(long)]
        json: bool,
    },
    /// Refresh signed metadata; never install software or start the runtime.
    Check {
        #[arg(long, value_enum)]
        channel: Option<super::UpdateChannel>,
        #[arg(long)]
        json: bool,
    },
    /// Apply the reviewed candidate through its existing signed installer.
    Apply {
        /// Item identifier from `updates list`, or `all`.
        selection: String,
        #[arg(long)]
        yes: bool,
        #[arg(long)]
        json: bool,
    },
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Inventory {
    pub schema_version: u32,
    pub channel: String,
    pub checked_at: Option<u64>,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Item {
    pub id: String,
    pub name: String,
    pub installed_version: String,
    pub candidate_version: Option<String>,
    pub availability: String,
    pub verification: String,
    pub action: String,
    pub message: String,
    pub artifact_sha256: Option<String>,
    #[serde(default)]
    pub candidate_digest: Option<String>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ChannelMetadata {
    schema_version: u32,
    kind: String,
    installed_version: String,
    channel_version: String,
    channel: String,
    target: String,
    artifact_sha256: String,
    verification: String,
    #[serde(default)]
    binds_candidate_digest: bool,
}

fn initial() -> Inventory {
    Inventory {
        schema_version: 1,
        channel: "stable".into(),
        checked_at: None,
        items: vec![Item {
            id: "aos".into(),
            name: "AOS".into(),
            installed_version: env!("CARGO_PKG_VERSION").into(),
            candidate_version: None,
            availability: "unknown".into(),
            verification: "none".into(),
            action: "check".into(),
            message: "Check for updates. Bundled Astrid and distribution capsules update with AOS."
                .into(),
            artifact_sha256: None,
            candidate_digest: None,
        }],
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn state_path(home: &AosHome) -> PathBuf {
    home.root().join("update/command-center")
}
fn error(message: impl Into<String>) -> String {
    message.into()
}

fn read_json<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    let metadata = fs::symlink_metadata(path).map_err(|e| e.to_string())?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(error("Update state must be a regular file"));
    }
    let mut data = Vec::new();
    fs::File::open(path)
        .map_err(|e| e.to_string())?
        .take(65_537)
        .read_to_end(&mut data)
        .map_err(|e| e.to_string())?;
    if data.len() > 65_536 {
        return Err(error("Update state is too large"));
    }
    serde_json::from_slice(&data).map_err(|e| e.to_string())
}

pub(crate) fn list() -> Result<Inventory, String> {
    let home = AosHome::resolve().map_err(|e| e.to_string())?;
    let path = state_path(&home).join("inventory.json");
    if !path.try_exists().map_err(|e| e.to_string())? {
        return Ok(initial());
    }
    let mut inventory: Inventory = read_json(&path)?;
    if inventory.schema_version != 1
        || !["stable", "dev", "nightly"].contains(&inventory.channel.as_str())
        || inventory.items.len() > 4
        || inventory.items.iter().any(|i| {
            !["aos", "oracle:codex", "oracle:claude", "oracle:grok"].contains(&i.id.as_str())
        })
        || inventory
            .items
            .iter()
            .map(|i| &i.id)
            .collect::<std::collections::HashSet<_>>()
            .len()
            != inventory.items.len()
    {
        return Err(error("Unsupported update inventory; check again"));
    }
    for item in &mut inventory.items {
        if inventory
            .checked_at
            .is_some_and(|at| now().saturating_sub(at) >= 86_400)
            && ["available", "current", "ahead"].contains(&item.availability.as_str())
        {
            item.availability = "stale".into();
            item.action = "check".into();
            item.message =
                "This result is more than a day old. Check again before choosing an update.".into();
        }
        if item.id == "aos" && item.installed_version != env!("CARGO_PKG_VERSION") {
            item.installed_version = env!("CARGO_PKG_VERSION").into();
            item.availability = "unknown".into();
            item.action = "check".into();
            item.message = "Installed version changed; check again".into();
        }
    }
    Ok(inventory)
}

fn private_directory(path: &Path) -> Result<(), String> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if !metadata.is_dir() || metadata.file_type().is_symlink() {
                return Err(error("Update state directory is not a private directory"));
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::MetadataExt;
                if metadata.uid() != rustix::process::geteuid().as_raw()
                    || metadata.mode() & 0o077 != 0
                {
                    return Err(error(
                        "Update state directory must belong to this user and have mode 0700",
                    ));
                }
            }
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            let mut builder = fs::DirBuilder::new();
            #[cfg(unix)]
            {
                use std::os::unix::fs::DirBuilderExt;
                builder.mode(0o700);
            }
            builder.create(path).map_err(|e| e.to_string())?;
        }
        Err(e) => return Err(e.to_string()),
    }
    Ok(())
}

fn lock(home: &AosHome) -> Result<fs::File, String> {
    // Do not create an installation as a side effect of checking for updates.
    if !home.root().is_dir() {
        return Err(error("AOS is not installed in this home"));
    }
    private_directory(home.root())?;
    private_directory(&home.root().join("update"))?;
    private_directory(&state_path(home))?;
    let path = state_path(home).join("operation.lock");
    let mut options = fs::OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options
            .mode(0o600)
            .custom_flags(rustix::fs::OFlags::NOFOLLOW.bits() as i32);
    }
    let file = options.open(path).map_err(|e| e.to_string())?;
    if !file.metadata().map_err(|e| e.to_string())?.is_file() {
        return Err(error("Invalid update lock"));
    }
    file.try_lock_exclusive()
        .map_err(|_| error("Another update operation is running; try again when it finishes"))?;
    Ok(file)
}

fn save(home: &AosHome, inventory: &Inventory) -> Result<(), String> {
    let path = state_path(home).join(format!(".inventory-{}", uuid::Uuid::new_v4()));
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut file = options.open(&path).map_err(|e| e.to_string())?;
    let result = (|| {
        file.write_all(&serde_json::to_vec(inventory).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
        fs::rename(&path, state_path(home).join("inventory.json")).map_err(|e| e.to_string())
    })();
    if result.is_err() {
        let _ = fs::remove_file(path);
    }
    result
}

fn discover(channel: &str) -> Result<Item, String> {
    let binary = std::env::current_exe().map_err(|e| e.to_string())?;
    let bytes = process::capture(
        Command::new(binary)
            .args(["update", "--check", "--json", "--channel", channel])
            .env_remove("AOS_VERSION"),
        Duration::from_secs(180),
    )?;
    let metadata: ChannelMetadata = serde_json::from_slice(&bytes).map_err(|_| error("Installed updater does not support structured checks; update using the public installer"))?;
    item_from_metadata(metadata, channel)
}

fn item_from_metadata(metadata: ChannelMetadata, channel: &str) -> Result<Item, String> {
    if metadata.schema_version != 1
        || metadata.kind != "aos"
        || metadata.channel != channel
        || metadata.verification != "metadata"
        || metadata.installed_version != env!("CARGO_PKG_VERSION")
        || metadata.target.is_empty()
        || metadata.artifact_sha256.len() != 64
        || !metadata
            .artifact_sha256
            .bytes()
            .all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase())
    {
        return Err(error("Invalid authenticated update metadata"));
    }
    let installed =
        semver::Version::parse(&metadata.installed_version).map_err(|e| e.to_string())?;
    let candidate = semver::Version::parse(&metadata.channel_version).map_err(|e| e.to_string())?;
    let availability = match candidate.cmp(&installed) {
        std::cmp::Ordering::Greater => "available",
        std::cmp::Ordering::Equal => "current",
        std::cmp::Ordering::Less => "ahead",
    };
    let homebrew = std::env::var_os("UNICITY_AOS_INSTALL_METHOD").as_deref()
        == Some(std::ffi::OsStr::new("homebrew"));
    Ok(Item { id: "aos".into(), name: "AOS".into(), installed_version: metadata.installed_version,
        candidate_version: Some(metadata.channel_version), availability: availability.into(), verification: "metadata".into(),
        action: if homebrew { "package_manager" } else if !metadata.binds_candidate_digest { "review" } else if availability == "available" { "apply" } else { "none" }.into(),
        message: if homebrew { "Managed by Homebrew. Run brew upgrade unicity-aos/tap/aos." } else if !metadata.binds_candidate_digest { "This installed updater cannot bind installation to the reviewed archive. Rerun the public installer before using in-app updates." } else { "Includes bundled Astrid and distribution capsules. Metadata authenticated; archive verification happens during installation." }.into(),
        artifact_sha256: Some(metadata.artifact_sha256), candidate_digest: None })
}

pub(crate) fn check(channel: Option<&str>) -> Result<Inventory, String> {
    let home = AosHome::resolve().map_err(|e| e.to_string())?;
    let _lock = lock(&home)?;
    let previous = list().unwrap_or_else(|_| initial());
    let channel = channel.unwrap_or(&previous.channel);
    if !["stable", "dev", "nightly"].contains(&channel) {
        return Err(error("Invalid update channel"));
    }
    let item = match discover(channel) {
        Ok(item) => item,
        Err(message) => {
            let mut item = initial().items.remove(0);
            item.availability = "failed".into();
            item.message = message;
            item
        }
    };
    let mut inventory = Inventory {
        schema_version: 1,
        channel: channel.into(),
        checked_at: Some(now()),
        items: vec![item],
    };
    inventory.items.extend(oracle::check(&home));
    save(&home, &inventory)?;
    Ok(inventory)
}

fn refresh() -> Result<Inventory, String> {
    let previous = list()?;
    // No implicit channel choice on a legacy install. The first explicit
    // check selects the channel; subsequent refreshes retain that selection.
    if previous
        .checked_at
        .is_none_or(|at| now().saturating_sub(at) < 86_400)
    {
        return Ok(previous);
    }
    check(Some(&previous.channel))
}

pub(crate) fn apply(selection: &str) -> Result<Inventory, String> {
    if !["aos", "all", "oracle:codex", "oracle:claude", "oracle:grok"].contains(&selection) {
        return Err(error("Unknown update selection"));
    }
    let home = AosHome::resolve().map_err(|e| e.to_string())?;
    let _lock = lock(&home)?;
    let mut inventory = list()?;
    let selected: Vec<usize> = inventory
        .items
        .iter()
        .enumerate()
        .filter(|(_, i)| i.action == "apply" && (selection == "all" || selection == i.id))
        .map(|(index, _)| index)
        .collect();
    if selected.is_empty() {
        return Err(error("No reviewed updates selected; check first"));
    }
    for index in selected {
        let reviewed = &inventory.items[index];
        let result = if reviewed.id == "aos" {
            apply_aos(reviewed, &inventory.channel)
        } else {
            oracle::apply(&home, reviewed)
        };
        let item = &mut inventory.items[index];
        item.action = "check".into();
        match result {
            Ok(()) => {
                item.availability = "activation_required".into();
                item.message = "Installer completed. Existing runtime and host sessions may still use the previous version; reconnect and verify activation.".into();
            }
            Err(message) => {
                item.availability = "failed".into();
                item.message = message;
            }
        }
        save(&home, &inventory)?;
        // A failed product update must not be followed by a plugin requiring
        // that product. Independent host failures remain separate results.
        if inventory.items[index].id == "aos" && inventory.items[index].availability == "failed" {
            break;
        }
    }
    Ok(inventory)
}

fn apply_aos(reviewed: &Item, channel: &str) -> Result<(), String> {
    let current = discover(channel)?;
    if current.action != "apply"
        || current.candidate_version != reviewed.candidate_version
        || current.artifact_sha256 != reviewed.artifact_sha256
    {
        return Err(error(
            "Candidate changed since discovery; check again before applying",
        ));
    }
    let version = current
        .candidate_version
        .as_deref()
        .ok_or_else(|| error("Missing candidate version"))?;
    let binary = std::env::current_exe().map_err(|e| e.to_string())?;
    process::run(
        Command::new(binary)
            .args(["update", "--version", version])
            .env(
                "AOS_EXPECTED_ARTIFACT_SHA256",
                current.artifact_sha256.as_deref().unwrap_or_default(),
            ),
        Duration::from_mins(15),
    )?;
    Ok(())
}

pub(crate) fn run(args: Arguments) -> ExitCode {
    let (result, json) = match args.command {
        Action::Refresh { json } => (refresh(), json),
        Action::Capsules { principal, json } => (capsules::check(&principal), json),
        Action::List { json } => (list(), json),
        Action::Check { channel, json } => (check(channel.map(super::UpdateChannel::as_str)), json),
        Action::Apply {
            selection,
            yes,
            json,
        } => (
            if yes {
                apply(&selection)
            } else {
                Err(error(
                    "Applying updates requires --yes. Active sessions may need reconnection.",
                ))
            },
            json,
        ),
    };
    match result {
        Ok(inventory) => {
            if json {
                println!(
                    "{}",
                    serde_json::to_string(&inventory).expect("serializable inventory")
                );
            } else {
                for item in &inventory.items {
                    println!(
                        "{} {}: {}\n  {}",
                        item.name, item.installed_version, item.availability, item.message
                    );
                }
            }
            if inventory.items.iter().any(|i| i.availability == "failed") {
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Err(message) => {
            eprintln!("aos updates: {message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn metadata(version: &str) -> ChannelMetadata {
        ChannelMetadata {
            schema_version: 1,
            kind: "aos".into(),
            installed_version: env!("CARGO_PKG_VERSION").into(),
            channel_version: version.into(),
            channel: "stable".into(),
            target: "aarch64-apple-darwin".into(),
            artifact_sha256: "a".repeat(64),
            verification: "metadata".into(),
            binds_candidate_digest: true,
        }
    }
    #[test]
    fn compares_versions_not_inequality() {
        assert_eq!(
            item_from_metadata(metadata("2026.9.2"), "stable")
                .unwrap()
                .availability,
            "ahead"
        );
        assert_eq!(
            item_from_metadata(metadata(env!("CARGO_PKG_VERSION")), "stable")
                .unwrap()
                .availability,
            "current"
        );
        assert_eq!(
            item_from_metadata(metadata("2026.10.0"), "stable")
                .unwrap()
                .availability,
            "available"
        );
    }
    #[test]
    fn rejects_wrong_channel_and_invalid_digest() {
        assert!(item_from_metadata(metadata("2026.10.0"), "dev").is_err());
        let mut value = metadata("2026.10.0");
        value.artifact_sha256 = "bad".into();
        assert!(item_from_metadata(value, "stable").is_err());
    }
    #[test]
    fn legacy_updater_cannot_apply_an_unbound_candidate() {
        let mut value = metadata("2026.10.0");
        value.binds_candidate_digest = false;
        let item = item_from_metadata(value, "stable").unwrap();
        assert_eq!(item.availability, "available");
        assert_eq!(item.action, "review");
    }
}
