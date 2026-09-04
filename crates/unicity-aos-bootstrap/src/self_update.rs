//! Product-owned self-update command handling.

use std::process::{Command, ExitCode};

use crate::cli::{UpdateArgs, UpdateChannel};
use std::ffi::OsStr;
use std::io;
use unicity_aos_bootstrap::AosHome;

pub(crate) fn handle_self_update(args: &UpdateArgs) -> ExitCode {
    if std::env::var_os("UNICITY_AOS_INSTALL_METHOD").as_deref() == Some(OsStr::new("homebrew")) {
        if args.version.is_some()
            || matches!(
                args.channel,
                Some(UpdateChannel::Dev | UpdateChannel::Nightly)
            )
        {
            eprintln!("aos: Homebrew installations follow only the signed stable channel");
            return ExitCode::from(2);
        }
        return command_exit_code(
            Command::new("brew")
                .args(["upgrade", "unicity-aos/tap/aos"])
                .status(),
            "run Homebrew upgrade",
        );
    }

    let home = match AosHome::resolve() {
        Ok(home) => home,
        Err(error) => {
            eprintln!("aos: resolve product home for update: {error}");
            return ExitCode::FAILURE;
        }
    };
    let installer = home.root().join("libexec/install.sh");
    match std::fs::symlink_metadata(&installer) {
        Ok(metadata) if metadata.is_file() && !metadata.file_type().is_symlink() => {}
        Ok(_) => {
            eprintln!(
                "aos: trusted installed updater is not a regular file: {}",
                installer.display()
            );
            return ExitCode::FAILURE;
        }
        Err(error) => {
            eprintln!(
                "aos: trusted installed updater is unavailable at {}: {error}",
                installer.display()
            );
            return ExitCode::FAILURE;
        }
    }

    let mut command = Command::new("sh");
    command.arg(installer);
    if let Some(version) = &args.version {
        command.args(["--version", version]);
    } else {
        command.args([
            "--channel",
            args.channel.unwrap_or(UpdateChannel::Stable).as_str(),
        ]);
    }
    command.args(["--yes", "--no-migrate-prompt"]);
    command_exit_code(command.status(), "run the installed signed AOS updater")
}

fn command_exit_code(status: io::Result<std::process::ExitStatus>, operation: &str) -> ExitCode {
    match status {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => ExitCode::from(status.code().unwrap_or(1).clamp(1, i32::from(u8::MAX)) as u8),
        Err(error) => {
            eprintln!("aos: failed to {operation}: {error}");
            ExitCode::FAILURE
        }
    }
}
