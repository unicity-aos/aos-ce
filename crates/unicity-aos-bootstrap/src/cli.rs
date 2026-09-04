//! Product-owned command-line surface parsed before runtime passthrough.

use clap::{Args, Parser, Subcommand, ValueEnum};

use crate::{hook, mcp};

// Product-owned commands are parsed here. Unknown roots bypass this parser and
// are delegated byte-for-byte to the bundled runtime by `main`.
#[derive(Parser)]
#[command(name = "Unicity AOS", bin_name = "aos")]
#[command(version)]
#[command(about = "Unicity Agent Operating System")]
#[command(long_about = None)]
#[command(
    after_help = "All other commands are inherited from the bundled runtime. Running `aos` without a command displays product help until the native AOS chat surface lands."
)]
pub(crate) struct ProductCli {
    /// Authenticated runtime principal for principal-scoped AOS commands.
    #[arg(
        long,
        value_name = "PRINCIPAL",
        value_parser = clap::builder::NonEmptyStringValueParser::new()
    )]
    pub(crate) principal: Option<String>,
    #[command(subcommand)]
    pub(crate) command: Option<ProductCommand>,
}

#[derive(Subcommand)]
pub(crate) enum ProductCommand {
    /// Apply Unicity CE; retained as an alias of `distro apply`.
    Init(DistributionApplyArgs),
    /// Show product status from the typed local runtime operation.
    Status(StatusArgs),
    /// Import compatible state from a standalone runtime installation.
    Migrate {
        #[command(subcommand)]
        command: MigrateCommand,
    },
    /// Update AOS and its coordinated runtime executable set.
    #[command(name = "update", alias = "self-update", alias = "self_update")]
    Update(UpdateArgs),
    /// Apply the selected Unicity CE distribution to one principal.
    Distro {
        #[command(subcommand)]
        command: DistroCommand,
    },
    /// Deliver a host hook through the authenticated AOS event bus.
    Hook(hook::HookArgs),
    /// Expose this AOS installation to an MCP host over stdio.
    Mcp {
        #[command(subcommand)]
        command: McpCommand,
    },
    /// Serve the loopback-only product health endpoint.
    ServeHealth,
    /// Run the bundled runtime daemon in the foreground.
    Daemon {
        #[command(subcommand)]
        command: DaemonCommand,
    },
}

#[derive(Subcommand)]
pub(crate) enum DaemonCommand {
    /// Run the persistent bundled daemon in the foreground.
    Foreground(ForegroundDaemonArgs),
}

#[derive(Args)]
pub(crate) struct ForegroundDaemonArgs {
    /// Project workspace owned by this daemon.
    #[arg(long, value_name = "PATH")]
    pub(crate) workspace: Option<std::path::PathBuf>,
    /// Enable debug-level daemon logging.
    #[arg(short, long)]
    pub(crate) verbose: bool,
}

#[derive(Subcommand)]
pub(crate) enum McpCommand {
    /// Serve AOS tools and broker interactions over stdio.
    Serve(mcp::ServeArgs),
}

#[derive(Subcommand)]
pub(crate) enum DistroCommand {
    /// Apply the selected Unicity CE distribution.
    Apply(DistributionApplyArgs),
}

#[derive(Args)]
pub(crate) struct StatusArgs {
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

#[derive(Args)]
pub(crate) struct UpdateArgs {
    /// Follow the signed stable, dev, or nightly product channel.
    #[arg(long, value_enum, conflicts_with = "version")]
    pub(crate) channel: Option<UpdateChannel>,
    /// Install an exact signed AOS calendar-semver release.
    #[arg(long, value_parser = parse_aos_version, conflicts_with = "channel")]
    pub(crate) version: Option<String>,
}

#[derive(Clone, Copy, ValueEnum)]
pub(crate) enum UpdateChannel {
    Stable,
    Dev,
    Nightly,
}

impl UpdateChannel {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Dev => "dev",
            Self::Nightly => "nightly",
        }
    }
}

pub(crate) fn parse_aos_version(value: &str) -> Result<String, String> {
    let components = value.split('.').collect::<Vec<_>>();
    let canonical = |component: &str| {
        component == "0"
            || (component.as_bytes().first().is_some_and(u8::is_ascii_digit)
                && !component.starts_with('0')
                && component.bytes().all(|byte| byte.is_ascii_digit()))
    };
    if components.len() != 3
        || components[0].len() != 4
        || !components[0].bytes().all(|byte| byte.is_ascii_digit())
        || !canonical(components[1])
        || !canonical(components[2])
    {
        return Err("expected YYYY.MINOR.PATCH without leading zeroes".to_owned());
    }
    let year = components[0]
        .parse::<u16>()
        .map_err(|_| "release year is invalid".to_owned())?;
    if !(2026..=2099).contains(&year) {
        return Err("release year must be between 2026 and 2099".to_owned());
    }
    Ok(value.to_owned())
}

#[derive(Args)]
pub(crate) struct DistributionApplyArgs {
    /// Selected Distro.toml; the bundled Unicity CE manifest is the default.
    #[arg(value_name = "DISTRO_TOML")]
    pub(crate) distro: Option<std::path::PathBuf>,
    /// Accept defaults without prompting.
    #[arg(short = 'y', long = "yes")]
    pub(crate) yes: bool,
    /// Forbid network access during initialization.
    #[arg(long)]
    pub(crate) offline: bool,
    /// Supply a distribution variable as KEY=VALUE; repeat as needed.
    #[arg(long = "var", value_name = "KEY=VALUE")]
    pub(crate) vars: Vec<String>,
}

#[derive(Subcommand)]
pub(crate) enum MigrateCommand {
    /// Copy compatible state from a standalone runtime home.
    Runtime {
        /// Absolute path to the standalone runtime home.
        #[arg(long, value_name = "ABSOLUTE_LEGACY_HOME")]
        from: std::path::PathBuf,
    },
}
