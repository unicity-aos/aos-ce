//! Product daemon identity without changing client file resolution.

use std::ffi::OsStr;
use std::io;
use std::process::Command;

use crate::AosHome;

#[cfg(test)]
mod tests;

impl AosHome {
    /// Build a command for the bundled runtime with product CLI arguments.
    ///
    /// Daemon identity is the product runtime root. The caller's directory
    /// still resolves relative file arguments and supplies MCP project context.
    /// Argument boundaries are preserved without a shell, and the runtime's
    /// workspace verification remains authoritative.
    ///
    /// # Errors
    /// Returns an error if the product root cannot be made absolute or the
    /// inherited PATH cannot be represented safely as a child PATH.
    pub fn runtime_command_with_args<I, S>(&self, args: I) -> io::Result<Command>
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        let runtime_binary = self.runtime_binary();
        let mut command =
            self.runtime_executable_command(&runtime_binary, std::iter::empty::<&OsStr>())?;
        command
            .arg("--daemon-workspace")
            .arg(std::path::absolute(self.runtime_home())?)
            .args(args);
        Ok(command)
    }
}
