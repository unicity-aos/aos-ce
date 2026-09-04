//! `aos` — the product command surface for Unicity AOS.
//!
//! Unicity AOS is a distribution built on Astrid Runtime. AOS-owned commands
//! shadow matching runtime roots; every other root passes through unchanged to
//! the bundled runtime under the product-owned home and workspace layout.

use std::ffi::OsString;
use std::io::{self, IsTerminal, Write};
use std::path::Path;
use std::process::ExitCode;

use astrid_core::PrincipalId;
use clap::{CommandFactory, Parser};
use cli::{
    DaemonCommand, DistributionApplyArgs, DistroCommand, ForegroundDaemonArgs, McpCommand,
    MigrateCommand, ProductCli, ProductCommand,
};
use dispatch::{
    ambiguous_leading_principal, handle_runtime_stop, is_owned_root, leading_owned_root,
    resolve_home, runtime_args_for_dispatch, runtime_stop_requested,
};
use self_update::handle_self_update;
use unicity_aos_bootstrap::{AOS_WORKSPACE_STATE_DIR, AosHome, distro_trust, status};

mod cli;
mod dispatch;
mod hook;
mod mcp;
mod self_update;

#[cfg(unix)]
fn main() -> ExitCode {
    let args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if let Some(exit_code) = handle_product_command(&args) {
        return exit_code;
    }
    if runtime_stop_requested(&args) {
        return handle_runtime_stop(&args);
    }
    let runtime_args = runtime_args_for_dispatch(args);
    let home = match resolve_home() {
        Ok(home) => home,
        Err(code) => return code,
    };
    match home.exec_runtime_with_args(runtime_args) {
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
    if runtime_stop_requested(&args) {
        return handle_runtime_stop(&args);
    }
    let runtime_args = runtime_args_for_dispatch(args);
    let home = match resolve_home() {
        Ok(home) => home,
        Err(code) => return code,
    };
    match home.run_runtime_with_args(runtime_args) {
        Ok(status) => child_exit_code(status),
        Err(error) => {
            eprintln!("aos: failed to start bundled runtime: {error}");
            ExitCode::FAILURE
        }
    }
}

fn handle_product_command(args: &[OsString]) -> Option<ExitCode> {
    if args.is_empty() {
        return offer_first_run_migration().or_else(|| Some(print_product_help()));
    }
    if let Some(root) = ambiguous_leading_principal(args) {
        eprintln!(
            "aos: ambiguous '--principal {root}': provide an authenticated principal before the AOS-owned command, for example `aos --principal alice {root}`"
        );
        return Some(ExitCode::from(2));
    }

    let first = args.first()?.to_str()?;
    let product_invocation = matches!(first, "-h" | "--help" | "-V" | "--version")
        || (first == "help" && help_targets_product(args))
        || is_owned_root(first)
        || leading_owned_root(args).is_some();
    if !product_invocation {
        return None;
    }

    let cli = match ProductCli::try_parse_from(
        std::iter::once(OsString::from("aos")).chain(args.iter().cloned()),
    ) {
        Ok(cli) => cli,
        Err(error) => {
            let exit_code = if error.use_stderr() {
                ExitCode::from(2)
            } else {
                ExitCode::SUCCESS
            };
            if let Err(print_error) = error.print() {
                eprintln!("aos: failed to print command help: {print_error}");
                return Some(ExitCode::FAILURE);
            }
            return Some(exit_code);
        }
    };

    if cli.principal.is_some()
        && !matches!(
            &cli.command,
            Some(
                ProductCommand::Init(_)
                    | ProductCommand::Distro { command: _ }
                    | ProductCommand::Hook(_)
                    | ProductCommand::Mcp { .. }
                    | ProductCommand::Status(_)
            )
        )
    {
        eprintln!(
            "aos: '--principal' is supported for `aos distro apply`, `aos init`, `aos status`, `aos hook`, and `aos mcp`; this AOS-owned command does not accept a runtime principal"
        );
        return Some(ExitCode::from(2));
    }

    match cli.command {
        Some(ProductCommand::Init(args)) => {
            Some(prepare_distribution_apply(cli.principal.as_deref(), &args))
        }
        Some(ProductCommand::Status(args)) => {
            Some(handle_status(cli.principal, args.principal, args.json))
        }
        Some(ProductCommand::Migrate {
            command: MigrateCommand::Runtime { from },
        }) => Some(handle_migrate_runtime(&from)),
        Some(ProductCommand::Update(args)) => Some(handle_self_update(&args)),
        Some(ProductCommand::Distro {
            command: DistroCommand::Apply(args),
        }) => Some(prepare_distribution_apply(cli.principal.as_deref(), &args)),
        Some(ProductCommand::Hook(args)) => Some(handle_hook(cli.principal, args)),
        Some(ProductCommand::Mcp {
            command: McpCommand::Serve(args),
        }) => Some(handle_mcp_serve(cli.principal, args)),
        Some(ProductCommand::ServeHealth) => Some(handle_health_service()),
        Some(ProductCommand::Daemon {
            command: DaemonCommand::Foreground(args),
        }) => Some(handle_foreground_daemon(&args)),
        None => Some(print_product_help()),
    }
}

fn handle_foreground_daemon(args: &ForegroundDaemonArgs) -> ExitCode {
    let home = match resolve_home() {
        Ok(home) => home,
        Err(code) => return code,
    };
    #[cfg(unix)]
    {
        match home.exec_foreground_daemon(args.workspace.as_deref(), args.verbose) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("aos: failed to run foreground daemon: {error}");
                ExitCode::FAILURE
            }
        }
    }
    #[cfg(not(unix))]
    {
        match home
            .foreground_daemon_command(args.workspace.as_deref(), args.verbose)
            .and_then(|mut command| command.status())
        {
            Ok(status) => child_exit_code(status),
            Err(error) => {
                eprintln!("aos: failed to run foreground daemon: {error}");
                ExitCode::FAILURE
            }
        }
    }
}

fn handle_mcp_serve(principal: Option<String>, args: mcp::ServeArgs) -> ExitCode {
    let home = match resolve_home() {
        Ok(home) => home,
        Err(code) => return code,
    };
    if let Err(error) = home.ensure_runtime_available() {
        eprintln!("aos: bundled runtime inventory preflight failed: {error}");
        return ExitCode::FAILURE;
    }
    mcp::handle_serve(principal, args)
}

fn prepare_distribution_apply(principal: Option<&str>, args: &DistributionApplyArgs) -> ExitCode {
    let principal = match resolve_distribution_principal("principal", principal) {
        Ok(principal) => principal,
        Err(code) => return code,
    };

    let home = match resolve_home() {
        Ok(home) => home,
        Err(code) => return code,
    };
    let selected = home.selected_distro_path();
    if let Some(requested) = &args.distro {
        match selected_distribution_matches(&selected, requested) {
            Ok(true) => {}
            Ok(false) => {
                eprintln!(
                    "aos: only the selected Unicity CE distribution can be applied: {}",
                    selected.display()
                );
                return ExitCode::FAILURE;
            }
            Err(error) => {
                eprintln!("aos: failed to select the requested distribution: {error}");
                return ExitCode::FAILURE;
            }
        }
    }
    if let Err(error) = home.ensure_runtime_available() {
        eprintln!("aos: failed to prepare the selected distribution: {error}");
        return ExitCode::FAILURE;
    }

    let pin_key = match distro_trust::selected_signing_key(&selected) {
        Ok(key) => key,
        Err(error) => {
            eprintln!("aos: selected distribution trust identity is unavailable: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = home.run_runtime_lifecycle(["start"]) {
        eprintln!("aos: failed to mount Astrid before trust seeding: {error}");
        return ExitCode::FAILURE;
    }
    if let Err(error) = distro_trust::seed_selected_pin(&home, &selected) {
        eprintln!("aos: failed to seed the mounted trust projection: {error}");
        let shutdown = stop_distribution_apply(&home);
        if let Err(stop_error) = shutdown {
            eprintln!("aos: trust-seed shutdown failed: {stop_error}");
        }
        return ExitCode::FAILURE;
    }

    let mut runtime_args = vec![
        OsString::from("--principal"),
        OsString::from(principal.to_string()),
        OsString::from("distro"),
        OsString::from("apply"),
        selected.clone().into_os_string(),
        OsString::from("--yes"),
    ];
    if args.offline {
        runtime_args.push(OsString::from("--offline"));
    }
    for value in &args.vars {
        runtime_args.push(OsString::from("--var"));
        runtime_args.push(OsString::from(value));
    }

    let applied = home
        .apply_selected_distribution(runtime_args)
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(std::io::Error::other(format!(
                    "distribution apply exited with {}",
                    status.code().unwrap_or(1)
                )))
            }
        });
    let receipt = applied.and_then(|()| {
        stop_distribution_apply(&home).map_err(std::io::Error::other)?;
        distro_trust::write_active_receipt(&home, &selected, principal.as_str(), &pin_key)
    });

    match receipt {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            let primary = error;
            let stopped = stop_distribution_apply(&home);
            eprintln!("aos: failed to complete the distribution apply: {primary}");
            if let Err(stop_error) = stopped {
                eprintln!("aos: distribution apply shutdown failed: {stop_error}");
            }
            ExitCode::FAILURE
        }
    }
}

fn stop_distribution_apply(home: &AosHome) -> Result<(), String> {
    home.run_runtime_lifecycle(["stop"])
        .map_err(|error| format!("runtime stop failed to start: {error}"))
        .and_then(|status| {
            if status.success() {
                Ok(())
            } else {
                Err(format!(
                    "runtime stop exited with {}",
                    status.code().unwrap_or(1)
                ))
            }
        })
        .and_then(|()| status::wait_for_stopped_projection(home))
}

fn resolve_distribution_principal(
    label: &str,
    value: Option<&str>,
) -> Result<PrincipalId, ExitCode> {
    let value = value.ok_or_else(|| {
        eprintln!(
            "aos: `--principal PRINCIPAL` is required for `aos distro apply`; AOS does not select a default principal"
        );
        ExitCode::from(2)
    })?;
    PrincipalId::new(value).map_err(|error| {
        eprintln!("aos: invalid distribution {label} principal: {error}");
        ExitCode::from(2)
    })
}

fn selected_distribution_matches(selected: &Path, requested: &Path) -> io::Result<bool> {
    Ok(selected.canonicalize()? == requested.canonicalize()?)
}

fn help_targets_product(args: &[OsString]) -> bool {
    match args.get(1).and_then(|argument| argument.to_str()) {
        None => true,
        Some(root) => is_owned_root(root),
    }
}

fn handle_health_service() -> ExitCode {
    let home = match resolve_home() {
        Ok(home) => home,
        Err(code) => return code,
    };

    set_runtime_environment(&home);

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

fn handle_hook(principal: Option<String>, args: hook::HookArgs) -> ExitCode {
    let home = match resolve_home() {
        Ok(home) => home,
        Err(code) => return code,
    };
    set_runtime_environment(&home);
    let principal = principal.unwrap_or_else(|| "default".to_owned());
    match hook::handle(principal, args) {
        Ok(Some(context)) => {
            print!("{context}");
            if let Err(error) = io::stdout().flush() {
                eprintln!("aos: failed to write hook response: {error}");
                ExitCode::FAILURE
            } else {
                ExitCode::SUCCESS
            }
        }
        Ok(None) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("aos: hook delivery failed: {error}");
            ExitCode::FAILURE
        }
    }
}

fn status_principal(
    leading_principal: Option<String>,
    trailing_principal: Option<String>,
) -> Result<PrincipalId, String> {
    let principal = match (leading_principal, trailing_principal) {
        (Some(_), Some(_)) => {
            return Err(
                "'--principal' was provided both before and after `status`; provide it once"
                    .to_owned(),
            );
        }
        (Some(principal), None) | (None, Some(principal)) => Some(principal),
        (None, None) => None,
    };
    principal.map_or_else(
        || Ok(PrincipalId::default()),
        |principal| {
            PrincipalId::new(principal)
                .map_err(|error| format!("invalid status principal: {error}"))
        },
    )
}

fn handle_status(
    leading_principal: Option<String>,
    command_principal: Option<String>,
    json: bool,
) -> ExitCode {
    let principal = match status_principal(leading_principal, command_principal) {
        Ok(principal) => principal,
        Err(error) => {
            eprintln!("aos: {error}");
            return ExitCode::from(2);
        }
    };
    let home = match resolve_home() {
        Ok(home) => home,
        Err(code) => return code,
    };
    set_runtime_environment(&home);
    let runtime = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("aos: failed to start status client: {error}");
            return ExitCode::FAILURE;
        }
    };
    let status = match runtime.block_on(unicity_aos_bootstrap::status::read_for_principal(
        &home, principal,
    )) {
        Ok(status) => status,
        Err(error) => {
            eprintln!("aos: runtime status unavailable: {error}");
            return ExitCode::FAILURE;
        }
    };

    if json {
        match serde_json::to_string(&status) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("aos: failed to encode status: {error}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("Unicity AOS");
        println!("State: {}", status.state);
        println!("PID: {}", status.pid);
        println!("Uptime: {}s", status.uptime_secs);
        println!("Runtime version: {}", status.runtime_version);
        println!("Connected clients: {}", status.connected_clients);
        println!("Loaded capsules: {}", status.loaded_capsules.len());
    }
    ExitCode::SUCCESS
}

fn set_runtime_environment(home: &AosHome) {
    // Safety: this runs before the current-thread client runtime starts and before this
    // dedicated CLI process creates any other threads.
    unsafe {
        std::env::set_var("ASTRID_HOME", home.runtime_home());
        std::env::set_var("ASTRID_WORKSPACE_STATE_DIR", AOS_WORKSPACE_STATE_DIR);
        std::env::set_var("ASTRID_RUN_DIR", home.run_root());
        std::env::set_var(
            "ASTRID_CLIENT_CONFIG_PATH",
            home.root().join("etc/astrid/client.toml"),
        );
    }
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

    println!("Found a standalone runtime home at {}.", source.display());
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

fn handle_migrate_runtime(source: &Path) -> ExitCode {
    let home = match AosHome::resolve() {
        Ok(home) => home,
        Err(error) => {
            eprintln!("aos: failed to resolve product home: {error}");
            return ExitCode::FAILURE;
        }
    };
    match home.migrate_runtime_from(source) {
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
            "Imported legacy distro state was preserved. Run `aos distro apply --principal PRINCIPAL` to deliberately apply Unicity CE; provider configuration and imported state remain in place."
        );
    }
}

fn print_product_help() -> ExitCode {
    if let Err(error) = ProductCli::command().print_help() {
        eprintln!("aos: failed to print command help: {error}");
        return ExitCode::FAILURE;
    }
    println!();
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use clap::Parser;
    use std::ffi::OsString;

    use super::{
        ProductCli, ProductCommand, handle_product_command, help_targets_product, status_principal,
    };
    use crate::cli::DaemonCommand;
    use crate::dispatch::{
        child_exit_code, is_owned_root, leading_owned_root, runtime_args_for_dispatch,
        runtime_stop_requested,
    };

    #[test]
    fn product_cli_parses_owned_init_surface() {
        let cli = ProductCli::try_parse_from([
            "aos",
            "--principal",
            "alice",
            "init",
            "--yes",
            "--offline",
            "--var",
            "model=gpt-5",
        ])
        .expect("parse product init");
        let Some(ProductCommand::Init(init)) = cli.command else {
            panic!("expected product init command");
        };
        assert_eq!(cli.principal.as_deref(), Some("alice"));
        assert!(init.yes);
        assert!(init.offline);
        assert_eq!(init.vars, ["model=gpt-5"]);

        for flag in ["--allow-unsigned", "--accept-new-key"] {
            assert!(ProductCli::try_parse_from(["aos", "distro", "apply", flag]).is_err());
        }
    }

    #[test]
    fn product_cli_parses_persistent_foreground_daemon() {
        let cli = ProductCli::try_parse_from([
            "aos",
            "daemon",
            "foreground",
            "--workspace",
            "/workspace",
            "--verbose",
        ])
        .expect("parse foreground daemon");
        let Some(ProductCommand::Daemon {
            command: DaemonCommand::Foreground(args),
        }) = cli.command
        else {
            panic!("expected foreground daemon command");
        };
        assert_eq!(
            args.workspace.as_deref(),
            Some(std::path::Path::new("/workspace"))
        );
        assert!(args.verbose);
    }

    #[test]
    fn product_cli_parses_and_validates_status_principal() {
        let cli = ProductCli::try_parse_from(["aos", "--principal", "alice", "status"])
            .expect("parse principal-scoped product status");
        assert_eq!(cli.principal.as_deref(), Some("alice"));
        let Some(ProductCommand::Status(status)) = cli.command else {
            panic!("expected status");
        };
        assert!(status.principal.is_none());

        let cli = ProductCli::try_parse_from(["aos", "status", "--principal", "bob"])
            .expect("parse status-local principal");
        assert!(cli.principal.is_none());
        let Some(ProductCommand::Status(status)) = cli.command else {
            panic!("expected status");
        };
        assert_eq!(status.principal.as_deref(), Some("bob"));

        assert_eq!(
            status_principal(Some("alice".to_owned()), None)
                .expect("valid explicit principal")
                .as_str(),
            "alice"
        );
        assert_eq!(
            status_principal(None, Some("bob".to_owned()))
                .expect("valid status-local principal")
                .as_str(),
            "bob"
        );
        assert_eq!(
            status_principal(None, None)
                .expect("omitted principal keeps compatibility default")
                .as_str(),
            "default"
        );
        assert!(status_principal(None, Some("not/a/principal".to_owned())).is_err());
        assert!(status_principal(Some("alice".to_owned()), Some("bob".to_owned())).is_err());
    }

    #[test]
    fn product_version_preserves_the_installer_contract() {
        let Err(version) = ProductCli::try_parse_from(["aos", "--version"]) else {
            panic!("--version exits through Clap");
        };

        assert_eq!(
            version.to_string(),
            format!("Unicity AOS {}\n", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn product_apply_rejects_unselected_paths_before_runtime_dispatch() {
        assert!(ProductCli::try_parse_from(["aos", "distro", "apply"]).is_ok());
        assert!(
            ProductCli::try_parse_from(["aos", "distro", "apply", "/other/Distro.toml"]).is_ok()
        );
        assert!(ProductCli::try_parse_from(["aos", "init", "--grant-capsules"]).is_err());
        assert!(ProductCli::try_parse_from(["aos", "init", "--principal", "alice"]).is_err());
    }

    #[test]
    fn runtime_dispatch_does_not_invent_a_grant_backdoor() {
        assert_eq!(
            runtime_args_for_dispatch(vec![OsString::from("init")]),
            [OsString::from("init")]
        );
        assert_eq!(
            runtime_args_for_dispatch(vec![OsString::from("distro"), OsString::from("apply")]),
            [OsString::from("distro"), OsString::from("apply")]
        );
        assert_eq!(
            runtime_args_for_dispatch(vec![OsString::from("doctor")]),
            [OsString::from("doctor")]
        );
    }

    #[test]
    fn unowned_command_is_left_for_runtime_parser() {
        assert!(handle_product_command(&[OsString::from("doctor")]).is_none());
    }

    #[test]
    fn runtime_stop_keeps_the_inherited_argument_surface() {
        assert!(runtime_stop_requested(&[OsString::from("stop")]));
        assert!(runtime_stop_requested(&[
            OsString::from("--principal"),
            OsString::from("operator"),
            OsString::from("stop"),
        ]));
        assert!(!runtime_stop_requested(&[
            OsString::from("capsule"),
            OsString::from("stop"),
        ]));
        assert!(runtime_stop_requested(&[
            OsString::from("--future-runtime-global"),
            OsString::from("future-value"),
            OsString::from("stop"),
        ]));
        assert!(!runtime_stop_requested(&[
            OsString::from("--future-runtime-global"),
            OsString::from("future-value"),
            OsString::from("capsule"),
            OsString::from("stop"),
        ]));
    }

    #[test]
    fn help_is_owned_only_for_the_product_root_or_product_commands() {
        assert!(help_targets_product(&[OsString::from("help")]));
        for root in [
            "init",
            "status",
            "migrate",
            "update",
            "distro",
            "daemon",
            "serve-health",
        ] {
            let args = [OsString::from("help"), OsString::from(root)];
            assert!(help_targets_product(&args));
            assert!(handle_product_command(&args).is_some());
        }
        for root in ["doctor", "capsule", "completion"] {
            let args = [OsString::from("help"), OsString::from(root)];
            assert!(!help_targets_product(&args));
            assert!(handle_product_command(&args).is_none());
            assert_eq!(runtime_args_for_dispatch(args.to_vec()), args);
        }
    }

    #[test]
    fn leading_runtime_globals_cannot_bypass_owned_roots() {
        assert_eq!(
            leading_owned_root(&[
                OsString::from("--principal"),
                OsString::from("alice"),
                OsString::from("status"),
            ]),
            Some("status")
        );
        assert!(
            handle_product_command(&[
                OsString::from("--principal"),
                OsString::from("alice"),
                OsString::from("status"),
            ])
            .is_some()
        );
        assert!(ProductCli::try_parse_from(["aos", "--principal", "alice", "init"]).is_ok());
        assert!(
            handle_product_command(&[
                OsString::from("--principal"),
                OsString::from("init"),
                OsString::from("status"),
            ])
            .is_some()
        );
        assert!(
            handle_product_command(&[OsString::from("--principal"), OsString::from("init")])
                .is_some()
        );
    }

    #[test]
    fn unknown_runtime_command_with_distro_flag_is_exact_passthrough() {
        assert!(
            handle_product_command(&[
                OsString::from("frobnicate"),
                OsString::from("--distro"),
                OsString::from("other"),
            ])
            .is_none()
        );
        assert!(handle_product_command(&[OsString::from("capsule")]).is_none());
    }

    #[test]
    fn clap_rejects_extra_product_arguments() {
        assert!(ProductCli::try_parse_from(["aos", "self-update", "extra"]).is_err());
        assert!(ProductCli::try_parse_from(["aos", "migrate", "runtime"]).is_err());
    }

    #[test]
    fn update_aliases_and_status_are_product_owned() {
        for command in ["update", "self-update", "self_update"] {
            let cli = ProductCli::try_parse_from(["aos", command]).expect("parse update alias");
            assert!(matches!(cli.command, Some(ProductCommand::Update(_))));
        }
        let cli =
            ProductCli::try_parse_from(["aos", "status", "--json"]).expect("parse product status");
        let Some(ProductCommand::Status(status)) = cli.command else {
            panic!("expected product status command");
        };
        assert!(status.json);
    }

    #[test]
    fn runtime_command_contract_matches_the_product_router() {
        let contract: toml::Value = include_str!("../../../release/runtime-command-surface.toml")
            .parse()
            .expect("parse runtime command surface");
        let roots = contract["roots"].as_table().expect("root classifications");

        for root in roots["product-owned"]
            .as_array()
            .expect("product-owned roots")
        {
            assert!(is_owned_root(root.as_str().expect("runtime root")));
        }
        for bucket in ["inherited", "hidden-inherited"] {
            for root in roots[bucket].as_array().expect("inherited roots") {
                assert!(!is_owned_root(root.as_str().expect("runtime root")));
            }
        }
        assert_eq!(
            roots["shared"].as_array().expect("shared roots"),
            &[toml::Value::String("help".to_owned())]
        );
    }

    #[cfg(unix)]
    #[test]
    fn child_exit_mapping_preserves_codes_and_maps_signals_to_failure() {
        use std::os::unix::process::ExitStatusExt;

        let success = std::process::Command::new("sh")
            .args(["-c", "exit 0"])
            .status()
            .expect("run successful child");
        let failure = std::process::Command::new("sh")
            .args(["-c", "exit 37"])
            .status()
            .expect("run failed child");

        assert_eq!(child_exit_code(success), std::process::ExitCode::SUCCESS);
        assert_eq!(child_exit_code(failure), std::process::ExitCode::from(37));
        assert_eq!(
            child_exit_code(std::process::ExitStatus::from_raw(9)),
            std::process::ExitCode::FAILURE
        );
    }
}
