//! Product-owned runtime layout and launcher for Unicity AOS.
//!
//! Astrid Runtime keeps its standalone `ASTRID_HOME` and `.astrid` compatibility
//! contract. AOS instead owns `~/.aos` and passes a private runtime home
//! to the bundled runtime process only; it never changes the caller's process
//! environment or rewrites a standalone runtime installation.

pub mod aos_home;
mod capsules;
pub mod distro_trust;
mod fs_validation;
pub mod health;
mod migration;
mod release_inventory;
pub mod status;
pub(crate) mod test_fixtures;

pub use aos_home::AosHome;
pub use migration::{LegacyDistro, MigrationOutcome};

pub(crate) const UNICITY_CE_MANIFEST: &str =
    include_str!("../../../distros/community/unicity-ce/Distro.toml");
pub(crate) const PRODUCT_VERSION: &str = env!("CARGO_PKG_VERSION");
pub(crate) const DISTRO_LOCK_FILE: &str = "Distro.lock";
pub(crate) const DISTRO_SIGNATURE_FILE: &str = "Distro.sig";
pub(crate) const RELEASE_MANIFEST_FILE: &str = "release-manifest.json";
pub(crate) const RELEASE_STATEMENT_DIR: &str = "signed";
pub(crate) const RELEASE_VERIFIER_DIR: &str = "verifier";
pub(crate) const RELEASE_VERIFIER_NAME: &str = "cosign";
pub(crate) const RELEASE_VERIFIER_VERSION: &str = "v3.1.1";
pub(crate) const RELEASE_REPOSITORY: &str = "unicity-aos/aos-ce";
pub(crate) const RELEASE_ISSUER: &str = "https://token.actions.githubusercontent.com";

#[cfg(windows)]
pub(crate) const RUNTIME_EXECUTABLE_NAMES: &[&str] = &[
    "astrid.exe",
    "astrid-daemon.exe",
    "astrid-build.exe",
    "astrid-emit.exe",
];

#[cfg(not(windows))]
pub(crate) const RUNTIME_EXECUTABLE_NAMES: &[&str] =
    &["astrid", "astrid-daemon", "astrid-build", "astrid-emit"];

/// Product-owned per-project state directory selected for all AOS runtime access.
pub const AOS_WORKSPACE_STATE_DIR: &str = ".aos";
