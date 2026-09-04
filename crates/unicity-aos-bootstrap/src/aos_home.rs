//! Product-owned `~/.aos` state, release selection, and runtime dispatch.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus};

use crate::capsules::{capsule_assets_from_manifest, validate_capsule_dir};
use crate::fs_validation::{create_private_dir, validate_regular_file};
use crate::migration::{self, LegacyDistro, MigrationOutcome};
use crate::release_inventory::validate_path_entry;
use crate::{
    AOS_WORKSPACE_STATE_DIR, DISTRO_LOCK_FILE, DISTRO_SIGNATURE_FILE, PRODUCT_VERSION,
    RELEASE_MANIFEST_FILE, RELEASE_STATEMENT_DIR, RELEASE_VERIFIER_DIR, RELEASE_VERIFIER_NAME,
    RUNTIME_EXECUTABLE_NAMES, UNICITY_CE_MANIFEST,
};

/// Product state owned by one Unicity AOS installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AosHome {
    root: PathBuf,
}

impl AosHome {
    /// Resolve the AOS home directory.
    ///
    /// `AOS_HOME` is an explicit product override. Otherwise AOS uses
    /// `~/.aos`, independently of Astrid Runtime's standalone home.
    ///
    /// # Errors
    /// Returns an error when neither `AOS_HOME` nor `HOME` is present.
    pub fn resolve() -> io::Result<Self> {
        Self::resolve_with(|name| std::env::var_os(name))
    }

    fn resolve_with<F>(get: F) -> io::Result<Self>
    where
        F: Fn(&str) -> Option<OsString>,
    {
        if let Some(root) = get("AOS_HOME") {
            return Self::from_environment_root(root, "AOS_HOME");
        }

        let home = default_home(&get)?;
        let home = Self::validated_environment_root(home, default_home_name())?;
        Ok(Self::from_root(home.join(".aos")))
    }

    fn from_environment_root(root: OsString, variable: &str) -> io::Result<Self> {
        Ok(Self::from_root(Self::validated_environment_root(
            root, variable,
        )?))
    }

    fn validated_environment_root(root: OsString, variable: &str) -> io::Result<PathBuf> {
        if root.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{variable} must not be empty"),
            ));
        }

        let root = PathBuf::from(root);
        if !root.is_absolute() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{variable} must be an absolute path"),
            ));
        }
        validate_path_entry(&root, variable)?;
        Ok(root)
    }

    /// Build an AOS home from an explicit root, useful for embedding and tests.
    #[must_use]
    pub fn from_root(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The product-owned AOS root.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// The private home passed to the bundled Astrid Runtime process.
    #[must_use]
    pub fn runtime_home(&self) -> PathBuf {
        self.root.join("runtime")
    }

    /// The transient AOS process-coordination root reserved for a runtime that
    /// supports an explicit run-directory contract.
    ///
    /// The currently pinned runtime derives `run/` from `ASTRID_HOME`; AOS must
    /// not bridge the two layouts with a symlink or an ignored variable.
    #[must_use]
    pub fn run_root(&self) -> PathBuf {
        self.root.join("run")
    }

    /// The immutable assets installed for this exact AOS product version.
    #[must_use]
    pub fn release_dir(&self) -> PathBuf {
        self.root.join("releases").join(PRODUCT_VERSION)
    }

    /// The release-owned stable launcher input.
    #[must_use]
    pub fn release_bin_dir(&self) -> PathBuf {
        self.release_dir().join("bin")
    }

    /// The release-owned directory containing bundled Astrid executables.
    #[must_use]
    pub fn release_runtime_bin_dir(&self) -> PathBuf {
        self.release_dir().join("runtime").join("bin")
    }

    /// The installer-owned release inventory used for launch-time tamper checks.
    #[must_use]
    pub fn release_manifest_path(&self) -> PathBuf {
        self.release_dir().join(RELEASE_MANIFEST_FILE)
    }

    /// The release-owned directory holding the signed immutable-release record.
    #[must_use]
    pub fn release_statement_dir(&self) -> PathBuf {
        self.release_dir().join(RELEASE_STATEMENT_DIR)
    }

    /// The signed statement for the selected immutable release.
    #[must_use]
    pub fn release_statement_path(&self) -> PathBuf {
        self.release_statement_dir()
            .join(format!("unicity-aos-{PRODUCT_VERSION}-release.toml"))
    }

    /// The Sigstore bundle for the selected immutable release statement.
    #[must_use]
    pub fn release_statement_bundle_path(&self) -> PathBuf {
        self.release_statement_path().with_file_name(format!(
            "unicity-aos-{PRODUCT_VERSION}-release.toml.sigstore.json"
        ))
    }

    /// The release-owned directory holding the pinned Sigstore verifier.
    #[must_use]
    pub fn release_verifier_dir(&self) -> PathBuf {
        self.release_dir().join(RELEASE_VERIFIER_DIR)
    }

    /// The checksummed Sigstore verifier persisted by the installer.
    #[must_use]
    pub fn release_verifier_path(&self) -> PathBuf {
        self.release_verifier_dir().join(RELEASE_VERIFIER_NAME)
    }

    /// The stable activation pointer for this product home.
    #[must_use]
    pub fn activation_binary(&self) -> PathBuf {
        self.root.join("bin").join("aos")
    }

    /// The receipt written only after a successful standalone-runtime import.
    #[must_use]
    pub fn migration_receipt(&self) -> PathBuf {
        self.root.join("migrations/astrid-home-v1.json")
    }

    /// The exact packaged Unicity CE distribution selected for product applies.
    #[must_use]
    pub fn selected_distro_path(&self) -> PathBuf {
        self.release_dir().join("Distro.toml")
    }

    /// The packaged lock bound to the exact selected Distro.toml bytes.
    #[must_use]
    pub fn selected_distro_lock_path(&self) -> PathBuf {
        self.release_dir().join(DISTRO_LOCK_FILE)
    }

    /// The packaged signature authenticating the selected distribution lock.
    #[must_use]
    pub fn selected_distro_signature_path(&self) -> PathBuf {
        self.release_dir().join(DISTRO_SIGNATURE_FILE)
    }

    /// Product-versioned capsule assets installed alongside this AOS binary.
    ///
    /// `UNICITY_AOS_CAPSULE_DIR` is reserved for package managers which keep
    /// immutable product assets outside the mutable AOS home. The override must
    /// identify an absolute, real directory containing exactly the capsule set
    /// selected by the embedded Community Edition manifest.
    pub fn capsule_dir(&self) -> io::Result<PathBuf> {
        self.capsule_dir_with(|name| std::env::var_os(name))
    }

    pub(crate) fn capsule_dir_with<F>(&self, get: F) -> io::Result<PathBuf>
    where
        F: Fn(&str) -> Option<OsString>,
    {
        let configured = get("UNICITY_AOS_CAPSULE_DIR");
        let path = match configured {
            Some(path) => Self::validated_environment_root(path, "UNICITY_AOS_CAPSULE_DIR")?,
            None => self.release_dir().join("capsules"),
        };
        validate_capsule_dir(&path, &capsule_assets_from_manifest()?)
    }

    /// Validate the immutable packaged distribution selected for applies.
    ///
    /// # Errors
    /// Returns an error when the selected distribution is not the exact packaged
    /// Community Edition bytes or either authenticated sibling is absent.
    pub fn ensure_selected_distribution(&self) -> io::Result<()> {
        let selected = self.selected_distro_path();
        validate_regular_file(&selected, false)?;
        if fs::read(&selected)?.as_slice() != UNICITY_CE_MANIFEST.as_bytes() {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "selected distribution differs from the packaged Community Edition bytes",
            ));
        }
        validate_regular_file(&self.selected_distro_lock_path(), false)?;
        validate_regular_file(&self.selected_distro_signature_path(), false)?;
        self.capsule_dir()?;
        Ok(())
    }

    /// Apply the selected distribution through the runtime's single
    /// distribution transaction.
    ///
    /// # Errors
    /// Returns an error when authenticated release preflight fails, the
    /// selected distribution assets are unavailable, or the runtime cannot be
    /// started.
    pub fn apply_selected_distribution<I, S>(&self, args: I) -> io::Result<ExitStatus>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.ensure_runtime_available()?;
        let status = self
            .runtime_command_with_args(args)?
            .status()
            .map_err(|error| {
                io::Error::new(
                    error.kind(),
                    format!("failed to apply the selected distribution: {error}"),
                )
            })?;
        Ok(status)
    }

    /// Run one bounded AOS-owned runtime lifecycle command.
    pub fn run_runtime_lifecycle<I, S>(&self, args: I) -> io::Result<ExitStatus>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.ensure_runtime_available()?;
        self.runtime_command_with_args(args)?.status()
    }

    /// The conventional standalone Astrid Runtime home that first-run AOS can offer
    /// to import. This does not inspect `ASTRID_HOME`: an override may name another
    /// product or service installation and must be supplied explicitly by the user.
    pub fn default_legacy_runtime_home() -> io::Result<PathBuf> {
        let home = default_home(&|name| std::env::var_os(name))?;
        Ok(PathBuf::from(home).join(".astrid"))
    }

    /// The installed bundled-runtime executable.
    #[must_use]
    pub fn runtime_binary(&self) -> PathBuf {
        self.release_runtime_bin_dir().join(runtime_binary_name())
    }

    /// The daemon executable installed beside the bundled runtime CLI.
    #[must_use]
    pub fn runtime_daemon_binary(&self) -> PathBuf {
        self.runtime_binary()
            .with_file_name(runtime_daemon_binary_name())
    }

    /// Import a standalone Astrid Runtime home into this product installation.
    ///
    /// This is an explicit copy operation. It leaves the standalone source in
    /// place so the operator retains a rollback path and historical provenance.
    ///
    /// # Errors
    /// Returns an error for unsafe paths, a running source runtime, an
    /// incompatible target, or a failed staging/validation operation.
    pub fn migrate_runtime_from(&self, source: impl AsRef<Path>) -> io::Result<MigrationOutcome> {
        migration::migrate_runtime(self, source.as_ref())
    }

    /// Legacy product distro locks preserved by the last runtime import.
    ///
    /// # Errors
    /// Returns an error when no migration receipt exists or the receipt cannot be
    /// read or decoded.
    pub fn imported_legacy_distros(&self) -> io::Result<Vec<LegacyDistro>> {
        migration::imported_legacy_distros(self)
    }

    /// Create the product and bundled-runtime state directories.
    ///
    /// This intentionally creates neither a standalone Astrid home nor a
    /// project `.astrid` directory.
    ///
    /// # Errors
    /// Returns an error when the directories cannot be created.
    pub fn ensure_layout(&self) -> io::Result<()> {
        create_private_dir(&self.root)?;
        create_private_dir(&self.runtime_home())?;
        Ok(())
    }

    /// Build a command for the bundled runtime with a process-local home.
    ///
    /// The `ASTRID_HOME` override is applied only to this child process. AOS
    /// therefore can bundle the neutral runtime without changing the host
    /// shell, another AOS install, or a standalone Astrid Runtime installation.
    /// # Errors
    /// Returns an error when the private runtime bin or inherited host PATH
    /// cannot be represented safely as a child PATH.
    pub fn runtime_command(&self) -> io::Result<Command> {
        self.runtime_command_with_args(std::iter::empty::<&OsStr>())
    }

    /// Build a command for the bundled runtime with product CLI arguments.
    ///
    /// The command is executed directly, not through a shell. This preserves
    /// argument boundaries and leaves the runtime in charge of its established
    /// local socket, credentials, and operator protocol.
    /// # Errors
    /// Returns an error when the private runtime bin or inherited host PATH
    /// cannot be represented safely as a child PATH.
    pub fn runtime_command_with_args<I, S>(&self, args: I) -> io::Result<Command>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let runtime_binary = self.runtime_binary();
        self.runtime_executable_command(&runtime_binary, args)
    }

    fn runtime_executable_command<I, S>(&self, executable: &Path, args: I) -> io::Result<Command>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let runtime_bin = executable.parent().ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "bundled executable must have a parent directory",
            )
        })?;
        let mut command = Command::new(executable);
        command
            .env("ASTRID_HOME", self.runtime_home())
            .env("ASTRID_WORKSPACE_STATE_DIR", AOS_WORKSPACE_STATE_DIR)
            .env("ASTRID_ENFORCED_DISTRO", self.selected_distro_path())
            .env("ASTRID_RUN_DIR", self.run_root())
            .env(
                "ASTRID_CLIENT_CONFIG_PATH",
                self.root.join("etc/astrid/client.toml"),
            )
            .env(
                "PATH",
                Self::runtime_child_path(runtime_bin, std::env::var_os("PATH"))?,
            );
        command.args(args);
        Ok(command)
    }

    /// Build a foreground command for the bundled daemon.
    ///
    /// The command receives the same product-owned runtime home, workspace
    /// state directory, enforced distro, and `PATH` as ordinary AOS runtime
    /// dispatch. Daemon diagnostics are routed to stderr for process
    /// supervisors; this does not alter daemon lifetime.
    ///
    /// # Errors
    /// Returns an error when the bundled daemon or product capsule set is
    /// unavailable, or the child `PATH` cannot be represented safely.
    pub fn foreground_daemon_command(
        &self,
        workspace: Option<&Path>,
        verbose: bool,
    ) -> io::Result<Command> {
        self.ensure_runtime_available()?;
        let daemon_binary = self.runtime_daemon_binary();
        self.ensure_runtime_executable(&daemon_binary, "daemon")?;
        let mut args = Vec::new();
        if let Some(workspace) = workspace {
            args.push(OsString::from("--workspace"));
            args.push(workspace.as_os_str().to_owned());
        }
        if verbose {
            args.push(OsString::from("--verbose"));
        }
        let mut command = self.runtime_executable_command(&daemon_binary, args)?;
        command.env("ASTRID_DAEMON_LOG_TARGET", "stderr");
        Ok(command)
    }

    fn runtime_child_path(runtime_bin: &Path, host_path: Option<OsString>) -> io::Result<OsString> {
        let mut child_path = vec![runtime_bin.to_path_buf()];
        if let Some(host_path) = host_path {
            child_path.extend(std::env::split_paths(&host_path));
        }
        std::env::join_paths(child_path).map_err(|error| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("cannot construct the bundled runtime PATH: {error}"),
            )
        })
    }

    /// Spawn the bundled runtime with its AOS-owned runtime home.
    ///
    /// # Errors
    /// Returns an error when the bundled executable is absent or cannot start.
    pub fn spawn_runtime(&self) -> io::Result<Child> {
        self.spawn_runtime_with_args(std::iter::empty::<&OsStr>())
    }

    /// Spawn the bundled runtime with runtime CLI arguments.
    ///
    /// This path uses the runtime's normal local operator credentials. The
    /// runtime home remains scoped to this AOS installation.
    ///
    /// # Errors
    /// Returns an error when the bundled executable is absent or cannot start.
    pub fn spawn_runtime_with_args<I, S>(&self, args: I) -> io::Result<Child>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.ensure_runtime_available()?;
        self.runtime_command_with_args(args)?.spawn()
    }

    /// Replace the current Unix process with a bundled runtime command.
    ///
    /// `exec` preserves the runtime's signal and exit semantics for terminal
    /// users and service managers; it never returns on success.
    ///
    /// # Errors
    /// Returns an error when the bundled executable is absent or cannot start.
    #[cfg(unix)]
    pub fn exec_runtime_with_args<I, S>(&self, args: I) -> io::Result<()>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        use std::os::unix::process::CommandExt;

        self.ensure_runtime_available()?;
        Err(self.runtime_command_with_args(args)?.exec())
    }

    /// Replace the current Unix process with the persistent bundled daemon.
    ///
    /// This is the process-supervisor path: the daemon receives signals
    /// directly and owns the final exit status. Callers decide the workspace
    /// argument, while AOS fixes the product home, distro, and workspace-state
    /// layout.
    ///
    /// # Errors
    /// Returns an error when the bundled daemon or product assets are
    /// unavailable, or the process cannot be replaced.
    #[cfg(unix)]
    pub fn exec_foreground_daemon(
        &self,
        workspace: Option<&Path>,
        verbose: bool,
    ) -> io::Result<()> {
        use std::os::unix::process::CommandExt;

        Err(self.foreground_daemon_command(workspace, verbose)?.exec())
    }

    /// Run a bundled-runtime command.
    ///
    /// The runtime remains the authority for socket authentication and local
    /// credentials. AOS provides only product-owned installation state and
    /// preserves the runtime's exit status for scripts and service managers.
    ///
    /// # Errors
    /// Returns an error when the bundled executable is absent or cannot start.
    pub fn run_runtime_with_args<I, S>(&self, args: I) -> io::Result<ExitStatus>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.spawn_runtime_with_args(args)?.wait()
    }
}

#[cfg(windows)]
pub(crate) fn default_home<F>(get: &F) -> io::Result<OsString>
where
    F: Fn(&str) -> Option<OsString>,
{
    if let Some(home) = get("USERPROFILE") {
        return Ok(home);
    }

    match (get("HOMEDRIVE"), get("HOMEPATH")) {
        (Some(drive), Some(path)) => Ok(PathBuf::from(drive).join(path).into_os_string()),
        _ => Err(io::Error::new(
            io::ErrorKind::NotFound,
            "AOS_HOME, USERPROFILE, and HOMEDRIVE/HOMEPATH are all unset",
        )),
    }
}

#[cfg(not(windows))]
fn default_home<F>(get: &F) -> io::Result<OsString>
where
    F: Fn(&str) -> Option<OsString>,
{
    get("HOME")
        .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "AOS_HOME and HOME are both unset"))
}

#[cfg(windows)]
const fn default_home_name() -> &'static str {
    "USERPROFILE"
}

#[cfg(not(windows))]
const fn default_home_name() -> &'static str {
    "HOME"
}

pub(crate) const fn runtime_binary_name() -> &'static str {
    RUNTIME_EXECUTABLE_NAMES[0]
}

pub(crate) const fn runtime_daemon_binary_name() -> &'static str {
    RUNTIME_EXECUTABLE_NAMES[1]
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_fixtures::fixtures::*;
    use std::ffi::OsString;
    use std::io::ErrorKind;
    use std::path::PathBuf;
    #[test]
    fn runtime_command_scopes_global_and_project_state_to_aos() {
        let home = AosHome::from_root("/tmp/unicity-aos-test");
        let caller_path = std::env::var_os("PATH");
        let command = home.runtime_command().expect("build runtime command");
        let env_value = |target: &str| {
            command
                .get_envs()
                .find_map(|(name, value)| (name == target).then_some(value))
                .flatten()
                .expect("runtime command sets product-scoped environment")
        };

        assert_eq!(env_value("ASTRID_HOME"), "/tmp/unicity-aos-test/runtime");
        assert_eq!(env_value("ASTRID_WORKSPACE_STATE_DIR"), ".aos");
        assert_eq!(
            env_value("ASTRID_CLIENT_CONFIG_PATH"),
            "/tmp/unicity-aos-test/etc/astrid/client.toml"
        );
        let path_entries: Vec<_> = std::env::split_paths(env_value("PATH")).collect();
        assert_eq!(
            path_entries.first(),
            Some(&PathBuf::from(format!(
                "/tmp/unicity-aos-test/releases/{}/runtime/bin",
                env!("CARGO_PKG_VERSION")
            )))
        );
        assert_eq!(std::env::var_os("PATH"), caller_path);
    }

    #[test]
    fn runtime_command_emplaces_the_bundled_unicity_ce_distro() {
        let home = AosHome::from_root("/tmp/unicity-aos-test");
        let command = home.runtime_command().expect("build runtime command");
        let distro = command
            .get_envs()
            .find_map(|(name, value)| (name == "ASTRID_ENFORCED_DISTRO").then_some(value))
            .flatten()
            .expect("runtime command sets ASTRID_ENFORCED_DISTRO");

        assert_eq!(
            distro.to_string_lossy(),
            format!(
                "/tmp/unicity-aos-test/releases/{}/Distro.toml",
                env!("CARGO_PKG_VERSION")
            )
        );
    }

    #[test]
    fn runtime_command_forwards_product_cli_arguments_without_a_shell() {
        let home = AosHome::from_root("/tmp/unicity-aos-test");
        let command = home
            .runtime_command_with_args(["status", "--json"])
            .expect("build runtime command");
        let args: Vec<_> = command.get_args().collect();

        assert_eq!(args, ["status", "--json"]);
        assert_eq!(command.get_program(), home.runtime_binary());
    }

    #[test]
    fn foreground_daemon_uses_the_product_environment_without_ephemeral_mode() {
        let fixture = temporary_home();
        let home = AosHome::from_root(&fixture);
        install_capsule_fixtures(home.root());

        let command = home
            .foreground_daemon_command(Some(std::path::Path::new("/workspace")), true)
            .expect("build foreground daemon command");

        assert_eq!(command.get_program(), home.runtime_daemon_binary());
        assert_eq!(
            command.get_args().collect::<Vec<_>>(),
            ["--workspace", "/workspace", "--verbose"]
        );
        assert!(
            command.get_args().all(|argument| argument != "--ephemeral"),
            "foreground daemon must retain persistent lifetime"
        );
        let env_value = |target: &str| {
            command
                .get_envs()
                .find_map(|(name, value)| (name == target).then_some(value))
                .flatten()
                .expect("foreground daemon sets product environment")
        };
        assert_eq!(env_value("ASTRID_HOME"), home.runtime_home());
        assert_eq!(env_value("ASTRID_WORKSPACE_STATE_DIR"), ".aos");
        assert_eq!(
            env_value("ASTRID_CLIENT_CONFIG_PATH"),
            fixture.join("etc/astrid/client.toml")
        );
        assert_eq!(env_value("ASTRID_DAEMON_LOG_TARGET"), "stderr");
        assert_eq!(
            env_value("ASTRID_ENFORCED_DISTRO"),
            home.selected_distro_path()
        );
        fs::remove_dir_all(fixture).expect("remove foreground daemon fixture");
    }

    #[cfg(unix)]
    #[test]
    fn explicit_home_override_wins_over_the_host_home() {
        let home = AosHome::resolve_with(|name| match name {
            "AOS_HOME" => Some(OsString::from("/var/lib/aos")),
            "HOME" => Some(OsString::from("/home/operator")),
            _ => None,
        })
        .expect("absolute override resolves");

        assert_eq!(home.root(), PathBuf::from("/var/lib/aos"));
    }

    #[test]
    fn empty_or_relative_override_is_rejected() {
        for root in ["", "runtime"] {
            let error = AosHome::resolve_with(|name| match name {
                "AOS_HOME" => Some(OsString::from(root)),
                _ => None,
            })
            .expect_err("unsafe override must fail");
            assert_eq!(error.kind(), ErrorKind::InvalidInput);
        }
    }

    #[cfg(unix)]
    #[test]
    fn product_home_with_a_path_separator_is_rejected() {
        let error = AosHome::resolve_with(|name| match name {
            "AOS_HOME" => Some(OsString::from("/tmp/aos:test")),
            _ => None,
        })
        .expect_err("an unrepresentable runtime bin must fail closed");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);

        let home = AosHome::from_root("/tmp/aos:test");
        let error = home
            .runtime_command()
            .expect_err("explicit roots must fail at command construction too");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn child_path_preserves_host_entries_and_handles_an_absent_host_path() {
        let home = AosHome::from_root("/tmp/unicity-aos-test");
        let host_entries = [PathBuf::from("/usr/local/bin"), PathBuf::from("/usr/bin")];
        let host_path = std::env::join_paths(&host_entries).expect("build host PATH");
        let runtime_bin = home.release_runtime_bin_dir();
        let child_path =
            AosHome::runtime_child_path(&runtime_bin, Some(host_path)).expect("build child PATH");
        assert_eq!(
            std::env::split_paths(&child_path).collect::<Vec<_>>(),
            [
                PathBuf::from(format!(
                    "/tmp/unicity-aos-test/releases/{}/runtime/bin",
                    env!("CARGO_PKG_VERSION")
                )),
                PathBuf::from("/usr/local/bin"),
                PathBuf::from("/usr/bin"),
            ]
        );

        let child_path =
            AosHome::runtime_child_path(&runtime_bin, None).expect("build private-only child PATH");
        assert_eq!(
            std::env::split_paths(&child_path).collect::<Vec<_>>(),
            [PathBuf::from(format!(
                "/tmp/unicity-aos-test/releases/{}/runtime/bin",
                env!("CARGO_PKG_VERSION")
            ))]
        );
    }

    #[test]
    fn empty_default_home_is_rejected() {
        let error = AosHome::resolve_with(|name| match name {
            "HOME" => Some(OsString::new()),
            _ => None,
        })
        .expect_err("empty host home must fail");
        assert_eq!(error.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn packaged_distribution_is_selected_at_its_immutable_path() {
        let root = temporary_home();
        let home = AosHome::from_root(&root);
        install_capsule_fixtures(&root);
        let path = home.selected_distro_path();
        home.ensure_selected_distribution()
            .expect("select exact packaged distribution");
        assert_eq!(
            path,
            root.join(format!(
                "releases/{}/Distro.toml",
                env!("CARGO_PKG_VERSION")
            ))
        );
        assert_eq!(fs::read(&path).unwrap(), UNICITY_CE_MANIFEST.as_bytes());

        fs::write(&path, "tampered").expect("tamper selected distribution");
        let error = home
            .ensure_selected_distribution()
            .expect_err("rewritten selected distribution must fail closed");
        assert_eq!(error.kind(), ErrorKind::InvalidData);
        fs::remove_dir_all(root).expect("remove temporary product home");
    }

    #[test]
    fn selected_distribution_requires_authenticated_siblings() {
        let root = temporary_home();
        let home = AosHome::from_root(&root);
        install_capsule_fixtures(&root);
        fs::remove_file(home.selected_distro_lock_path()).expect("remove lock");
        let error = home
            .ensure_selected_distribution()
            .expect_err("missing lock must fail closed");
        assert_eq!(error.kind(), ErrorKind::NotFound);

        fs::write(home.selected_distro_lock_path(), b"fixture lock").expect("restore lock");
        set_private_file_permissions(&home.selected_distro_lock_path())
            .expect("restore private lock mode");
        fs::remove_file(home.selected_distro_signature_path()).expect("remove signature");
        let error = home
            .ensure_selected_distribution()
            .expect_err("missing signature must fail closed");
        assert_eq!(error.kind(), ErrorKind::NotFound);
        fs::remove_dir_all(root).expect("remove temporary product home");
    }

    #[cfg(unix)]
    #[test]
    fn product_layout_and_selected_distribution_are_private() {
        use std::os::unix::fs::PermissionsExt;

        let root = temporary_home();
        let home = AosHome::from_root(&root);
        install_capsule_fixtures(&root);
        let selected = home.selected_distro_path();

        for directory in [&root, &home.release_dir()] {
            assert_eq!(
                fs::metadata(directory)
                    .expect("read directory metadata")
                    .permissions()
                    .mode()
                    & 0o777,
                0o700
            );
        }
        assert!(
            !home.runtime_home().exists(),
            "selection must not create private runtime state or executable copies"
        );
        assert_eq!(
            fs::metadata(selected)
                .expect("read selected distribution metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        fs::remove_dir_all(root).expect("remove temporary product home");
    }

    #[cfg(unix)]
    #[test]
    fn selected_distribution_refuses_a_symlink() {
        use std::os::unix::fs::symlink;

        let root = temporary_home();
        let home = AosHome::from_root(&root);
        install_capsule_fixtures(&root);
        let selected = home.selected_distro_path();
        fs::remove_file(&selected).expect("remove packaged distribution before symlink test");
        let external = root.join("outside.toml");
        fs::write(&external, UNICITY_CE_MANIFEST).expect("write external distribution");
        symlink(&external, &selected).expect("symlink selected distribution");

        assert_eq!(
            home.ensure_selected_distribution()
                .expect_err("selected distribution symlink must fail closed")
                .kind(),
            ErrorKind::InvalidInput
        );
        assert!(selected.is_symlink());
        fs::remove_dir_all(root).expect("remove temporary product home");
    }

    #[test]
    fn bundled_distro_version_matches_the_product_release() {
        let manifest: toml::Value = UNICITY_CE_MANIFEST.parse().expect("parse bundled manifest");
        assert_eq!(
            manifest["distro"]["version"].as_str(),
            Some(env!("CARGO_PKG_VERSION")),
            "the product binary and bundled Unicity CE manifest must release together"
        );
    }

    #[test]
    fn bundled_distro_uses_product_project_state() {
        let manifest: toml::Value = UNICITY_CE_MANIFEST.parse().expect("parse bundled manifest");
        let capsules = manifest["capsule"]
            .as_array()
            .expect("manifest capsule array");
        let cwd_dirs: Vec<_> = capsules
            .iter()
            .filter_map(|capsule| capsule.get("env"))
            .filter_map(|env| env.get("cwd_dir"))
            .filter_map(toml::Value::as_str)
            .collect();
        assert!(!cwd_dirs.is_empty(), "fixture must exercise project state");
        assert!(
            cwd_dirs.iter().all(|path| *path == ".aos"),
            "product capsules must not create Astrid-branded project state"
        );
    }

    #[test]
    fn missing_migration_receipt_is_reported_to_callers() {
        let root = temporary_home();
        let home = AosHome::from_root(&root);

        let error = home
            .imported_legacy_distros()
            .expect_err("missing receipt must not look like an empty import");
        assert_eq!(error.kind(), ErrorKind::NotFound);
    }
}
