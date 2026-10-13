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

/// Match the calendar versions accepted by the signed product installer.
pub(crate) fn parse_release(value: &str) -> Result<String, String> {
    let version = semver::Version::parse(value).map_err(|error| error.to_string())?;
    if !(2026..=2099).contains(&version.major) || !version.build.is_empty() {
        return Err("expected a canonical AOS calendar release version".into());
    }
    let pre = version.pre.as_str();
    if !pre.is_empty() {
        if let Some(rc) = pre.strip_prefix("rc.") {
            if rc.starts_with('0') || rc.is_empty() || !rc.bytes().all(|byte| byte.is_ascii_digit())
            {
                return Err("expected a positive canonical RC number".into());
            }
        } else if let Some(nightly) = pre.strip_prefix("nightly.") {
            let Some((date, commit)) = nightly.split_once(".g") else {
                return Err("expected a nightly date and full source commit".into());
            };
            if !valid_date(date)
                || commit.len() != 40
                || !commit
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
            {
                return Err("expected a valid nightly date and lowercase source commit".into());
            }
        } else {
            return Err("expected stable, rc.N or nightly.YYYYMMDD.gCOMMIT".into());
        }
    }
    Ok(value.to_owned())
}

fn valid_date(date: &str) -> bool {
    if date.len() != 8 || !date.bytes().all(|byte| byte.is_ascii_digit()) {
        return false;
    }
    let year: u16 = date[..4].parse().unwrap_or(0);
    let month: u8 = date[4..6].parse().unwrap_or(0);
    let day: u8 = date[6..].parse().unwrap_or(0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if year.is_multiple_of(400) || (year.is_multiple_of(4) && !year.is_multiple_of(100)) => {
            29
        }
        2 => 28,
        _ => 0,
    };
    day > 0 && day <= days
}

#[cfg(test)]
mod tests {
    use super::super::UpdateChannel;

    #[test]
    fn exact_updates_accept_canonical_dev_and_nightly_releases() {
        for version in [
            "2026.10.0",
            "2026.10.0-rc.7",
            "2026.10.0-nightly.20261013.gaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        ] {
            assert_eq!(super::parse_release(version).unwrap(), version);
        }
        for version in [
            "2026.01.0",
            "2026.10.0-rc.0",
            "2026.10.0-rc.01",
            "2026.10.0-beta.1",
            "2026.10.0+build",
            "2026.10.0-nightly.20260230.gaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "2026.10.0-nightly.20261013.gabc",
        ] {
            assert!(super::parse_release(version).is_err(), "{version}");
        }
    }

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
