//! Product version reporting is local and never invokes the runtime updater.
use clap::{Args, ValueEnum};
use std::process::ExitCode;

#[derive(Args, Default)]
pub(crate) struct Arguments {
    #[arg(long, value_enum, default_value = "text")]
    format: Format,
}

#[derive(Clone, Copy, Default, ValueEnum)]
enum Format {
    #[default]
    #[value(alias = "pretty")]
    Text,
    Json,
}

pub(crate) fn print(args: &Arguments) -> ExitCode {
    let runtime = super::distro_trust::ASTRID_RUNTIME_VERSION;
    match args.format {
        Format::Text => println!(
            "Unicity AOS {}\n  runtime: Astrid {runtime}",
            env!("CARGO_PKG_VERSION")
        ),
        Format::Json => println!(
            "{}",
            serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "runtime_version": runtime,
            })
        ),
    }
    ExitCode::SUCCESS
}

#[cfg(test)]
mod tests {
    use super::super::UpdateChannel;

    #[test]
    fn automatic_updates_follow_the_installed_release_train() {
        for (version, expected) in [
            ("2026.9.4", "stable"),
            ("2026.10.0-rc.4", "dev"),
            ("2026.10.0-nightly.20261013.gabcdef", "nightly"),
        ] {
            assert_eq!(UpdateChannel::for_version(version).as_str(), expected);
        }
    }
}
