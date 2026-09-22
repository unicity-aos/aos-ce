//! Bounded updater subprocesses. Output is private, drained without pipe deadlock,
//! and never interpreted as shell syntax.
use std::{
    fs, io,
    process::{Command, Stdio},
    time::{Duration, Instant},
};

pub(super) fn capture(command: &mut Command, timeout: Duration) -> Result<Vec<u8>, String> {
    execute(command, timeout, true)
}

pub(super) fn run(command: &mut Command, timeout: Duration) -> Result<(), String> {
    execute(command, timeout, false).map(|_| ())
}

fn execute(
    command: &mut Command,
    timeout: Duration,
    capture_output: bool,
) -> Result<Vec<u8>, String> {
    let directory = std::env::temp_dir().join(format!("aos-update-{}", uuid::Uuid::new_v4()));
    let mut builder = fs::DirBuilder::new();
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        builder.mode(0o700);
    }
    builder.create(&directory).map_err(|e| e.to_string())?;
    struct Cleanup(std::path::PathBuf);
    impl Drop for Cleanup {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }
    let _cleanup = Cleanup(directory.clone());
    let output = directory.join("stdout");
    let errors = directory.join("stderr");
    let stdout = fs::File::create(&output).map_err(|e| e.to_string())?;
    let stderr = fs::File::create(&errors).map_err(|e| e.to_string())?;
    command.stdin(Stdio::null()).stdout(stdout).stderr(stderr);
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command.spawn().map_err(|e| e.to_string())?;
    let start = Instant::now();
    let result = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Ok(status),
            Err(error) => break Err(error.to_string()),
            Ok(None) => {}
        }
        if start.elapsed() >= timeout {
            break Err(
                "Update operation timed out; check the installed state before retrying".into(),
            );
        }
        if [&output, &errors]
            .iter()
            .any(|p| fs::metadata(p).is_ok_and(|m| m.len() > 4 * 1024 * 1024))
        {
            break Err("Update operation exceeded its output limit".into());
        }
        std::thread::sleep(Duration::from_millis(40));
    };
    if result.is_err() {
        #[cfg(unix)]
        if let Some(pid) = rustix::process::Pid::from_raw(child.id() as i32) {
            let _ = rustix::process::kill_process_group(pid, rustix::process::Signal::KILL);
        }
        let _ = child.kill();
        let _ = child.wait();
    }
    let status = result?;
    if !status.success() {
        // Subprocess diagnostics can contain local paths or credentials. Keep
        // them out of shared UI/cache data; the exit status is actionable.
        return Err(format!(
            "Updater exited with {status}; run the corresponding update command for diagnostics"
        ));
    }
    if !capture_output {
        return Ok(Vec::new());
    }
    use io::Read;
    let mut bytes = Vec::new();
    fs::File::open(output)
        .map_err(|e| e.to_string())?
        .take(65_537)
        .read_to_end(&mut bytes)
        .map_err(|e| e.to_string())?;
    if bytes.len() > 65_536 {
        return Err("Updater response is too large".into());
    }
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn failure_is_not_empty_success() {
        assert!(
            capture(
                Command::new("sh").args(["-c", "exit 7"]),
                Duration::from_secs(1)
            )
            .is_err()
        );
    }
    #[test]
    fn successful_installer_logs_are_not_a_json_response() {
        let mut command = Command::new("sh");
        command.args(["-c", "dd if=/dev/zero bs=1024 count=70 2>/dev/null"]);
        assert!(run(&mut command, Duration::from_secs(2)).is_ok());
    }
    #[test]
    fn hanging_process_is_bounded() {
        let start = Instant::now();
        assert!(
            capture(
                Command::new("sh").args(["-c", "sleep 20"]),
                Duration::from_millis(100)
            )
            .is_err()
        );
        assert!(start.elapsed() < Duration::from_secs(2));
    }
}
