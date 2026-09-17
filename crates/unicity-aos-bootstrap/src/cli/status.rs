//! Command surface for product status.

use std::path::PathBuf;
use std::process::ExitCode;

use astrid_core::PrincipalId;
use clap::Args;

#[derive(Args)]
pub(crate) struct StatusArgs {
    /// Verify an existing macOS mount against this runtime's authenticated lease.
    #[arg(long, requires = "json", conflicts_with = "include_capsules")]
    pub(crate) mountpoint: Option<PathBuf>,
    /// Include principal-visible capsule metadata without starting the runtime.
    #[arg(long, requires = "json")]
    pub(crate) include_capsules: bool,
    /// Authenticated runtime principal for this status request.
    #[arg(
        id = "status-principal",
        long = "principal",
        value_name = "PRINCIPAL",
        value_parser = clap::builder::NonEmptyStringValueParser::new()
    )]
    pub(crate) principal: Option<String>,
    /// Print a machine-readable JSON status object.
    #[arg(long)]
    pub(crate) json: bool,
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

pub(crate) fn handle_status(
    leading_principal: Option<String>,
    command_principal: Option<String>,
    json: bool,
    include_capsules: bool,
    mountpoint: Option<PathBuf>,
) -> ExitCode {
    let principal = match status_principal(leading_principal, command_principal) {
        Ok(principal) => principal,
        Err(error) => {
            eprintln!("aos: {error}");
            return ExitCode::from(2);
        }
    };
    let home = match crate::resolve_home() {
        Ok(home) => home,
        Err(code) => return code,
    };
    crate::set_runtime_environment(&home);
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
    let mut aos_status = match runtime.block_on(unicity_aos_bootstrap::status::read_with_inventory(
        &home,
        principal.clone(),
        include_capsules,
    )) {
        Ok(aos_status) => aos_status,
        Err(error) => {
            eprintln!("aos: runtime status unavailable: {error}");
            return ExitCode::FAILURE;
        }
    };

    if let Some(path) = mountpoint {
        if aos_status.state != "running" {
            eprintln!("aos: a mounted volume requires a running runtime");
            return ExitCode::FAILURE;
        }
        match runtime.block_on(unicity_aos_bootstrap::status::mounted_volume::read(
            &home, principal, &path,
        )) {
            Ok(mount) => aos_status.mounted_volume = Some(mount),
            Err(error) => {
                eprintln!("aos: mounted volume unavailable: {error}");
                return ExitCode::FAILURE;
            }
        }
    }

    if json {
        match serde_json::to_string(&aos_status) {
            Ok(json) => println!("{json}"),
            Err(error) => {
                eprintln!("aos: failed to encode status: {error}");
                return ExitCode::FAILURE;
            }
        }
    } else {
        println!("Unicity AOS");
        println!("State: {}", aos_status.state);
        println!("PID: {}", aos_status.pid);
        println!("Uptime: {}", aos_status.uptime_secs);
        println!("Runtime version: {}", aos_status.runtime_version);
        println!("Connected clients: {}", aos_status.connected_clients);
        println!("Loaded capsules: {}", aos_status.loaded_capsules.len());
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::{ProductCli, ProductCommand, is_owned_root};

    use super::status_principal;

    #[test]
    fn product_cli_parses_and_validates_status_principal() {
        assert!(ProductCli::try_parse_from(["aos", "status", "--include-capsules"]).is_err());
        let inventory_cli = ProductCli::try_parse_from([
            "aos",
            "status",
            "--json",
            "--include-capsules",
            "--principal",
            "alice",
        ])
        .expect("parse inventory status");
        let Some(ProductCommand::Status(inventory)) = inventory_cli.command else {
            panic!("expected inventory status");
        };
        assert!(inventory.include_capsules);
        assert_eq!(inventory.principal.as_deref(), Some("alice"));
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
        assert!(is_owned_root("status"));
    }
}
