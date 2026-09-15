//! Complete Astrid's bounded installer batches without reimplementing receipts.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use std::time::Duration;

// Astrid 2026.9.0's InstallCapsule protocol allows ten requests per minute.
// This is compatibility with that published contract, not a new AOS limit.
const INSTALL_BATCH: usize = 10;
const INSTALL_WINDOW: Duration = Duration::from_secs(61);

// The preparation pass always installs the embedded fleet for default, not
// the caller's Oracle principal. Do not extend host-specific grant sets here.
pub(super) fn grant_args(assets: &[String]) -> Vec<String> {
    let mut args = ["--principal", "default", "agent", "modify", "default"]
        .map(str::to_owned)
        .to_vec();
    for asset in assets {
        args.push("--add-capsule".to_owned());
        args.push(asset.trim_end_matches(".capsule").to_owned());
    }
    args
}

pub(super) fn initialize(mut command: Command, expected: usize) -> io::Result<()> {
    resume(
        expected,
        || run_pass(&mut command),
        || {
            eprintln!(
                "aos: continuing capsule installation after the runtime's rate-limit window..."
            );
            std::thread::sleep(INSTALL_WINDOW);
        },
    )
}

fn run_pass(command: &mut Command) -> io::Result<(bool, String)> {
    run_pass_to(command, &mut io::stderr().lock())
}

fn run_pass_to(command: &mut Command, output: &mut dyn Write) -> io::Result<(bool, String)> {
    let path = std::env::temp_dir().join(format!("aos-init-{}.log", uuid::Uuid::new_v4()));
    let mut options = OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(&path)?;
    // Keep stderr file-backed so a daemon inheriting the descriptor cannot
    // hold a pipe open forever. Poll the regular file while the direct child
    // runs so interactive questions are visible before stdin asks for input.
    let result = (|| {
        let mut child = command.stderr(Stdio::from(file)).spawn()?;
        let mut reader = File::open(&path)?;
        let mut captured = Vec::new();
        let status = loop {
            stream_available(&mut reader, output, &mut captured)?;
            if let Some(status) = child.try_wait()? {
                stream_available(&mut reader, output, &mut captured)?;
                break status;
            }
            std::thread::sleep(Duration::from_millis(25));
        };
        output.flush()?;
        Ok((
            status.success(),
            String::from_utf8_lossy(&captured).into_owned(),
        ))
    })();
    let _ = fs::remove_file(path);
    result
}

fn stream_available(
    reader: &mut File,
    output: &mut dyn Write,
    captured: &mut Vec<u8>,
) -> io::Result<()> {
    let start = captured.len();
    reader.read_to_end(captured)?;
    if captured.len() > start {
        output.write_all(&captured[start..])?;
        output.flush()?;
    }
    Ok(())
}

fn resume(
    expected: usize,
    mut run: impl FnMut() -> io::Result<(bool, String)>,
    mut wait: impl FnMut(),
) -> io::Result<()> {
    let mut previous = 0;
    // At most ceil(N/10) incomplete passes plus a final successful pass.
    // A partial existing installation may consume less than the first batch.
    let max_passes = expected.div_ceil(INSTALL_BATCH) + 1;
    for pass in 0..max_passes {
        let (success, stderr) = run()?;
        if success {
            return Ok(());
        }
        let Some((completed, total)) = partial_progress(&stderr) else {
            return Err(io::Error::other(
                "bundled CE system-fleet initializer exited unsuccessfully; see runtime error above",
            ));
        };
        if total != expected || completed <= previous || completed >= total {
            return Err(io::Error::other(
                "bundled CE initializer made no valid progress",
            ));
        }
        previous = completed;
        if pass + 1 == max_passes {
            break;
        }
        wait();
    }
    Err(io::Error::other(
        "bundled CE initializer exceeded its bounded resume passes",
    ))
}

fn partial_progress(stderr: &str) -> Option<(usize, usize)> {
    // Only the pinned CLI's explicit partial-batch result can be resumed.
    // Ordinary package, validation or lifecycle failures must remain errors.
    if stderr.contains("Failed to install ") || stderr.contains("Configuration for ") {
        return None;
    }
    let (_, message) = stderr.rsplit_once("Installation incomplete: ")?;
    let (counts, suffix) = message.split_once(" capsule(s) installed")?;
    if !suffix.contains("re-run `astrid init` to retry the rest.") {
        return None;
    }
    let (completed, total) = counts.split_once('/')?;
    Some((completed.parse().ok()?, total.parse().ok()?))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(unix)]
    struct AcknowledgePrompt {
        bytes: Vec<u8>,
        path: std::path::PathBuf,
    }

    #[cfg(unix)]
    impl Write for AcknowledgePrompt {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            self.bytes.extend_from_slice(buf);
            fs::write(&self.path, b"seen")?;
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn final_grant_uses_only_the_embedded_default_fleet() {
        assert_eq!(
            grant_args(&["aos-cli.capsule".into(), "aos-mcp.capsule".into()]),
            [
                "--principal",
                "default",
                "agent",
                "modify",
                "default",
                "--add-capsule",
                "aos-cli",
                "--add-capsule",
                "aos-mcp"
            ]
        );
    }

    #[cfg(unix)]
    #[test]
    fn child_prompt_is_streamed_before_the_child_exits() {
        let acknowledgement =
            std::env::temp_dir().join(format!("aos-init-prompt-seen-{}", uuid::Uuid::new_v4()));
        let mut output = AcknowledgePrompt {
            bytes: Vec::new(),
            path: acknowledgement.clone(),
        };
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "printf 'Visible question? ' >&2; i=0; while [ $i -lt 50 ]; do [ -f \"$1\" ] && exit 0; i=$((i + 1)); sleep 0.02; done; exit 9",
            "aos-init-test",
            acknowledgement.to_str().expect("temporary path is UTF-8"),
        ]);

        let result = run_pass_to(&mut command, &mut output).expect("run child");
        let _ = fs::remove_file(acknowledgement);

        assert!(result.0, "child never observed its streamed prompt");
        assert_eq!(result.1, "Visible question? ");
        assert_eq!(output.bytes, b"Visible question? ");
    }

    fn partial(completed: usize) -> (bool, String) {
        (
            false,
            format!(
                "Installation incomplete: {completed}/22 capsule(s) installed — re-run `astrid init` to retry the rest."
            ),
        )
    }

    #[test]
    fn completes_ten_ten_two_in_one_product_invocation() {
        let mut passes = [partial(10), partial(20), (true, String::new())].into_iter();
        let mut waits = 0;
        resume(22, || Ok(passes.next().unwrap()), || waits += 1).unwrap();
        assert_eq!(waits, 2);
        assert!(passes.next().is_none());
    }

    #[test]
    fn complete_install_never_waits() {
        resume(
            22,
            || Ok((true, String::new())),
            || panic!("unexpected wait"),
        )
        .unwrap();
    }

    #[test]
    fn stalled_resume_stops_instead_of_looping() {
        let mut calls = 0;
        assert!(
            resume(
                22,
                || {
                    calls += 1;
                    Ok(partial(10))
                },
                || {}
            )
            .is_err()
        );
        assert_eq!(calls, 2);
    }

    #[test]
    fn package_failure_is_not_treated_as_batch_exhaustion() {
        let (_, partial) = partial(10);
        let error = format!("Failed to install aos-shell: invalid signature\n{partial}");
        assert!(
            resume(
                22,
                || Ok((false, error.clone())),
                || panic!("unexpected wait")
            )
            .is_err()
        );
    }

    #[test]
    fn rejects_wrong_inventory_and_unrelated_errors() {
        assert!(resume(23, || Ok(partial(10)), || panic!("unexpected wait")).is_err());
        assert!(partial_progress("connection refused").is_none());
        assert!(partial_progress("Installation incomplete: 10/22 capsule(s) installed").is_none());
    }
}
