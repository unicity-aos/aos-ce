#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]
#![allow(missing_docs)]

//! Astrid tool adapter for the first bounded AOS Realm process.

use aos_realm_runtime::{HostFault, ProcessOutcome, RealmRuntime, RunLimits, SMOKE_WRITE_GUEST};
use astrid_sdk::prelude::*;
use astrid_sdk::schemars;
use serde::{Deserialize, Serialize};

const HARD_MAX_FUEL: u64 = 100_000;
const HARD_MAX_OUTPUT_BYTES: usize = 64 * 1024;
const HARD_MEMORY_BYTES: usize = 64 * 1024;

#[derive(Default)]
pub struct LinuxRealm;

#[derive(Clone, Copy, Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(rename_all = "kebab-case")]
pub enum RealmProgram {
    #[default]
    SmokeWrite,
}

#[derive(Debug, Default, Deserialize, schemars::JsonSchema)]
#[serde(deny_unknown_fields)]
pub struct ExecArgs {
    /// Program in the signed realm base image. The seed contains only smoke-write.
    #[serde(default)]
    pub program: RealmProgram,
    /// Optional lower fuel ceiling. It can never raise the capsule hard limit.
    pub fuel: Option<u64>,
    /// Optional lower output ceiling. It can never raise the capsule hard limit.
    pub max_output_bytes: Option<usize>,
}

#[derive(Debug, Serialize)]
struct ExecResponse {
    program: &'static str,
    outcome: &'static str,
    exit_status: Option<i32>,
    fault: Option<String>,
    stdout: String,
    stderr: String,
    fuel_consumed: u64,
    memory_limit_bytes: usize,
}

#[capsule]
impl LinuxRealm {
    /// Run one signed process from the principal's AOS Realm.
    ///
    /// The seed image contains only `smoke-write`. The process is a nested core
    /// WebAssembly module, receives only the private realm ABI, and cannot invoke
    /// a host shell or host process.
    #[astrid::tool("linux_realm_exec")]
    pub fn exec(&self, args: ExecArgs) -> Result<String, SysError> {
        let (program_name, guest) = match args.program {
            RealmProgram::SmokeWrite => ("smoke-write", SMOKE_WRITE_GUEST),
        };
        let limits = RunLimits {
            fuel: args.fuel.unwrap_or(HARD_MAX_FUEL).min(HARD_MAX_FUEL),
            memory_bytes: HARD_MEMORY_BYTES,
            output_bytes: args
                .max_output_bytes
                .unwrap_or(HARD_MAX_OUTPUT_BYTES)
                .min(HARD_MAX_OUTPUT_BYTES),
        };
        let report = RealmRuntime::default()
            .execute(guest, limits)
            .map_err(|error| SysError::ApiError(error.to_string()))?;
        let (outcome, exit_status, fault) = outcome_fields(&report.outcome);
        let response = ExecResponse {
            program: program_name,
            outcome,
            exit_status,
            fault,
            stdout: String::from_utf8_lossy(&report.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&report.stderr).into_owned(),
            fuel_consumed: report.fuel_consumed,
            memory_limit_bytes: report.memory_limit_bytes,
        };
        serde_json::to_string(&response).map_err(|error| SysError::ApiError(error.to_string()))
    }
}

fn outcome_fields(outcome: &ProcessOutcome) -> (&'static str, Option<i32>, Option<String>) {
    match outcome {
        ProcessOutcome::Exited(status) => ("exited", Some(*status), None),
        ProcessOutcome::FuelExhausted => {
            ("fuel-exhausted", None, Some("fuel exhausted".to_string()))
        }
        ProcessOutcome::HostFault(fault) => (
            "host-fault",
            None,
            Some(host_fault_name(*fault).to_string()),
        ),
        ProcessOutcome::Trapped(message) => ("trapped", None, Some(message.clone())),
    }
}

fn host_fault_name(fault: HostFault) -> &'static str {
    match fault {
        HostFault::MissingMemory => "missing-memory",
        HostFault::InvalidPointer => "invalid-pointer",
        HostFault::UnknownDescriptor(_) => "unknown-descriptor",
        HostFault::OutputLimit => "output-limit",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tool_runs_the_signed_smoke_program() {
        let json = LinuxRealm
            .exec(ExecArgs::default())
            .expect("realm tool succeeds");
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid response JSON");

        assert_eq!(value["outcome"], "exited");
        assert_eq!(value["exit_status"], 0);
        assert_eq!(value["stdout"], "hello from AOS Realm\n");
    }

    #[test]
    fn caller_can_only_reduce_fuel() {
        let json = LinuxRealm
            .exec(ExecArgs {
                fuel: Some(u64::MAX),
                ..ExecArgs::default()
            })
            .expect("realm tool succeeds");
        let value: serde_json::Value = serde_json::from_str(&json).expect("valid response JSON");

        assert!(value["fuel_consumed"].as_u64().expect("fuel is a u64") <= HARD_MAX_FUEL);
    }

    #[test]
    fn forged_principal_field_is_not_part_of_the_input_contract() {
        let error = serde_json::from_str::<ExecArgs>(
            r#"{"program":"smoke-write","principal":"someone-else"}"#,
        )
        .expect_err("unknown principal field must fail");

        assert!(error.to_string().contains("unknown field"));
    }

    #[test]
    fn actual_capsule_manifest_has_no_host_process_authority() {
        let manifest: toml::Value = include_str!("../Capsule.toml")
            .parse()
            .expect("Capsule.toml parses");
        let capabilities = manifest["capabilities"]
            .as_table()
            .expect("capabilities is a table");

        assert!(!capabilities.contains_key("host_process"));
    }
}
