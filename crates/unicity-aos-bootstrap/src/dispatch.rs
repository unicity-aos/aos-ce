//! Runtime passthrough, argument routing, and stop coordination helpers.

use std::ffi::OsString;
use std::io::{self, Write};
use std::process::{ExitCode, ExitStatus};
use std::time::Duration;

use unicity_aos_bootstrap::AosHome;

pub(crate) fn resolve_home() -> Result<AosHome, ExitCode> {
    AosHome::resolve().map_err(|error| {
        eprintln!("aos: failed to resolve product home: {error}");
        ExitCode::FAILURE
    })
}

pub(crate) fn child_exit_code(status: ExitStatus) -> ExitCode {
    if status.success() {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(status.code().unwrap_or(1).clamp(1, i32::from(u8::MAX)) as u8)
    }
}

pub(crate) fn runtime_args_for_dispatch(args: Vec<OsString>) -> Vec<OsString> {
    args
}

pub(crate) fn runtime_stop_requested(args: &[OsString]) -> bool {
    match leading_runtime_root_index(args) {
        Ok(Some(index)) => args.get(index).is_some_and(|root| root == "stop"),
        Ok(None) => false,
        Err(()) => fallback_runtime_root(args).is_some_and(|root| root == "stop"),
    }
}

fn fallback_runtime_root(args: &[OsString]) -> Option<&str> {
    args.iter().filter_map(|arg| arg.to_str()).find(|arg| {
        matches!(
            *arg,
            "chat"
                | "run"
                | "agent"
                | "group"
                | "caps"
                | "quota"
                | "invite"
                | "keypair"
                | "pair-device"
                | "secret"
                | "voucher"
                | "trust"
                | "audit"
                | "budget"
                | "session"
                | "capsule"
                | "mcp"
                | "distro"
                | "init"
                | "config"
                | "gc"
                | "start"
                | "status"
                | "stop"
                | "restart"
                | "logs"
                | "ps"
                | "top"
                | "who"
                | "doctor"
                | "setup"
                | "version"
                | "completions"
                | "update"
                | "self-update"
                | "self_update"
                | "help"
        )
    })
}

pub(crate) fn handle_runtime_stop(args: &[OsString]) -> ExitCode {
    let home = match resolve_home() {
        Ok(home) => home,
        Err(code) => return code,
    };
    let output = match home
        .ensure_runtime_available()
        .and_then(|()| home.runtime_command_with_args(args))
        .and_then(|mut command| command.output())
    {
        Ok(output) => output,
        Err(error) => {
            eprintln!("aos: failed to run bundled runtime stop: {error}");
            return ExitCode::FAILURE;
        }
    };

    let expected_disconnect = expected_shutdown_disconnect(&output);
    let confirmation = wait_for_confirmed_stop(&home);
    if confirmation.is_ok() && (output.status.success() || expected_disconnect) {
        if let Err(error) = std::io::stdout().write_all(&output.stdout) {
            return runtime_output_error(error);
        }
        if output.stdout.is_empty() {
            println!("Unicity AOS stopped.");
        }
        return ExitCode::SUCCESS;
    }

    let output_result = emit_runtime_output(&output);
    if let Err(error) = confirmation {
        eprintln!("aos: shutdown confirmation failed: {error}");
    }
    if let Err(error) = output_result {
        return runtime_output_error(error);
    }
    if output.status.success() {
        ExitCode::FAILURE
    } else {
        child_exit_code(output.status)
    }
}

fn expected_shutdown_disconnect(output: &std::process::Output) -> bool {
    if output.status.code() != Some(1) {
        return false;
    }
    let stderr = String::from_utf8_lossy(&output.stderr);
    stderr.contains("connection lost waiting on astrid.v1.response.shutdown.")
        && stderr.contains("connection closed before astrid.v1.response.shutdown.")
}

fn wait_for_confirmed_stop(home: &AosHome) -> Result<(), String> {
    const ATTEMPTS: usize = 100;
    const INTERVAL: Duration = Duration::from_millis(50);

    let mut last_failure = "runtime stopped-state confirmation returned no result".to_owned();
    for attempt in 0..ATTEMPTS {
        match unicity_aos_bootstrap::status::confirm_stopped(home) {
            Ok(status) if status.state == "stopped" => return Ok(()),
            Ok(status) => {
                last_failure = format!(
                    "runtime stopped-state confirmation returned '{}'",
                    status.state
                );
            }
            Err(error) => last_failure = error,
        }
        if attempt + 1 < ATTEMPTS {
            std::thread::sleep(INTERVAL);
        }
    }
    Err(format!(
        "runtime did not reach a confirmed stopped state within 5 seconds: {last_failure}"
    ))
}

fn emit_runtime_output(output: &std::process::Output) -> io::Result<()> {
    std::io::stdout().write_all(&output.stdout)?;
    std::io::stderr().write_all(&output.stderr)?;
    Ok(())
}

fn runtime_output_error(error: io::Error) -> ExitCode {
    eprintln!("aos: failed to write bundled runtime output: {error}");
    ExitCode::FAILURE
}

pub(crate) fn is_owned_root(value: &str) -> bool {
    matches!(
        value,
        "init"
            | "status"
            | "migrate"
            | "update"
            | "self-update"
            | "self_update"
            | "distro"
            | "hook"
            | "mcp"
            | "daemon"
            | "serve-health"
    )
}

pub(crate) fn ambiguous_leading_principal(args: &[OsString]) -> Option<&str> {
    if args.first()?.to_str()? != "--principal" {
        return None;
    }
    let value = args.get(1)?.to_str().filter(|value| is_owned_root(value))?;
    let later_command = leading_runtime_root_index(args.get(2..).unwrap_or_default())
        .ok()
        .flatten()
        .is_some();
    (!later_command).then_some(value)
}

pub(crate) fn leading_owned_root(args: &[OsString]) -> Option<&str> {
    let first = args.first()?.to_str()?;
    if !first.starts_with('-') || matches!(first, "-h" | "--help" | "-V" | "--version") {
        return None;
    }

    match leading_runtime_root_index(args) {
        Ok(Some(index)) => args
            .get(index)
            .and_then(|arg| arg.to_str())
            .filter(|root| is_owned_root(root)),
        Ok(None) => None,
        Err(()) => args
            .iter()
            .skip(1)
            .filter_map(|arg| arg.to_str())
            .find(|candidate| is_owned_root(candidate)),
    }
}

fn leading_runtime_root_index(args: &[OsString]) -> Result<Option<usize>, ()> {
    let mut index = 0;
    while index < args.len() {
        let arg = args[index].to_str().ok_or(())?;
        if !arg.starts_with('-') {
            return Ok(Some(index));
        }
        if arg == "--" {
            return Ok((index + 1 < args.len()).then_some(index + 1));
        }
        if matches!(
            arg,
            "-v" | "--verbose"
                | "-y"
                | "--yes"
                | "--yolo"
                | "--autonomous"
                | "--print-session"
                | "--snapshot-tui"
                | "--emit-path"
        ) {
            index += 1;
            continue;
        }
        if matches!(
            arg,
            "--format"
                | "--principal"
                | "-p"
                | "--prompt"
                | "--session"
                | "--tui-width"
                | "--tui-height"
                | "--workspace-state-dir"
        ) {
            if index + 1 >= args.len() {
                return Err(());
            }
            index += 2;
            continue;
        }
        if [
            "--format=",
            "--principal=",
            "--prompt=",
            "--session=",
            "--tui-width=",
            "--tui-height=",
            "--workspace-state-dir=",
        ]
        .iter()
        .any(|prefix| arg.starts_with(prefix))
        {
            index += 1;
            continue;
        }
        if arg.starts_with("-p") && arg.len() > 2 {
            index += 1;
            continue;
        }
        return Err(());
    }
    Ok(None)
}
