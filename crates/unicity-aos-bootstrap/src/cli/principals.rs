//! Command surface for owned-principal discovery.

use std::process::{ExitCode, Stdio};

use astrid_core::PrincipalId;
use clap::Args;

use super::super::principals::{
    MINE_UNSUPPORTED_MESSAGE, document_from_runtime_json, runtime_discovery_args,
    runtime_discovery_unsupported,
};

#[derive(Args)]
pub(crate) struct PrincipalsArgs {
    /// Print the owned-principal discovery document.
    #[arg(long)]
    pub(crate) json: bool,
    /// Authenticated operator principal used for discovery, not the viewed agent.
    #[arg(
        id = "principals-principal",
        long = "principal",
        value_name = "OPERATOR_PRINCIPAL",
        value_parser = clap::builder::NonEmptyStringValueParser::new()
    )]
    pub(crate) principal: Option<String>,
}

fn discovery_principal(
    leading_principal: Option<String>,
    trailing_principal: Option<String>,
) -> Result<PrincipalId, String> {
    let principal = match (leading_principal, trailing_principal) {
        (Some(_), Some(_)) => {
            return Err(
                "'--principal' was provided both before and after `principals`; provide it once"
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
                .map_err(|error| format!("invalid discovery principal: {error}"))
        },
    )
}

pub(crate) fn handle_principals(
    leading_principal: Option<String>,
    command_principal: Option<String>,
    json: bool,
) -> ExitCode {
    if !json {
        eprintln!("aos: `principals` requires --json");
        return ExitCode::from(2);
    }
    let principal = match discovery_principal(leading_principal, command_principal) {
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
    let mut command = match home.runtime_command_with_args(runtime_discovery_args(&principal)) {
        Ok(command) => command,
        Err(error) => {
            eprintln!("aos: owned-principal discovery failed: {error}");
            return ExitCode::FAILURE;
        }
    };
    command.stdin(Stdio::null());
    let output = match command.output() {
        Ok(output) => output,
        Err(error) => {
            eprintln!("aos: owned-principal discovery failed: {error}");
            return ExitCode::FAILURE;
        }
    };
    if !output.status.success() {
        if runtime_discovery_unsupported(&output) {
            eprintln!("aos: {MINE_UNSUPPORTED_MESSAGE}");
            return ExitCode::from(2);
        }
        let detail = String::from_utf8_lossy(&output.stderr);
        let detail = detail.trim();
        if detail.is_empty() {
            eprintln!("aos: owned-principal discovery failed");
        } else {
            eprintln!("aos: owned-principal discovery failed: {detail}");
        }
        return ExitCode::FAILURE;
    }
    let document = match document_from_runtime_json(&output.stdout) {
        Ok(document) => document,
        Err(error) => {
            eprintln!("aos: {error}");
            return ExitCode::FAILURE;
        }
    };
    match serde_json::to_string(&document) {
        Ok(json) => println!("{json}"),
        Err(error) => {
            eprintln!("aos: failed to encode owned-principal discovery: {error}");
            return ExitCode::FAILURE;
        }
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::{ProductCli, ProductCommand, is_owned_root};

    use super::discovery_principal;

    #[test]
    fn product_cli_parses_and_validates_owned_principal_discovery() {
        assert!(ProductCli::try_parse_from(["aos", "principals"]).is_ok());
        let cli = ProductCli::try_parse_from(["aos", "principals", "--json"])
            .expect("parse owned-principal discovery");
        let Some(ProductCommand::Principals(args)) = cli.command else {
            panic!("expected principals command");
        };
        assert!(args.json);
        assert!(args.principal.is_none());
        let cli =
            ProductCli::try_parse_from(["aos", "--principal", "operator", "principals", "--json"])
                .expect("parse operator-scoped discovery");
        assert_eq!(cli.principal.as_deref(), Some("operator"));
        let cli = ProductCli::try_parse_from(["aos", "principals", "--json", "--principal", "bob"])
            .expect("parse discovery-local principal");
        let Some(ProductCommand::Principals(args)) = cli.command else {
            panic!("expected principals command");
        };
        assert_eq!(args.principal.as_deref(), Some("bob"));
        assert_eq!(
            discovery_principal(None, None)
                .expect("omitted principal keeps compatibility default")
                .as_str(),
            "default"
        );
        assert!(discovery_principal(None, Some("not/a/principal".to_owned())).is_err());
        assert!(discovery_principal(Some("alice".to_owned()), Some("bob".to_owned())).is_err());
        assert!(is_owned_root("principals"));
    }
}
