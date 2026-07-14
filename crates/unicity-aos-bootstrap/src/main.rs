//! `aos` — the product command surface for Unicity AOS.
//!
//! Unicity AOS is a trusted distribution built on Astrid Runtime. The product
//! binary therefore delegates runtime and operator commands directly to its
//! bundled runtime, scoped to this installation's private `ASTRID_HOME`.

use std::ffi::{OsStr, OsString};
#[cfg(unix)]
use std::fs::OpenOptions;
use std::io::{self, IsTerminal, Write};
#[cfg(unix)]
use std::os::unix::fs::OpenOptionsExt;
#[cfg(unix)]
use std::path::Path;
use std::process::{Command, ExitCode};
#[cfg(unix)]
use std::time::{SystemTime, UNIX_EPOCH};

use unicity_aos_bootstrap::AosHome;

#[cfg(unix)]
fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if let Some(exit_code) = handle_product_command(&args) {
        return exit_code;
    }
    let home = match AosHome::resolve() {
        Ok(home) => home,
        Err(error) => {
            eprintln!("aos: failed to resolve product home: {error}");
            return ExitCode::FAILURE;
        }
    };
    let args = match product_runtime_args(&home, args) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("aos: failed to prepare Unicity CE: {error}");
            return ExitCode::FAILURE;
        }
    };

    match home.exec_runtime_with_args(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("aos: failed to start bundled runtime: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(not(unix))]
fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if let Some(exit_code) = handle_product_command(&args) {
        return exit_code;
    }
    let home = match AosHome::resolve() {
        Ok(home) => home,
        Err(error) => {
            eprintln!("aos: failed to resolve product home: {error}");
            return ExitCode::FAILURE;
        }
    };
    let args = match product_runtime_args(&home, args) {
        Ok(args) => args,
        Err(error) => {
            eprintln!("aos: failed to prepare Unicity CE: {error}");
            return ExitCode::FAILURE;
        }
    };

    match home.run_runtime_with_args(args) {
        Ok(status) if status.success() => ExitCode::SUCCESS,
        Ok(status) => ExitCode::from(status.code().unwrap_or(1).clamp(1, i32::from(u8::MAX)) as u8),
        Err(error) => {
            eprintln!("aos: failed to start bundled runtime: {error}");
            ExitCode::FAILURE
        }
    }
}

fn handle_product_command(args: &[OsString]) -> Option<ExitCode> {
    match args.first().and_then(|arg| arg.to_str()) {
        None => offer_first_run_migration().or_else(|| {
            print_help();
            Some(ExitCode::SUCCESS)
        }),
        Some("-h" | "--help") => {
            print_help();
            Some(ExitCode::SUCCESS)
        }
        Some("-V" | "--version") => {
            println!("Unicity AOS {}", env!("CARGO_PKG_VERSION"));
            Some(ExitCode::SUCCESS)
        }
        Some("self-update" | "self_update") => Some(handle_self_update(&args[1..])),
        Some("migrate") => Some(handle_migrate_command(&args[1..])),
        Some("serve-health") => Some(handle_health_service()),
        Some("init") if has_distro_override(&args[1..]) => {
            eprintln!("aos init always installs Unicity CE; use `astrid init` for another distro");
            Some(ExitCode::FAILURE)
        }
        Some("init") if has_help_flag(&args[1..]) => {
            print_init_help();
            Some(ExitCode::SUCCESS)
        }
        Some(_) => None,
    }
}

fn handle_self_update(args: &[OsString]) -> ExitCode {
    if !args.is_empty() {
        eprintln!("Usage: aos self-update");
        return ExitCode::FAILURE;
    }

    if std::env::var_os("UNICITY_AOS_INSTALL_METHOD").as_deref() == Some(OsStr::new("homebrew")) {
        return command_exit_code(
            Command::new("brew")
                .args(["upgrade", "unicity-aos/tap/aos"])
                .status(),
            "run Homebrew upgrade",
        );
    }

    #[cfg(unix)]
    {
        let installer = std::env::temp_dir().join(format!(
            "unicity-aos-update-{}-{}.sh",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos()
        ));
        let create = create_private_update_file(&installer);
        if let Err(error) = create {
            eprintln!("aos: failed to stage product updater: {error}");
            return ExitCode::FAILURE;
        }

        let url = "https://aos.unicity.ai/install.sh";
        let download = Command::new("curl")
            .args(["--proto", "=https", "--tlsv1.2", "-fsSL", url, "-o"])
            .arg(&installer)
            .status();
        let download_code = command_exit_code(download, "download the product updater");
        if download_code != ExitCode::SUCCESS {
            let _ = std::fs::remove_file(&installer);
            return download_code;
        }

        let mut update = Command::new("sh");
        update
            .arg(&installer)
            .args(["--yes", "--no-migrate-prompt"])
            .env_remove("AOS_VERSION");
        if let Ok(executable) = std::env::current_exe()
            && let Some(bin_dir) = executable.parent()
        {
            update.env("AOS_BIN_DIR", bin_dir);
        }
        let status = update.status();
        let _ = std::fs::remove_file(&installer);
        command_exit_code(status, "run the product updater")
    }

    #[cfg(not(unix))]
    {
        eprintln!(
            "aos: automatic product updates are not available on this platform; install the latest AOS package"
        );
        ExitCode::FAILURE
    }
}

#[cfg(unix)]
fn create_private_update_file(path: &Path) -> io::Result<std::fs::File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
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

fn handle_health_service() -> ExitCode {
    let home = match AosHome::resolve() {
        Ok(home) => home,
        Err(error) => {
            eprintln!("aos: failed to resolve product home: {error}");
            return ExitCode::FAILURE;
        }
    };

    // Safety: this runs before the Tokio runtime and the health server starts
    // any threads. The override affects only this dedicated child process, not
    // the invoking shell or a standalone Astrid Runtime installation.
    unsafe {
        std::env::set_var("ASTRID_HOME", home.runtime_home());
    }

    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("aos: failed to start product health runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(unicity_aos_bootstrap::health::serve_default()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("aos: health service failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn product_runtime_args(home: &AosHome, args: Vec<OsString>) -> io::Result<Vec<OsString>> {
    if args.first().is_some_and(|arg| arg == "init") {
        let mut runtime_args = vec![
            OsString::from("init"),
            OsString::from("--distro"),
            home.ensure_unicity_ce_manifest()?.into_os_string(),
        ];
        runtime_args.extend(args.into_iter().skip(1));
        Ok(runtime_args)
    } else {
        Ok(args)
    }
}

fn has_distro_override(args: &[OsString]) -> bool {
    args.iter().any(|arg| {
        arg.as_os_str() == OsStr::new("--distro")
            || arg.to_str().is_some_and(|arg| arg.starts_with("--distro="))
    })
}

fn has_help_flag(args: &[OsString]) -> bool {
    args.iter()
        .any(|arg| matches!(arg.to_str(), Some("-h" | "--help")))
}

fn offer_first_run_migration() -> Option<ExitCode> {
    if !io::stdin().is_terminal() {
        return None;
    }
    let home = AosHome::resolve().ok()?;
    if home.migration_receipt().is_file() {
        return None;
    }
    let source = AosHome::default_legacy_runtime_home().ok()?;
    if !source.is_dir() {
        return None;
    }

    println!(
        "Found a standalone Astrid Runtime home at {}.",
        source.display()
    );
    println!(
        "Unicity can copy compatible runtime state into {}. The existing home will stay unchanged.",
        home.runtime_home().display()
    );
    print!("Import it now? [y/N] ");
    io::stdout().flush().ok()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer).ok()?;
    if !matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes") {
        println!(
            "Skipped. You can import later with `aos migrate runtime --from {}`.",
            source.display()
        );
        return Some(ExitCode::SUCCESS);
    }

    match home.migrate_runtime_from(&source) {
        Ok(unicity_aos_bootstrap::MigrationOutcome::Migrated) => {
            println!(
                "Unicity AOS: imported the standalone runtime; the source was left unchanged."
            );
            print_legacy_distro_handoff(&home);
            Some(ExitCode::SUCCESS)
        }
        Ok(unicity_aos_bootstrap::MigrationOutcome::AlreadyMigrated) => Some(ExitCode::SUCCESS),
        Err(error) => {
            eprintln!("aos: runtime migration failed: {error}");
            Some(ExitCode::FAILURE)
        }
    }
}

fn handle_migrate_command(args: &[OsString]) -> ExitCode {
    let [subcommand, flag, source] = args else {
        eprintln!("Usage: aos migrate runtime --from <absolute-legacy-home>");
        return ExitCode::FAILURE;
    };
    if subcommand.as_os_str() != OsStr::new("runtime") || flag.as_os_str() != OsStr::new("--from") {
        eprintln!("Usage: aos migrate runtime --from <absolute-legacy-home>");
        return ExitCode::FAILURE;
    }

    let home = match AosHome::resolve() {
        Ok(home) => home,
        Err(error) => {
            eprintln!("aos: failed to resolve product home: {error}");
            return ExitCode::FAILURE;
        }
    };
    match home.migrate_runtime_from(std::path::Path::new(source)) {
        Ok(unicity_aos_bootstrap::MigrationOutcome::Migrated) => {
            println!(
                "Unicity AOS: imported the standalone runtime; the source was left unchanged."
            );
            print_legacy_distro_handoff(&home);
            ExitCode::SUCCESS
        }
        Ok(unicity_aos_bootstrap::MigrationOutcome::AlreadyMigrated) => {
            println!("Unicity AOS: this runtime migration is already complete.");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("aos: runtime migration failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn print_legacy_distro_handoff(home: &AosHome) {
    let distros = match home.imported_legacy_distros() {
        Ok(distros) => distros,
        Err(error) => {
            eprintln!("aos: migrated runtime, but could not read the migration receipt: {error}");
            return;
        }
    };
    if !distros.is_empty() {
        println!(
            "Imported legacy distro state was preserved. Run `aos init` to deliberately apply Unicity CE; provider configuration and imported state remain in place."
        );
    }
}

fn print_help() {
    println!(
        "Unicity AOS\n\nUsage:\n  aos init [--yes] [--offline] [--allow-unsigned] [--accept-new-key] [--var KEY=VALUE]\n  aos migrate runtime --from <absolute-legacy-home>\n  aos self-update\n  aos serve-health\n  aos <runtime command> [arguments...]\n\n`aos init` installs the Unicity CE manifest bundled with this product release. `aos self-update` updates the coordinated AOS and bundled Astrid executable set. `aos serve-health` binds only 127.0.0.1:8765 and exposes GET /v1/runtime/health. Unicity delegates runtime and operator commands to its bundled Astrid Runtime. The runtime state is scoped to ~/.unicity-os/runtime (or UNICITY_AOS_HOME)."
    );
}

fn print_init_help() {
    println!(
        "Unicity AOS\n\nUsage:\n  aos init [--yes] [--offline] [--allow-unsigned] [--accept-new-key] [--var KEY=VALUE]\n\nInstalls Unicity CE from the manifest bundled with this product release. For a different distro, use the Astrid Runtime CLI directly."
    );
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[cfg(unix)]
    use super::create_private_update_file;
    use super::{has_distro_override, product_runtime_args};
    use unicity_aos_bootstrap::AosHome;

    fn temporary_home() -> std::path::PathBuf {
        std::env::temp_dir().join(format!(
            "unicity-aos-product-init-{}-{}",
            std::process::id(),
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("clock after epoch")
                .as_nanos()
        ))
    }

    #[test]
    fn product_init_pins_unicity_ce_and_preserves_flags() {
        let root = temporary_home();
        let home = AosHome::from_root(&root);
        let args = product_runtime_args(
            &home,
            vec![
                OsString::from("init"),
                OsString::from("--yes"),
                OsString::from("--var"),
                OsString::from("model=gpt-5"),
            ],
        )
        .expect("materialize product manifest");
        assert_eq!(
            [&args[0], &args[1], &args[3], &args[4], &args[5]],
            ["init", "--distro", "--yes", "--var", "model=gpt-5"]
        );
        assert_eq!(
            args[2],
            root.join("distributions/unicity-ce/Distro.toml")
                .into_os_string()
        );
        fs::remove_dir_all(root).expect("remove temporary product home");
    }

    #[test]
    fn product_init_rejects_distro_overrides() {
        assert!(has_distro_override(&[
            OsString::from("--distro"),
            OsString::from("other")
        ]));
        assert!(has_distro_override(&[OsString::from("--distro=other")]));
        assert!(!has_distro_override(&[OsString::from("--yes")]));
    }

    #[cfg(unix)]
    #[test]
    fn product_updater_is_staged_privately() {
        let path = temporary_home();
        let file = create_private_update_file(&path).expect("create private updater");
        drop(file);
        let mode = fs::metadata(&path)
            .expect("read updater metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, 0o600);
        fs::remove_file(path).expect("remove updater");
    }
}
