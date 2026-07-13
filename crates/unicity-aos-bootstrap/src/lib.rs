//! Product-owned runtime layout and launcher for Unicity AOS.
//!
//! Astrid Runtime keeps its standalone `ASTRID_HOME` and `.astrid` compatibility
//! contract. AOS instead owns `~/.unicity-os` and passes a private runtime home
//! to the bundled runtime process only; it never changes the caller's process
//! environment or rewrites a standalone runtime installation.

use std::env;
use std::io;
use std::path::{Path, PathBuf};
use std::process::{Child, Command};

/// Product state owned by one Unicity AOS installation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AosHome {
    root: PathBuf,
}

impl AosHome {
    /// Resolve the AOS home directory.
    ///
    /// `UNICITY_AOS_HOME` is an explicit product override. Otherwise AOS uses
    /// `~/.unicity-os`, independently of Astrid Runtime's standalone home.
    ///
    /// # Errors
    /// Returns an error when neither `UNICITY_AOS_HOME` nor `HOME` is present.
    pub fn resolve() -> io::Result<Self> {
        if let Some(root) = env::var_os("UNICITY_AOS_HOME") {
            return Ok(Self::from_root(root));
        }

        let home = env::var_os("HOME").ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::NotFound,
                "UNICITY_AOS_HOME and HOME are both unset",
            )
        })?;
        Ok(Self::from_root(PathBuf::from(home).join(".unicity-os")))
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

    /// The installed bundled-runtime executable.
    #[must_use]
    pub fn runtime_binary(&self) -> PathBuf {
        self.runtime_home().join("bin").join(runtime_binary_name())
    }

    /// Create the product and bundled-runtime state directories.
    ///
    /// This intentionally creates neither a standalone Astrid home nor a
    /// project `.astrid` directory.
    ///
    /// # Errors
    /// Returns an error when the directories cannot be created.
    pub fn ensure_layout(&self) -> io::Result<()> {
        std::fs::create_dir_all(self.runtime_home())
    }

    /// Build a command for the bundled runtime with a process-local home.
    ///
    /// The `ASTRID_HOME` override is applied only to this child process. AOS
    /// therefore can bundle the neutral runtime without changing the host
    /// shell, another AOS install, or a standalone Astrid Runtime installation.
    #[must_use]
    pub fn runtime_command(&self) -> Command {
        let mut command = Command::new(self.runtime_binary());
        command.env("ASTRID_HOME", self.runtime_home());
        command
    }

    /// Spawn the bundled runtime with its AOS-owned runtime home.
    ///
    /// # Errors
    /// Returns an error when the bundled executable is absent or cannot start.
    pub fn spawn_runtime(&self) -> io::Result<Child> {
        let binary = self.runtime_binary();
        if !binary.is_file() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!(
                    "bundled runtime executable not found at {}",
                    binary.display()
                ),
            ));
        }
        self.ensure_layout()?;
        self.runtime_command().spawn()
    }
}

#[cfg(windows)]
const fn runtime_binary_name() -> &'static str {
    "astrid.exe"
}

#[cfg(not(windows))]
const fn runtime_binary_name() -> &'static str {
    "astrid"
}

#[cfg(test)]
mod tests {
    use super::AosHome;
    use std::path::PathBuf;

    #[test]
    fn runtime_is_scoped_beneath_the_product_home() {
        let home = AosHome::from_root("/tmp/unicity-aos-test");
        assert_eq!(home.root(), PathBuf::from("/tmp/unicity-aos-test"));
        assert_eq!(
            home.runtime_home(),
            PathBuf::from("/tmp/unicity-aos-test/runtime")
        );
        assert_eq!(
            home.runtime_binary(),
            PathBuf::from("/tmp/unicity-aos-test/runtime/bin/astrid")
        );
    }

    #[test]
    fn runtime_command_scopes_astrid_home_to_the_child() {
        let home = AosHome::from_root("/tmp/unicity-aos-test");
        let command = home.runtime_command();
        let runtime_home = command
            .get_envs()
            .find_map(|(name, value)| (name == "ASTRID_HOME").then_some(value))
            .flatten()
            .expect("runtime command sets ASTRID_HOME");

        assert_eq!(runtime_home, "/tmp/unicity-aos-test/runtime");
    }
}
