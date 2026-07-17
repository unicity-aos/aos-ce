#![deny(unsafe_code)]

//! Bounded execution of nested core WebAssembly processes.

use aos_realm_abi::{IMPORT_MODULE_V0, STDERR_FD, STDOUT_FD};
use std::{fmt, vec::Vec};
use wasmi::{
    Caller, Config, Engine, Error as WasmiError, Extern, Linker, Module, Store, StoreLimits,
    StoreLimitsBuilder, TrapCode,
};

/// Compiled smoke guest embedded into the capsule at build time.
pub const SMOKE_WRITE_GUEST: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/smoke_write.wasm"));

/// Hard limits for one nested process invocation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct RunLimits {
    /// Maximum interpreter fuel available to the process.
    pub fuel: u64,
    /// Maximum bytes in a single guest linear memory.
    pub memory_bytes: usize,
    /// Maximum combined bytes written to stdout and stderr.
    pub output_bytes: usize,
}

impl Default for RunLimits {
    fn default() -> Self {
        Self {
            fuel: 100_000,
            memory_bytes: 64 * 1024,
            output_bytes: 64 * 1024,
        }
    }
}

/// Terminal state of a nested process.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcessOutcome {
    /// Process called the realm `exit` import or returned from `_start`.
    Exited(i32),
    /// Process exhausted its deterministic instruction budget.
    FuelExhausted,
    /// Process violated a realm host-call boundary.
    HostFault(HostFault),
    /// Process trapped for another reason.
    Trapped(String),
}

/// Result and accounting for a process that was successfully launched.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecutionReport {
    /// Terminal process state.
    pub outcome: ProcessOutcome,
    /// Bytes written to guest stdout.
    pub stdout: Vec<u8>,
    /// Bytes written to guest stderr.
    pub stderr: Vec<u8>,
    /// Interpreter fuel consumed by the process and its host calls.
    pub fuel_consumed: u64,
    /// Linear-memory ceiling applied to this process.
    pub memory_limit_bytes: usize,
}

/// Failure before a process could start executing `_start`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LaunchError {
    /// Guest bytes are not a valid supported core WebAssembly module.
    InvalidModule(String),
    /// Guest imports, start behavior, or resource declarations cannot be admitted.
    Instantiation(String),
    /// Guest does not export `_start` with the required `() -> ()` signature.
    MissingStart(String),
    /// The runtime could not configure a required realm host import.
    RuntimeConfiguration(String),
}

impl fmt::Display for LaunchError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidModule(message) => write!(f, "invalid guest module: {message}"),
            Self::Instantiation(message) => write!(f, "guest instantiation denied: {message}"),
            Self::MissingStart(message) => write!(f, "guest _start is invalid: {message}"),
            Self::RuntimeConfiguration(message) => {
                write!(f, "realm runtime configuration failed: {message}")
            }
        }
    }
}

impl std::error::Error for LaunchError {}

/// Host-call violations exposed as stable realm faults.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HostFault {
    /// The process did not export the memory used by its pointers.
    MissingMemory,
    /// A guest pointer or length was negative, overflowing, or out of bounds.
    InvalidPointer,
    /// The process attempted to write a descriptor it does not own.
    UnknownDescriptor(i32),
    /// The process exceeded its combined stdout/stderr budget.
    OutputLimit,
}

impl fmt::Display for HostFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingMemory => f.write_str("guest has no exported memory"),
            Self::InvalidPointer => f.write_str("guest memory range is invalid"),
            Self::UnknownDescriptor(fd) => write!(f, "unknown guest descriptor {fd}"),
            Self::OutputLimit => f.write_str("guest output limit exceeded"),
        }
    }
}

impl std::error::Error for HostFault {}
impl wasmi::errors::HostError for HostFault {}

struct HostState {
    limits: StoreLimits,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
    output_limit: usize,
    monotonic_ns: i64,
}

impl HostState {
    fn output_len(&self) -> usize {
        self.stdout.len().saturating_add(self.stderr.len())
    }
}

/// Reference interpreter for the first AOS Realm guest ABI.
pub struct RealmRuntime {
    engine: Engine,
}

impl Default for RealmRuntime {
    fn default() -> Self {
        let mut config = Config::default();
        config.consume_fuel(true);
        Self {
            engine: Engine::new(&config),
        }
    }
}

impl RealmRuntime {
    /// Validates, instantiates, and runs one guest process.
    pub fn execute(&self, wasm: &[u8], limits: RunLimits) -> Result<ExecutionReport, LaunchError> {
        let module = Module::new(&self.engine, wasm)
            .map_err(|error| LaunchError::InvalidModule(error.to_string()))?;
        let store_limits = StoreLimitsBuilder::new()
            .instances(1)
            .memories(1)
            .tables(1)
            .memory_size(limits.memory_bytes)
            .trap_on_grow_failure(true)
            .build();
        let state = HostState {
            limits: store_limits,
            stdout: Vec::new(),
            stderr: Vec::new(),
            output_limit: limits.output_bytes,
            monotonic_ns: 0,
        };
        let mut store = Store::new(&self.engine, state);
        store.limiter(|state| &mut state.limits);
        store
            .set_fuel(limits.fuel)
            .map_err(|error| LaunchError::RuntimeConfiguration(error.to_string()))?;

        let mut linker = Linker::new(&self.engine);
        install_realm_v0(&mut linker)?;
        let instance = linker
            .instantiate_and_start(&mut store, &module)
            .map_err(|error| LaunchError::Instantiation(error.to_string()))?;
        let start = instance
            .get_typed_func::<(), ()>(&store, "_start")
            .map_err(|error| LaunchError::MissingStart(error.to_string()))?;

        let outcome = match start.call(&mut store, ()) {
            Ok(()) => ProcessOutcome::Exited(0),
            Err(error) => classify_process_error(&error),
        };
        let remaining_fuel = store.get_fuel().unwrap_or_default();
        let state = store.data();

        Ok(ExecutionReport {
            outcome,
            stdout: state.stdout.clone(),
            stderr: state.stderr.clone(),
            fuel_consumed: limits.fuel.saturating_sub(remaining_fuel),
            memory_limit_bytes: limits.memory_bytes,
        })
    }
}

fn install_realm_v0(linker: &mut Linker<HostState>) -> Result<(), LaunchError> {
    linker
        .func_wrap(
            IMPORT_MODULE_V0,
            "write",
            |mut caller: Caller<'_, HostState>, fd: i32, ptr: i32, len: i32| {
                realm_write(&mut caller, fd, ptr, len)
            },
        )
        .map_err(|error| LaunchError::RuntimeConfiguration(error.to_string()))?;
    linker
        .func_wrap(
            IMPORT_MODULE_V0,
            "clock-monotonic-ns",
            |caller: Caller<'_, HostState>| -> i64 { caller.data().monotonic_ns },
        )
        .map_err(|error| LaunchError::RuntimeConfiguration(error.to_string()))?;
    linker
        .func_wrap(
            IMPORT_MODULE_V0,
            "exit",
            |_caller: Caller<'_, HostState>, status: i32| -> Result<(), WasmiError> {
                Err(WasmiError::i32_exit(status))
            },
        )
        .map_err(|error| LaunchError::RuntimeConfiguration(error.to_string()))?;
    Ok(())
}

fn realm_write(
    caller: &mut Caller<'_, HostState>,
    fd: i32,
    ptr: i32,
    len: i32,
) -> Result<i32, WasmiError> {
    let offset = usize::try_from(ptr).map_err(|_| WasmiError::host(HostFault::InvalidPointer))?;
    let length = usize::try_from(len).map_err(|_| WasmiError::host(HostFault::InvalidPointer))?;
    let new_total = caller
        .data()
        .output_len()
        .checked_add(length)
        .ok_or_else(|| WasmiError::host(HostFault::OutputLimit))?;
    if new_total > caller.data().output_limit {
        return Err(WasmiError::host(HostFault::OutputLimit));
    }

    let memory = caller
        .get_export("memory")
        .and_then(Extern::into_memory)
        .ok_or_else(|| WasmiError::host(HostFault::MissingMemory))?;
    let mut bytes = vec![0; length];
    memory
        .read(&*caller, offset, &mut bytes)
        .map_err(|_| WasmiError::host(HostFault::InvalidPointer))?;

    match fd {
        STDOUT_FD => caller.data_mut().stdout.extend_from_slice(&bytes),
        STDERR_FD => caller.data_mut().stderr.extend_from_slice(&bytes),
        unknown => return Err(WasmiError::host(HostFault::UnknownDescriptor(unknown))),
    }
    Ok(len)
}

fn classify_process_error(error: &WasmiError) -> ProcessOutcome {
    if let Some(status) = error.i32_exit_status() {
        return ProcessOutcome::Exited(status);
    }
    if error.as_trap_code() == Some(TrapCode::OutOfFuel) {
        return ProcessOutcome::FuelExhausted;
    }
    if let Some(fault) = error.downcast_ref::<HostFault>() {
        return ProcessOutcome::HostFault(*fault);
    }
    ProcessOutcome::Trapped(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(wat_source: &str) -> Vec<u8> {
        wat::parse_str(wat_source).expect("valid test WAT")
    }

    #[test]
    fn smoke_guest_runs_behind_realm_imports() {
        let report = RealmRuntime::default()
            .execute(SMOKE_WRITE_GUEST, RunLimits::default())
            .expect("smoke guest launches");

        assert_eq!(report.outcome, ProcessOutcome::Exited(0));
        assert_eq!(report.stdout, b"hello from AOS Realm\n");
        assert!(report.stderr.is_empty());
        assert!(report.fuel_consumed > 0);
        assert_eq!(report.memory_limit_bytes, 64 * 1024);
    }

    #[test]
    fn malformed_module_is_rejected_before_launch() {
        let error = RealmRuntime::default()
            .execute(&[0x00], RunLimits::default())
            .expect_err("malformed bytes must fail");

        assert!(matches!(error, LaunchError::InvalidModule(_)));
    }

    #[test]
    fn undeclared_import_is_rejected() {
        let wasm = compile(
            r#"(module
                (import "host" "ambient" (func $ambient))
                (func (export "_start") (call $ambient)))"#,
        );
        let error = RealmRuntime::default()
            .execute(&wasm, RunLimits::default())
            .expect_err("ambient import must fail");

        assert!(matches!(error, LaunchError::Instantiation(_)));
    }

    #[test]
    fn out_of_bounds_pointer_becomes_stable_host_fault() {
        let wasm = compile(
            r#"(module
                (import "aos_realm_v0" "write"
                    (func $write (param i32 i32 i32) (result i32)))
                (memory (export "memory") 1 1)
                (func (export "_start")
                    (drop (call $write (i32.const 1) (i32.const 65535) (i32.const 2)))))"#,
        );
        let report = RealmRuntime::default()
            .execute(&wasm, RunLimits::default())
            .expect("guest launches");

        assert_eq!(
            report.outcome,
            ProcessOutcome::HostFault(HostFault::InvalidPointer)
        );
    }

    #[test]
    fn unknown_descriptor_is_rejected() {
        let wasm = compile(
            r#"(module
                (import "aos_realm_v0" "write"
                    (func $write (param i32 i32 i32) (result i32)))
                (memory (export "memory") 1 1)
                (data (i32.const 0) "x")
                (func (export "_start")
                    (drop (call $write (i32.const 9) (i32.const 0) (i32.const 1)))))"#,
        );
        let report = RealmRuntime::default()
            .execute(&wasm, RunLimits::default())
            .expect("guest launches");

        assert_eq!(
            report.outcome,
            ProcessOutcome::HostFault(HostFault::UnknownDescriptor(9))
        );
    }

    #[test]
    fn output_is_bounded_before_copying() {
        let mut limits = RunLimits::default();
        limits.output_bytes = 4;
        let report = RealmRuntime::default()
            .execute(SMOKE_WRITE_GUEST, limits)
            .expect("guest launches");

        assert_eq!(
            report.outcome,
            ProcessOutcome::HostFault(HostFault::OutputLimit)
        );
        assert!(report.stdout.is_empty());
    }

    #[test]
    fn infinite_guest_exhausts_fuel() {
        let wasm = compile(
            r#"(module
                (func (export "_start")
                    (loop $forever (br $forever))))"#,
        );
        let mut limits = RunLimits::default();
        limits.fuel = 100;
        let report = RealmRuntime::default()
            .execute(&wasm, limits)
            .expect("guest launches");

        assert_eq!(report.outcome, ProcessOutcome::FuelExhausted);
        assert_eq!(report.fuel_consumed, limits.fuel);
    }

    #[test]
    fn declared_memory_over_limit_is_rejected() {
        let wasm = compile(
            r#"(module
                (memory 2 2)
                (func (export "_start")))"#,
        );
        let error = RealmRuntime::default()
            .execute(&wasm, RunLimits::default())
            .expect_err("two pages exceed one-page limit");

        assert!(matches!(error, LaunchError::Instantiation(_)));
    }
}
