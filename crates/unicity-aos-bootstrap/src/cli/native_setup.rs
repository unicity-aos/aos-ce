//! Command surface for local-personal native-input enrollment.

use std::process::ExitCode;

use astrid_core::PrincipalId;
use clap::Args;

#[derive(Args)]
pub(crate) struct NativeSetupArgs {
    /// Print the local-personal setup receipt.
    #[arg(long)]
    json: bool,
    /// Confirm device enrollment for this local machine.
    #[arg(long)]
    confirm_enroll: bool,
    /// Confirm operator routing for the selected principal.
    #[arg(long)]
    confirm_route: bool,
    /// Owned principal to enroll. Required; never inferred from discovery.
    #[arg(
        id = "native-setup-principal",
        long = "principal",
        value_name = "OPERATOR_PRINCIPAL",
        value_parser = clap::builder::NonEmptyStringValueParser::new()
    )]
    principal: Option<String>,
}

fn setup_principal(
    leading_principal: Option<String>,
    trailing_principal: Option<String>,
) -> Result<PrincipalId, String> {
    let principal = match (leading_principal, trailing_principal) {
        (Some(_), Some(_)) => {
            return Err(
                "'--principal' was provided both before and after `native-setup`; provide it once"
                    .to_owned(),
            );
        }
        (Some(principal), None) | (None, Some(principal)) => principal,
        (None, None) => {
            return Err(
                "`aos native-setup` requires an explicit '--principal PRINCIPAL'".to_owned(),
            );
        }
    };
    let principal =
        PrincipalId::new(principal).map_err(|error| format!("invalid setup principal: {error}"))?;
    if principal == PrincipalId::anonymous() {
        return Err("native-input setup requires a non-anonymous owned principal".to_owned());
    }
    Ok(principal)
}

pub(crate) fn handle_native_setup(
    leading_principal: Option<String>,
    args: NativeSetupArgs,
) -> ExitCode {
    if !args.json || !args.confirm_enroll || !args.confirm_route {
        eprintln!(
            "aos: `native-setup` requires --json --confirm-enroll --confirm-route and an explicit --principal"
        );
        return ExitCode::from(2);
    }
    let principal = match setup_principal(leading_principal, args.principal) {
        Ok(principal) => principal,
        Err(error) => {
            eprintln!("aos: {error}");
            return ExitCode::from(2);
        }
    };
    run_native_setup(principal)
}

#[cfg(not(unix))]
fn run_native_setup(_principal: PrincipalId) -> ExitCode {
    eprintln!("aos: native-input setup is currently supported only on Unix");
    ExitCode::from(2)
}

#[cfg(unix)]
fn run_native_setup(principal: PrincipalId) -> ExitCode {
    let home = match crate::resolve_home() {
        Ok(home) => home,
        Err(code) => return code,
    };
    crate::set_runtime_environment(&home);
    match crate::native_setup::run(&home, &principal) {
        Ok(document) => match serde_json::to_string(&document) {
            Ok(json) => {
                println!("{json}");
                ExitCode::SUCCESS
            }
            Err(error) => {
                eprintln!("aos: failed to encode native-setup receipt: {error}");
                ExitCode::FAILURE
            }
        },
        Err(crate::native_setup::SetupError::Unsupported) => {
            eprintln!("aos: {}", crate::native_setup::UNSUPPORTED_MESSAGE);
            ExitCode::from(2)
        }
        Err(crate::native_setup::SetupError::Failed(message)) => {
            eprintln!("aos: {message}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use clap::Parser;

    use crate::{ProductCli, ProductCommand, is_owned_root};

    use super::setup_principal;

    #[test]
    fn product_cli_parses_and_validates_native_setup() {
        assert!(ProductCli::try_parse_from(["aos", "native-setup"]).is_ok());
        let cli = ProductCli::try_parse_from([
            "aos",
            "native-setup",
            "--json",
            "--confirm-enroll",
            "--confirm-route",
            "--principal",
            "alice",
        ])
        .expect("parse native-setup");
        let Some(ProductCommand::NativeSetup(args)) = cli.command else {
            panic!("expected native-setup command");
        };
        assert!(args.json);
        assert!(args.confirm_enroll);
        assert!(args.confirm_route);
        assert_eq!(args.principal.as_deref(), Some("alice"));
        let cli = ProductCli::try_parse_from([
            "aos",
            "--principal",
            "alice",
            "native-setup",
            "--json",
            "--confirm-enroll",
            "--confirm-route",
        ])
        .expect("parse leading native-setup principal");
        assert_eq!(cli.principal.as_deref(), Some("alice"));
        assert!(setup_principal(None, None).is_err());
        assert!(setup_principal(None, Some("anonymous".to_owned())).is_err());
        assert!(setup_principal(None, Some("not/a/principal".to_owned())).is_err());
        assert!(setup_principal(Some("alice".to_owned()), Some("bob".to_owned())).is_err());
        assert_eq!(
            setup_principal(None, Some("alice".to_owned()))
                .expect("explicit principal")
                .as_str(),
            "alice"
        );
        assert!(is_owned_root("native-setup"));
        assert!(!is_owned_root("setup"));
    }
}
