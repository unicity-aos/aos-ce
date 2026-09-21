//! Child-only defaults for the bundled runtime; never mutate the host environment.

use std::ffi::OsStr;
use std::process::Command;

// On many-core musl hosts, unconstrained Rayon compilation contends heavily
// during cold WASM loading. Four workers restored practical first-run latency
// in the packaged ARM64 journey without restricting runtime CPU affinity.
const MUSL_RAYON_WORKERS: usize = 4;

pub(super) fn configure(command: &mut Command) {
    apply_rayon_default(
        command,
        cfg!(target_env = "musl"),
        std::env::var_os("RAYON_NUM_THREADS").as_deref(),
        std::thread::available_parallelism().map_or(1, usize::from),
    );
}

fn apply_rayon_default(
    command: &mut Command,
    musl: bool,
    inherited: Option<&OsStr>,
    available: usize,
) {
    // Preserve explicit operator settings, including Rayon's own zero/default
    // semantics. GNU and Darwin retain their existing scheduling defaults.
    if musl && inherited.is_none() {
        command.env(
            "RAYON_NUM_THREADS",
            available.clamp(1, MUSL_RAYON_WORKERS).to_string(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn musl_default_is_bounded_by_available_cpus() {
        for (available, expected) in [(0, "1"), (1, "1"), (2, "2"), (4, "4"), (64, "4")] {
            let mut command = Command::new("astrid");
            apply_rayon_default(&mut command, true, None, available);
            assert_eq!(
                command.get_envs().collect::<Vec<_>>(),
                vec![(OsStr::new("RAYON_NUM_THREADS"), Some(OsStr::new(expected)))]
            );
        }
    }

    #[test]
    fn explicit_settings_are_not_overridden() {
        for value in ["1", "8", "0", "", "invalid"] {
            let mut command = Command::new("astrid");
            apply_rayon_default(&mut command, true, Some(OsStr::new(value)), 64);
            assert_eq!(command.get_envs().count(), 0);
        }
    }

    #[test]
    fn other_platforms_keep_their_defaults() {
        let mut command = Command::new("astrid");
        apply_rayon_default(&mut command, false, None, 64);
        assert_eq!(command.get_envs().count(), 0);
    }
}
