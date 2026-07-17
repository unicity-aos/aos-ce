#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![allow(missing_docs)]

//! Principal-scoped command and workspace adapter for the first AOS Realm.

mod host;

use aos_realm_runtime::{
    CAT_GUEST, ECHO_GUEST, HostFault, PWD_GUEST, ProcessConfig, ProcessOutcome, RealmHost,
    RealmIoError, RealmRuntime, RunLimits, SMOKE_WRITE_GUEST, WRITE_FILE_GUEST,
};
use astrid_sdk::prelude::*;
use astrid_sdk::schemars;
use host::{
    AstridRealmHost, DEFAULT_CWD, REALM_NAME, ensure_layout, layout_initialized, validate_cwd,
};
use serde::{Deserialize, Serialize};

const HARD_MAX_FUEL: u64 = 100_000;
const HARD_MAX_OUTPUT_BYTES: usize = 64 * 1024;
const HARD_MEMORY_BYTES: usize = 64 * 1024;

#[derive(Default)]
pub struct LinuxRealm;

#[derive(Clone, Copy, Debug, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RealmProgram {
    SmokeWrite,
    Pwd,
    Echo,
    WriteFile,
    Cat,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecArgs {
    /// Signed program selection. Omit to use `command`, or omit both for `pwd`.
    pub program: Option<RealmProgram>,
    /// Exact command name. This is never evaluated by a host shell.
    pub command: Option<String>,
    /// Command arguments. Shell operators have no special meaning.
    #[serde(default)]
    pub args: Vec<String>,
    /// Guest-visible CWD beneath `/workspace`, `/home/agent`, or `/tmp`.
    pub cwd: Option<String>,
    /// Optional lower fuel ceiling. It can never raise the capsule hard limit.
    pub fuel: Option<u64>,
    /// Optional lower output ceiling. It can never raise the capsule hard limit.
    pub max_output_bytes: Option<usize>,
}

#[derive(Debug, Serialize)]
struct ExecResponse {
    realm: &'static str,
    owner_principal: String,
    program: String,
    argv: Vec<String>,
    cwd: String,
    outcome: &'static str,
    exit_status: Option<i32>,
    fault: Option<String>,
    stdout: String,
    stderr: String,
    fuel_consumed: u64,
    memory_limit_bytes: usize,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct StatusArgs {}

#[derive(Debug, Serialize)]
struct MountStatus {
    guest_path: &'static str,
    source: &'static str,
    mode: &'static str,
    durable: bool,
}

#[derive(Debug, Serialize)]
struct StatusResponse {
    realm: &'static str,
    owner_principal: String,
    state: &'static str,
    default_cwd: &'static str,
    home: &'static str,
    mounts: Vec<MountStatus>,
    commands: [&'static str; 5],
    workspace_commit: &'static str,
    host_process: bool,
}

#[derive(Debug)]
struct SelectedProgram {
    name: &'static str,
    guest: &'static [u8],
    argv: Vec<String>,
}

#[capsule]
impl LinuxRealm {
    /// Run one signed command in the caller's principal-scoped AOS Realm.
    ///
    /// `/workspace` maps to the invocation's confined Astrid copy-on-write
    /// `cwd://` mount; its changes require an outer Astrid promotion.
    /// `/home/agent` maps to durable principal-owned realm storage. Commands are
    /// nested core WebAssembly modules and cannot invoke a host shell or process.
    #[astrid::tool("linux_realm_exec", mutable)]
    pub fn exec(&self, args: ExecArgs) -> Result<String, SysError> {
        let principal = caller_principal()?;
        ensure_layout()?;
        let cwd = args.cwd.as_deref().unwrap_or(DEFAULT_CWD);
        validate_cwd(cwd)?;
        let response = run_command(args, principal, Box::<AstridRealmHost>::default())?;
        serde_json::to_string(&response).map_err(|error| SysError::ApiError(error.to_string()))
    }

    /// Inspect the initialized realm without exposing physical host paths.
    #[astrid::tool("linux_realm_status")]
    pub fn status(&self, _args: StatusArgs) -> Result<String, SysError> {
        let principal = caller_principal()?;
        let response = status_response(principal, layout_initialized()?);
        serde_json::to_string(&response).map_err(|error| SysError::ApiError(error.to_string()))
    }
}

fn caller_principal() -> Result<String, SysError> {
    astrid_sdk::runtime::caller()?
        .principal
        .filter(|principal| !principal.is_empty())
        .ok_or_else(|| SysError::ApiError("AOS Realm requires a stamped principal".to_string()))
}

fn run_command(
    args: ExecArgs,
    principal: String,
    realm_host: Box<dyn RealmHost>,
) -> Result<ExecResponse, SysError> {
    let selected = select_program(&args)?;
    let cwd = args.cwd.clone().unwrap_or_else(|| DEFAULT_CWD.to_string());
    let limits = RunLimits {
        fuel: args.fuel.unwrap_or(HARD_MAX_FUEL).min(HARD_MAX_FUEL),
        memory_bytes: HARD_MEMORY_BYTES,
        output_bytes: args
            .max_output_bytes
            .unwrap_or(HARD_MAX_OUTPUT_BYTES)
            .min(HARD_MAX_OUTPUT_BYTES),
    };
    let report = RealmRuntime::default()
        .execute_process(
            selected.guest,
            ProcessConfig {
                argv: selected.argv.clone(),
                cwd: cwd.clone(),
            },
            limits,
            realm_host,
        )
        .map_err(|error| SysError::ApiError(error.to_string()))?;
    let (outcome, exit_status, fault) = outcome_fields(&report.outcome);
    Ok(ExecResponse {
        realm: REALM_NAME,
        owner_principal: principal,
        program: selected.name.to_string(),
        argv: selected.argv,
        cwd,
        outcome,
        exit_status,
        fault,
        stdout: String::from_utf8_lossy(&report.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&report.stderr).into_owned(),
        fuel_consumed: report.fuel_consumed,
        memory_limit_bytes: report.memory_limit_bytes,
    })
}

fn select_program(args: &ExecArgs) -> Result<SelectedProgram, SysError> {
    if args.program.is_some() && args.command.is_some() {
        return Err(SysError::ApiError(
            "choose either program or command, not both".to_string(),
        ));
    }
    let program = if let Some(program) = args.program {
        program
    } else if let Some(command) = args.command.as_deref() {
        match command {
            "smoke-write" => RealmProgram::SmokeWrite,
            "pwd" => RealmProgram::Pwd,
            "echo" => RealmProgram::Echo,
            "write-file" => RealmProgram::WriteFile,
            "cat" => RealmProgram::Cat,
            _ => {
                return Err(SysError::ApiError(format!(
                    "unsupported realm command `{command}`; supported: pwd, echo, write-file, cat, smoke-write"
                )));
            }
        }
    } else {
        RealmProgram::Pwd
    };

    let (name, guest, argv) = match program {
        RealmProgram::SmokeWrite => {
            require_arity("smoke-write", &args.args, 0)?;
            (
                "smoke-write",
                SMOKE_WRITE_GUEST,
                vec!["smoke-write".to_string()],
            )
        }
        RealmProgram::Pwd => {
            require_arity("pwd", &args.args, 0)?;
            ("pwd", PWD_GUEST, vec!["pwd".to_string()])
        }
        RealmProgram::Echo => (
            "echo",
            ECHO_GUEST,
            vec!["echo".to_string(), args.args.join(" ")],
        ),
        RealmProgram::WriteFile => {
            require_arity("write-file", &args.args, 2)?;
            let mut argv = vec!["write-file".to_string()];
            argv.extend(args.args.iter().cloned());
            ("write-file", WRITE_FILE_GUEST, argv)
        }
        RealmProgram::Cat => {
            require_arity("cat", &args.args, 1)?;
            (
                "cat",
                CAT_GUEST,
                vec!["cat".to_string(), args.args[0].clone()],
            )
        }
    };
    Ok(SelectedProgram { name, guest, argv })
}

fn require_arity(command: &str, args: &[String], expected: usize) -> Result<(), SysError> {
    if args.len() == expected {
        Ok(())
    } else {
        Err(SysError::ApiError(format!(
            "{command} expects {expected} argument(s), received {}",
            args.len()
        )))
    }
}

fn status_response(principal: String, initialized: bool) -> StatusResponse {
    StatusResponse {
        realm: REALM_NAME,
        owner_principal: principal,
        state: if initialized {
            "ready"
        } else {
            "uninitialized"
        },
        default_cwd: DEFAULT_CWD,
        home: "/home/agent",
        mounts: vec![
            MountStatus {
                guest_path: "/home/agent",
                source: "principal-home",
                mode: "rw",
                durable: true,
            },
            MountStatus {
                guest_path: "/workspace",
                source: "invocation-cwd",
                mode: "rw",
                durable: false,
            },
            MountStatus {
                guest_path: "/tmp",
                source: "principal-tmp",
                mode: "rw",
                durable: false,
            },
        ],
        commands: ["pwd", "echo", "write-file", "cat", "smoke-write"],
        workspace_commit: "outer-astrid-promotion-required",
        host_process: false,
    }
}

fn outcome_fields(outcome: &ProcessOutcome) -> (&'static str, Option<i32>, Option<String>) {
    match outcome {
        ProcessOutcome::Exited(status) => ("exited", Some(*status), None),
        ProcessOutcome::FuelExhausted => {
            ("fuel-exhausted", None, Some("fuel exhausted".to_string()))
        }
        ProcessOutcome::HostFault(fault) => ("host-fault", None, Some(host_fault_name(*fault))),
        ProcessOutcome::Trapped(message) => ("trapped", None, Some(message.clone())),
    }
}

fn host_fault_name(fault: HostFault) -> String {
    match fault {
        HostFault::MissingMemory => "missing-memory".to_string(),
        HostFault::InvalidPointer => "invalid-pointer".to_string(),
        HostFault::UnknownDescriptor(_) => "unknown-descriptor".to_string(),
        HostFault::OutputLimit => "output-limit".to_string(),
        HostFault::MissingArgument => "missing-argument".to_string(),
        HostFault::BufferTooSmall => "buffer-too-small".to_string(),
        HostFault::InvalidUtf8 => "invalid-utf8".to_string(),
        HostFault::InvalidArgument => "invalid-argument".to_string(),
        HostFault::Io(error) => format!("io-{}", io_error_name(error)),
    }
}

fn io_error_name(error: RealmIoError) -> &'static str {
    match error {
        RealmIoError::NotFound => "not-found",
        RealmIoError::Denied => "denied",
        RealmIoError::InvalidPath => "invalid-path",
        RealmIoError::IsDirectory => "is-directory",
        RealmIoError::NotDirectory => "not-directory",
        RealmIoError::TooLarge => "too-large",
        RealmIoError::Unsupported => "unsupported",
        RealmIoError::Io => "failure",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct TestHost;

    impl RealmHost for TestHost {
        fn open(
            &mut self,
            _cwd: &str,
            _path: &str,
            _mode: aos_realm_runtime::OpenMode,
        ) -> Result<Box<dyn aos_realm_runtime::RealmFile>, RealmIoError> {
            Err(RealmIoError::Denied)
        }
    }

    #[test]
    fn command_runs_as_a_nested_guest_with_explicit_cwd() {
        let response = run_command(
            ExecArgs {
                command: Some("pwd".to_string()),
                cwd: Some("/workspace/project".to_string()),
                ..ExecArgs::default()
            },
            "alice".to_string(),
            Box::<TestHost>::default(),
        )
        .expect("realm command succeeds");

        assert_eq!(response.owner_principal, "alice");
        assert_eq!(response.outcome, "exited");
        assert_eq!(response.exit_status, Some(0));
        assert_eq!(response.stdout, "/workspace/project\n");
    }

    #[test]
    fn caller_can_only_reduce_fuel() {
        let response = run_command(
            ExecArgs {
                program: Some(RealmProgram::SmokeWrite),
                fuel: Some(u64::MAX),
                ..ExecArgs::default()
            },
            "alice".to_string(),
            Box::<TestHost>::default(),
        )
        .expect("realm command succeeds");

        assert!(response.fuel_consumed <= HARD_MAX_FUEL);
    }

    #[test]
    fn command_is_not_a_shell_command_line() {
        let error = select_program(&ExecArgs {
            command: Some("pwd && whoami".to_string()),
            ..ExecArgs::default()
        })
        .expect_err("shell syntax must not be interpreted");

        assert!(error.to_string().contains("unsupported realm command"));
    }

    #[test]
    fn forged_principal_field_is_not_part_of_the_input_contract() {
        let error =
            serde_json::from_str::<ExecArgs>(r#"{"command":"pwd","principal":"someone-else"}"#)
                .expect_err("unknown principal field must fail");

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn status_exposes_guest_mounts_without_physical_paths() {
        let json = serde_json::to_string(&status_response("alice".to_string(), true))
            .expect("status serializes");

        assert!(json.contains("/workspace"));
        assert!(json.contains("/home/agent"));
        assert!(json.contains("outer-astrid-promotion-required"));
        assert!(!json.contains("/Users/"));
        assert!(!json.contains(".astrid/home"));
    }

    #[test]
    fn actual_capsule_manifest_has_scoped_fs_and_no_host_process_authority() {
        let manifest: toml::Value = include_str!("../Capsule.toml")
            .parse()
            .expect("Capsule.toml parses");
        let capabilities = manifest["capabilities"]
            .as_table()
            .expect("capabilities is a table");

        assert!(!capabilities.contains_key("host_process"));
        assert!(capabilities.contains_key("fs_read"));
        assert!(capabilities.contains_key("fs_write"));
    }
}
