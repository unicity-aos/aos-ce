//! Private full-system backend boundary.
//!
//! Native conformance tests retain the AOS-owned reference interpreter in the
//! controller process. Production capsules place that exact machine behind a
//! signed Astrid compute worker, leaving 9P and every other host effect in the
//! principal-affine controller component.

#[cfg(any(not(target_arch = "wasm32"), test))]
use aos_realm_machine::MachineConfig;
use aos_realm_machine::{HostRequestFailure, MAX_HARTS};
#[cfg(any(target_arch = "wasm32", test))]
use aos_realm_vcpu_protocol as protocol;

#[cfg(target_arch = "wasm32")]
use aos_realm_vcpu_protocol::{Operation, Outcome, RequestFailure, Status, field};

#[cfg(not(target_arch = "wasm32"))]
use aos_realm_machine::{HostRequestId, Machine, SliceOutcome};
#[cfg(target_arch = "wasm32")]
use astrid_sdk::compute::{ComputeGroup, GroupRequest, Parallelism, WorkDescriptor};

/// Stable identity of the selected Linux machine implementation.
///
/// Compute changes where the interpreter executes, not the machine semantics
/// exposed to tools, traces, checkpoints, or differential tests.
pub(crate) const DEFAULT_LINUX_BACKEND_ID: &str = "aos-rv64-interpreter";
/// Smallest measured RAM envelope that boots the development guest.
///
/// This is a machine viability floor, not an automatic default or policy cap.
const MIN_BOOT_RAM_BYTES: u64 = 512 * 1024 * 1024;
#[cfg(any(target_arch = "wasm32", test))]
const AUTO_PREWARM_HARTS: u32 = 1;
#[cfg(not(target_arch = "wasm32"))]
const LINUX_SYSTEM_BLOCK_CHANNEL: u32 = 3;
#[cfg(any(target_arch = "wasm32", test))]
const PREWARM_LOCK: &str = include_str!("../linux/PREWARM.lock");

#[cfg(not(target_arch = "wasm32"))]
const LINUX_IMAGE: &[u8] = include_bytes!("../assets/linux-kernel.img");
#[cfg(not(target_arch = "wasm32"))]
const LINUX_SYSTEM_PATH: &str =
    concat!(env!("CARGO_MANIFEST_DIR"), "/assets/linux-system.squashfs");

pub(crate) fn wall_time_seconds() -> Result<u64, String> {
    #[cfg(target_arch = "wasm32")]
    let now =
        astrid_sdk::time::now().map_err(|error| format!("read admitted wall clock: {error}"))?;
    #[cfg(not(target_arch = "wasm32"))]
    let now = std::time::SystemTime::now();
    now.duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_secs())
        .map_err(|_| "wall clock is before the Unix epoch".to_string())
}

#[cfg(not(target_arch = "wasm32"))]
fn linux_bootargs(wall_time_seconds: u64, system_bytes: usize) -> String {
    format!(
        "earlycon=sbi console=hvc0 init=/init panic=-1 aos.wall_time={wall_time_seconds} aos.system_bytes={system_bytes}"
    )
}

/// Normalized host request produced by the machine backend.
#[derive(Debug)]
pub(crate) struct LinuxHostRequest {
    pub(crate) id: u64,
    pub(crate) channel: u32,
    pub(crate) message: Vec<u8>,
    pub(crate) max_response_bytes: usize,
}

/// Normalized scheduling result independent of its execution substrate.
#[derive(Debug)]
pub(crate) enum LinuxSliceOutcome {
    Yielded,
    Halted { passed: bool, code: u32 },
    HostRequest(LinuxHostRequest),
    Trapped(String),
}

/// Normalized accounting and output for one machine slice.
#[derive(Debug)]
pub(crate) struct LinuxSliceReport {
    pub(crate) outcome: LinuxSliceOutcome,
    pub(crate) console: Vec<u8>,
    pub(crate) steps_executed: u64,
}

/// Principal-resident full-system Linux machine.
#[derive(Debug)]
pub(crate) enum LinuxMachine {
    #[cfg(not(target_arch = "wasm32"))]
    Reference(ReferenceMachine),
    #[cfg(target_arch = "wasm32")]
    Compute(ComputeMachine),
}

/// Guest resources selected by the principal and admitted by Astrid.
///
/// This lives in the wasm32 controller, so guest RAM is deliberately u64:
/// the controller's own address space must not cap a memory64 compute worker.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct LinuxMachineConfig {
    pub(crate) ram_bytes: u64,
    pub(crate) max_console_bytes: usize,
}

#[cfg(not(target_arch = "wasm32"))]
#[derive(Debug)]
pub(crate) struct ReferenceMachine {
    machine: Machine,
    pending_request: Option<HostRequestId>,
    system: Vec<u8>,
    shell_v2: bool,
    ram_bytes: u64,
    hart_count: u32,
}

impl LinuxMachine {
    /// Admit and initialize the production backend for this build target.
    pub(crate) fn new(
        config: LinuxMachineConfig,
        configured_hart_count: u32,
    ) -> Result<Self, String> {
        let wall_time_seconds = wall_time_seconds()?;
        #[cfg(not(target_arch = "wasm32"))]
        {
            let mut config = config;
            let hart_count = reference_hart_count(configured_hart_count)?;
            if config.ram_bytes == 0 {
                // Native execution is the conformance/reference lane and has
                // no Astrid compute admission query. Production wasm resolves
                // auto sizing from its admitted group.
                config.ram_bytes = MIN_BOOT_RAM_BYTES;
            }
            let machine_config = MachineConfig {
                ram_bytes: usize::try_from(config.ram_bytes)
                    .map_err(|_| "Linux guest RAM exceeds the reference host address space")?,
                max_console_bytes: config.max_console_bytes,
            };
            let mut machine = Machine::new_with_harts(machine_config, hart_count as usize)
                .map_err(|error| error.to_string())?;
            let (system, shell_v2) = std::fs::read(LINUX_SYSTEM_PATH)
                .map(|system| (system, true))
                .unwrap_or_else(|_| (vec![0; 4096], false));
            machine
                .boot_linux(
                    LINUX_IMAGE,
                    &[],
                    &linux_bootargs(wall_time_seconds, system.len()),
                )
                .map_err(|error| error.to_string())?;
            Ok(Self::Reference(ReferenceMachine {
                machine,
                pending_request: None,
                system,
                shell_v2,
                ram_bytes: config.ram_bytes,
                hart_count,
            }))
        }

        #[cfg(target_arch = "wasm32")]
        ComputeMachine::new(config, configured_hart_count, wall_time_seconds).map(Self::Compute)
    }

    #[cfg(test)]
    pub(crate) fn new_reference(config: MachineConfig) -> Result<Self, String> {
        let ram_bytes = u64::try_from(config.ram_bytes).map_err(|_| "reference RAM exceeds u64")?;
        Machine::new(config)
            .map(|machine| {
                Self::Reference(ReferenceMachine {
                    machine,
                    pending_request: None,
                    system: Vec::new(),
                    shell_v2: false,
                    ram_bytes,
                    hart_count: 1,
                })
            })
            .map_err(|error| error.to_string())
    }

    #[cfg(test)]
    pub(crate) fn new_reference_with_image(
        config: MachineConfig,
        hart_count: u32,
        image: &[u8],
        system: &[u8],
    ) -> Result<Self, String> {
        let hart_count = reference_hart_count(hart_count)?;
        let wall_time_seconds = wall_time_seconds()?;
        let ram_bytes = u64::try_from(config.ram_bytes).map_err(|_| "reference RAM exceeds u64")?;
        let mut machine = Machine::new_with_harts(config, hart_count as usize)
            .map_err(|error| error.to_string())?;
        machine
            .boot_linux(image, &[], &linux_bootargs(wall_time_seconds, system.len()))
            .map_err(|error| error.to_string())?;
        Ok(Self::Reference(ReferenceMachine {
            machine,
            pending_request: None,
            system: system.to_vec(),
            shell_v2: !system.is_empty(),
            ram_bytes,
            hart_count,
        }))
    }

    pub(crate) const fn backend_id(&self) -> &'static str {
        DEFAULT_LINUX_BACKEND_ID
    }

    pub(crate) const fn ram_bytes(&self) -> u64 {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Reference(reference) => reference.ram_bytes,
            #[cfg(target_arch = "wasm32")]
            Self::Compute(compute) => compute.ram_bytes,
        }
    }

    pub(crate) const fn hart_count(&self) -> u32 {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Reference(reference) => reference.hart_count,
            #[cfg(target_arch = "wasm32")]
            Self::Compute(compute) => compute.hart_count,
        }
    }

    pub(crate) const fn supports_shell_v2(&self) -> bool {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Reference(reference) => reference.shell_v2,
            #[cfg(target_arch = "wasm32")]
            Self::Compute(_) => true,
        }
    }

    pub(crate) fn push_console_input(&mut self, bytes: &[u8]) -> Result<(), String> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Reference(reference) => {
                reference.machine.push_console_input(bytes);
                Ok(())
            }
            #[cfg(target_arch = "wasm32")]
            Self::Compute(compute) => compute.push_console_input(bytes),
        }
    }

    pub(crate) fn run_slice(
        &mut self,
        instruction_budget: u64,
    ) -> Result<LinuxSliceReport, String> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Reference(reference) => reference.run_slice(instruction_budget),
            #[cfg(target_arch = "wasm32")]
            Self::Compute(compute) => compute.run_slice(instruction_budget),
        }
    }

    pub(crate) fn complete_9p_request(&mut self, id: u64, response: &[u8]) -> Result<(), String> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Reference(reference) => reference.complete_9p_request(id, response),
            #[cfg(target_arch = "wasm32")]
            Self::Compute(compute) => compute.complete_9p_request(id, response),
        }
    }

    pub(crate) fn fail_9p_request(
        &mut self,
        id: u64,
        failure: HostRequestFailure,
    ) -> Result<(), String> {
        match self {
            #[cfg(not(target_arch = "wasm32"))]
            Self::Reference(reference) => reference.fail_9p_request(id, failure),
            #[cfg(target_arch = "wasm32")]
            Self::Compute(compute) => compute.fail_9p_request(id, failure),
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl ReferenceMachine {
    fn run_slice(&mut self, instruction_budget: u64) -> Result<LinuxSliceReport, String> {
        let mut steps_executed = 0_u64;
        let mut console = Vec::new();
        loop {
            let report = self.machine.run_slice(instruction_budget - steps_executed);
            steps_executed = steps_executed.saturating_add(report.steps_executed);
            console.extend_from_slice(&report.console);
            if let SliceOutcome::HostRequest(request) = &report.outcome
                && request.channel == LINUX_SYSTEM_BLOCK_CHANNEL
            {
                let offset = request
                    .message
                    .as_slice()
                    .try_into()
                    .map(u64::from_le_bytes)
                    .map_err(|_| "Linux system block request has an invalid offset".to_string())?;
                let offset = usize::try_from(offset)
                    .map_err(|_| "Linux system block offset is too large".to_string())?;
                let end = offset
                    .checked_add(request.max_response_bytes)
                    .filter(|end| *end <= self.system.len())
                    .ok_or_else(|| {
                        "Linux system block read exceeds the immutable image".to_string()
                    })?;
                self.machine
                    .complete_9p_request(request.id, &self.system[offset..end])
                    .map_err(|error| error.to_string())?;
                if steps_executed < instruction_budget {
                    continue;
                }
                return Ok(LinuxSliceReport {
                    outcome: LinuxSliceOutcome::Yielded,
                    console,
                    steps_executed,
                });
            }
            let outcome = match report.outcome {
                SliceOutcome::Yielded => LinuxSliceOutcome::Yielded,
                SliceOutcome::Halted(status) => LinuxSliceOutcome::Halted {
                    passed: status.passed,
                    code: status.code,
                },
                SliceOutcome::HostRequest(request) => {
                    self.pending_request = Some(request.id);
                    LinuxSliceOutcome::HostRequest(LinuxHostRequest {
                        id: request.id.get(),
                        channel: request.channel,
                        message: request.message,
                        max_response_bytes: request.max_response_bytes,
                    })
                }
                SliceOutcome::Trapped(trap) => LinuxSliceOutcome::Trapped(trap.to_string()),
            };
            return Ok(LinuxSliceReport {
                outcome,
                console,
                steps_executed,
            });
        }
    }

    fn pending_request(&self, raw: u64) -> Result<HostRequestId, String> {
        let id = self
            .pending_request
            .ok_or_else(|| "Linux machine has no pending 9P request".to_string())?;
        if id.get() != raw {
            return Err("Linux machine 9P request identity mismatch".to_string());
        }
        Ok(id)
    }

    fn complete_9p_request(&mut self, raw: u64, response: &[u8]) -> Result<(), String> {
        let id = self.pending_request(raw)?;
        self.machine
            .complete_9p_request(id, response)
            .map_err(|error| error.to_string())?;
        self.pending_request = None;
        Ok(())
    }

    fn fail_9p_request(&mut self, raw: u64, failure: HostRequestFailure) -> Result<(), String> {
        let id = self.pending_request(raw)?;
        self.machine
            .fail_9p_request(id, failure)
            .map_err(|error| error.to_string())?;
        self.pending_request = None;
        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
#[derive(Debug)]
pub(crate) struct ComputeMachine {
    group: ComputeGroup,
    control_offset: u64,
    ram_bytes: u64,
    hart_count: u32,
}

#[cfg(target_arch = "wasm32")]
#[derive(Debug)]
struct ComputeResponse {
    header: Vec<u8>,
    payload: Vec<u8>,
}

#[cfg(any(target_arch = "wasm32", test))]
struct PrewarmSpec {
    ram_bytes: u64,
    max_console_bytes: usize,
    hart_count: u32,
    binding: [u8; protocol::CHECKPOINT_BINDING_BYTES],
}

#[cfg(any(target_arch = "wasm32", test))]
impl PrewarmSpec {
    fn matches(&self, config: LinuxMachineConfig, hart_count: u32) -> bool {
        self.ram_bytes == config.ram_bytes
            && self.max_console_bytes == config.max_console_bytes
            && self.hart_count == hart_count
    }
}

#[cfg(target_arch = "wasm32")]
impl ComputeMachine {
    fn new(
        mut config: LinuxMachineConfig,
        configured_hart_count: u32,
        wall_time_seconds: u64,
    ) -> Result<Self, String> {
        let mut hart_count = explicit_hart_count(configured_hart_count)?;
        let auto_memory = config.ram_bytes == 0;
        let (initial_pages, mut maximum_pages, mut control_offset) =
            shared_memory_layout(config.ram_bytes)?;
        if auto_memory || hart_count.is_none() {
            // A short-lived generic-compute probe exposes the current
            // principal/host intersection without adding a Linux-specific host
            // call. Automatic CPU topology still selects the distributed
            // prewarm shape; an explicit multi-hart topology reserves exact
            // parallel workers below.
            let probe_maximum_pages = if auto_memory { 0 } else { initial_pages };
            let probe = open_compute_probe(initial_pages, probe_maximum_pages)?;
            let info = probe
                .info()
                .map_err(|error| format!("inspect admitted Linux vCPU group: {error}"))?;
            if auto_memory {
                config.ram_bytes = auto_guest_ram_bytes(info.maximum_memory_pages)?;
            }
            if hart_count.is_none() {
                hart_count = Some(auto_guest_hart_count(info.parallelism));
            }
            drop(probe);
            let (_, admitted_maximum_pages, admitted_control_offset) =
                shared_memory_layout(config.ram_bytes)?;
            maximum_pages = admitted_maximum_pages;
            control_offset = admitted_control_offset;
        }
        let hart_count = hart_count.ok_or_else(|| {
            "Linux vCPU auto admission did not produce a logical CPU count".to_string()
        })?;
        let group = open_compute_group(initial_pages, maximum_pages, hart_count)?;
        let machine = Self {
            group,
            control_offset,
            ram_bytes: config.ram_bytes,
            hart_count,
        };
        let prewarm = prewarm_spec()?;
        let (operation, input) = if prewarm.matches(config, hart_count) {
            (Operation::InitCheckpoint, prewarm.binding.to_vec())
        } else {
            (
                Operation::InitCold,
                wall_time_seconds.to_le_bytes().to_vec(),
            )
        };
        machine.invoke(operation, &input, |header| {
            protocol::write_u64(header, field::RAM_BYTES, config.ram_bytes);
            protocol::write_u64(
                header,
                field::MAX_CONSOLE_BYTES,
                config.max_console_bytes as u64,
            );
            protocol::write_u32(header, field::HART_COUNT, hart_count);
        })?;
        if hart_count > 1 {
            machine.invoke(Operation::PrepareParallel, &[], |header| {
                protocol::write_u32(header, field::HART_COUNT, hart_count);
            })?;
        }
        Ok(machine)
    }

    fn push_console_input(&self, bytes: &[u8]) -> Result<(), String> {
        self.invoke(Operation::PushConsole, bytes, |_| {})?;
        Ok(())
    }

    fn complete_9p_request(&self, id: u64, response: &[u8]) -> Result<(), String> {
        self.invoke(Operation::Complete9p, response, |header| {
            protocol::write_u64(header, field::REQUEST_ID, id);
        })?;
        Ok(())
    }

    fn fail_9p_request(&self, id: u64, failure: HostRequestFailure) -> Result<(), String> {
        self.invoke(Operation::Fail9p, &[], |header| {
            protocol::write_u64(header, field::REQUEST_ID, id);
            protocol::write_u32(
                header,
                field::REQUEST_FAILURE,
                match failure {
                    HostRequestFailure::Failed => RequestFailure::Failed as u32,
                    HostRequestFailure::Denied => RequestFailure::Denied as u32,
                },
            );
        })?;
        Ok(())
    }

    fn run_slice(&self, instruction_budget: u64) -> Result<LinuxSliceReport, String> {
        if self.hart_count > 1 {
            return self.run_parallel_slice(instruction_budget);
        }
        let response = self.invoke(Operation::RunSlice, &[], |header| {
            protocol::write_u64(header, field::SLICE_BUDGET, instruction_budget);
        })?;
        Self::decode_slice(response)
    }

    fn run_parallel_slice(&self, instruction_budget: u64) -> Result<LinuxSliceReport, String> {
        if instruction_budget == 0 {
            return Ok(LinuxSliceReport {
                outcome: LinuxSliceOutcome::Yielded,
                console: Vec::new(),
                steps_executed: 0,
            });
        }
        let worker_count = u64::from(self.hart_count).min(instruction_budget);
        let base_budget = instruction_budget / worker_count;
        let remainder = instruction_budget % worker_count;
        let mut work = Vec::new();
        work.try_reserve_exact(worker_count as usize)
            .map_err(|_| "Linux vCPU epoch allocation was denied".to_string())?;

        for hart_id in 0..worker_count {
            let budget = base_budget + u64::from(hart_id < remainder);
            let hart_id = u32::try_from(hart_id)
                .map_err(|_| "Linux vCPU hart identity is too large".to_string())?;
            let control_offset = protocol::control_offset(hart_id as usize)
                .ok_or_else(|| "Linux vCPU hart has no control descriptor".to_string())?;
            let request = Self::request(Operation::RunHartSlice, &[], |header| {
                protocol::write_u32(header, field::HART_ID, hart_id);
                protocol::write_u64(header, field::SLICE_BUDGET, budget);
            })?;
            self.group
                .write(control_offset, &request)
                .map_err(|error| format!("write Linux vCPU hart {hart_id} request: {error}"))?;
            work.push((
                hart_id,
                control_offset,
                WorkDescriptor {
                    offset: control_offset,
                    length: protocol::CONTROL_BYTES as u64,
                    tag: Operation::RunHartSlice as u64,
                    worker_index: Some(hart_id),
                    fuel: None,
                },
            ));
        }

        let mut jobs = Vec::new();
        jobs.try_reserve_exact(work.len())
            .map_err(|_| "Linux vCPU job allocation was denied".to_string())?;
        for (hart_id, control_offset, descriptor) in work {
            match self.group.submit(descriptor) {
                Ok(job) => jobs.push((hart_id, control_offset, job)),
                Err(error) => {
                    for (_, _, job) in jobs {
                        let _ = job.join();
                    }
                    return Err(format!("submit Linux vCPU hart {hart_id}: {error}"));
                }
            }
        }

        let mut reports = Vec::new();
        reports
            .try_reserve_exact(jobs.len())
            .map_err(|_| "Linux vCPU response allocation was denied".to_string())?;
        let mut first_error = None;
        for (hart_id, control_offset, job) in jobs {
            let result = match job.join() {
                Ok(result) => result,
                Err(error) => {
                    first_error
                        .get_or_insert_with(|| format!("join Linux vCPU hart {hart_id}: {error}"));
                    continue;
                }
            };
            if result.worker_status != 0 {
                first_error.get_or_insert_with(|| {
                    format!(
                        "Linux vCPU hart {hart_id} transport returned status {}",
                        result.worker_status
                    )
                });
                continue;
            }
            match self
                .read_response(control_offset)
                .and_then(Self::decode_slice)
            {
                Ok(report) => reports.push(report),
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        if let Some(error) = first_error {
            return Err(error);
        }

        let console = self
            .invoke(Operation::DrainConsole, &[], |_| {})
            .and_then(Self::decode_console)?;
        let mut steps_executed = 0_u64;
        let mut outcome = None;
        for report in reports {
            steps_executed = steps_executed.saturating_add(report.steps_executed);
            debug_assert!(report.console.is_empty());
            if outcome.is_none() && !matches!(&report.outcome, LinuxSliceOutcome::Yielded) {
                outcome = Some(report.outcome);
            }
        }
        Ok(LinuxSliceReport {
            outcome: outcome.unwrap_or(LinuxSliceOutcome::Yielded),
            console,
            steps_executed,
        })
    }

    fn decode_console(response: ComputeResponse) -> Result<Vec<u8>, String> {
        let console_len = field_len(&response.header, field::CONSOLE_LEN)?;
        let message_len = field_len(&response.header, field::MESSAGE_LEN)?;
        let error_len = field_len(&response.header, field::ERROR_LEN)?;
        if message_len != 0 || error_len != 0 || response.payload.len() != console_len {
            return Err("Linux vCPU console drain returned an invalid payload".to_string());
        }
        Ok(response.payload)
    }

    fn decode_slice(response: ComputeResponse) -> Result<LinuxSliceReport, String> {
        let console_len = field_len(&response.header, field::CONSOLE_LEN)?;
        let message_len = field_len(&response.header, field::MESSAGE_LEN)?;
        let error_len = field_len(&response.header, field::ERROR_LEN)?;
        let console_end = console_len;
        let message_end = console_end
            .checked_add(message_len)
            .ok_or_else(|| "Linux vCPU response range overflow".to_string())?;
        let error_end = message_end
            .checked_add(error_len)
            .ok_or_else(|| "Linux vCPU response range overflow".to_string())?;
        if error_end != response.payload.len() {
            return Err("Linux vCPU response fields do not match its payload".to_string());
        }
        let outcome = protocol::read_u32(&response.header, field::OUTCOME)
            .and_then(|value| Outcome::try_from(value).ok())
            .ok_or_else(|| "Linux vCPU returned an unknown scheduling outcome".to_string())?;
        let outcome = match outcome {
            Outcome::Yielded => LinuxSliceOutcome::Yielded,
            Outcome::Halted => LinuxSliceOutcome::Halted {
                passed: protocol::read_u32(&response.header, field::HALT_PASSED)
                    .unwrap_or_default()
                    != 0,
                code: protocol::read_u32(&response.header, field::HALT_CODE).unwrap_or_default(),
            },
            Outcome::HostRequest => LinuxSliceOutcome::HostRequest(LinuxHostRequest {
                id: required_u64(&response.header, field::REQUEST_ID, "request id")?,
                channel: required_u32(&response.header, field::REQUEST_CHANNEL, "request channel")?,
                message: response.payload[console_end..message_end].to_vec(),
                max_response_bytes: field_len(&response.header, field::MAX_RESPONSE_BYTES)?,
            }),
            Outcome::Trapped => LinuxSliceOutcome::Trapped(
                String::from_utf8_lossy(&response.payload[message_end..error_end]).into_owned(),
            ),
            Outcome::None => {
                return Err("Linux vCPU run returned no scheduling outcome".to_string());
            }
        };
        Ok(LinuxSliceReport {
            outcome,
            console: response.payload[..console_end].to_vec(),
            steps_executed: required_u64(&response.header, field::STEPS_EXECUTED, "slice steps")?,
        })
    }

    fn request(
        operation: Operation,
        input: &[u8],
        configure: impl FnOnce(&mut [u8]),
    ) -> Result<Vec<u8>, String> {
        let request_len = protocol::HEADER_BYTES
            .checked_add(input.len())
            .ok_or_else(|| "Linux vCPU request range overflow".to_string())?;
        if request_len > protocol::CONTROL_BYTES {
            return Err("Linux vCPU request exceeds its bounded descriptor".to_string());
        }
        let mut request = vec![0_u8; request_len];
        protocol::write_u32(&mut request, field::MAGIC, protocol::MAGIC);
        protocol::write_u32(&mut request, field::VERSION, protocol::VERSION);
        protocol::write_u32(&mut request, field::OPERATION, operation as u32);
        protocol::write_u32(
            &mut request,
            field::INPUT_LEN,
            u32::try_from(input.len()).map_err(|_| "Linux vCPU input is too large".to_string())?,
        );
        configure(&mut request[..protocol::HEADER_BYTES]);
        request[protocol::HEADER_BYTES..].copy_from_slice(input);
        Ok(request)
    }

    fn invoke(
        &self,
        operation: Operation,
        input: &[u8],
        configure: impl FnOnce(&mut [u8]),
    ) -> Result<ComputeResponse, String> {
        let request = Self::request(operation, input, configure)?;
        self.group
            .write(self.control_offset, &request)
            .map_err(|error| format!("write Linux vCPU request: {error}"))?;
        let result = self
            .group
            .submit(WorkDescriptor {
                offset: self.control_offset,
                length: protocol::CONTROL_BYTES as u64,
                tag: operation as u64,
                worker_index: (self.hart_count > 1).then_some(0),
                fuel: None,
            })
            .map_err(|error| format!("submit Linux vCPU operation: {error}"))?
            .join()
            .map_err(|error| format!("join Linux vCPU operation: {error}"))?;
        if result.worker_status != 0 {
            return Err(format!(
                "Linux vCPU transport returned status {}",
                result.worker_status
            ));
        }
        self.read_response(self.control_offset)
    }

    fn read_response(&self, control_offset: u64) -> Result<ComputeResponse, String> {
        let header = self
            .group
            .read(control_offset, protocol::HEADER_BYTES as u32)
            .map_err(|error| format!("read Linux vCPU response header: {error}"))?;
        if protocol::read_u32(&header, field::MAGIC) != Some(protocol::MAGIC)
            || protocol::read_u32(&header, field::VERSION) != Some(protocol::VERSION)
        {
            return Err("Linux vCPU returned an invalid protocol envelope".to_string());
        }
        let response_len = field_len(&header, field::RESPONSE_LEN)?;
        if response_len > protocol::CONTROL_BYTES - protocol::HEADER_BYTES {
            return Err("Linux vCPU response exceeds its bounded descriptor".to_string());
        }
        let payload = if response_len == 0 {
            Vec::new()
        } else {
            self.group
                .read(
                    control_offset + protocol::HEADER_BYTES as u64,
                    response_len as u32,
                )
                .map_err(|error| format!("read Linux vCPU response payload: {error}"))?
        };
        let status = protocol::read_u32(&header, field::STATUS)
            .and_then(|value| Status::try_from(value).ok())
            .ok_or_else(|| "Linux vCPU returned an unknown operation status".to_string())?;
        if status != Status::Ok {
            let error_len = field_len(&header, field::ERROR_LEN)?.min(payload.len());
            let error_start = payload.len().saturating_sub(error_len);
            let detail = String::from_utf8_lossy(&payload[error_start..]);
            return Err(format!("Linux vCPU {status:?}: {detail}"));
        }
        Ok(ComputeResponse { header, payload })
    }
}

#[cfg(any(target_arch = "wasm32", test))]
fn prewarm_spec() -> Result<PrewarmSpec, String> {
    let value = |name: &str| {
        PREWARM_LOCK
            .lines()
            .find_map(|line| {
                line.strip_prefix(name)
                    .and_then(|line| line.strip_prefix('='))
            })
            .ok_or_else(|| format!("Linux prewarm lock is missing {name}"))
    };
    let ram_bytes = value("ram_bytes")?
        .parse::<u64>()
        .map_err(|error| format!("Linux prewarm RAM is invalid: {error}"))?;
    let max_console_bytes = value("max_console_bytes")?
        .parse::<usize>()
        .map_err(|error| format!("Linux prewarm console limit is invalid: {error}"))?;
    let hart_count = value("hart_count")?
        .parse::<u32>()
        .map_err(|error| format!("Linux prewarm hart count is invalid: {error}"))?;
    let mut binding = [0_u8; protocol::CHECKPOINT_BINDING_BYTES];
    decode_hex_digest(value("linux_image_blake3")?, &mut binding[..32])?;
    decode_hex_digest(value("system_image_blake3")?, &mut binding[32..])?;
    Ok(PrewarmSpec {
        ram_bytes,
        max_console_bytes,
        hart_count,
        binding,
    })
}

#[cfg(any(target_arch = "wasm32", test))]
fn decode_hex_digest(value: &str, output: &mut [u8]) -> Result<(), String> {
    if value.len() != output.len() * 2 {
        return Err("Linux prewarm digest has the wrong length".to_string());
    }
    for (byte, pair) in output.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        let nibble = |value: u8| match value {
            b'0'..=b'9' => Some(value - b'0'),
            b'a'..=b'f' => Some(value - b'a' + 10),
            _ => None,
        };
        let high = nibble(pair[0])
            .ok_or_else(|| "Linux prewarm digest is not lowercase hexadecimal".to_string())?;
        let low = nibble(pair[1])
            .ok_or_else(|| "Linux prewarm digest is not lowercase hexadecimal".to_string())?;
        *byte = (high << 4) | low;
    }
    Ok(())
}

#[cfg(target_arch = "wasm32")]
fn open_compute_group(
    initial_memory_pages: u32,
    maximum_memory_pages: u32,
    hart_count: u32,
) -> Result<ComputeGroup, String> {
    let request = GroupRequest::new(
        protocol::WORKER_ID,
        initial_memory_pages,
        maximum_memory_pages,
    );
    let request = if maximum_memory_pages == 0 {
        request.auto_memory()
    } else {
        request
    };
    let request = if hart_count == 1 {
        request.deterministic()
    } else {
        request.parallel(Parallelism::Exact(hart_count))
    };
    ComputeGroup::open(&request)
        .map_err(|error| format!("admit Linux vCPU compute worker: {error}"))
}

#[cfg(target_arch = "wasm32")]
fn open_compute_probe(
    initial_memory_pages: u32,
    maximum_memory_pages: u32,
) -> Result<ComputeGroup, String> {
    let request = GroupRequest::new(
        protocol::WORKER_ID,
        initial_memory_pages,
        maximum_memory_pages,
    )
    .parallel(Parallelism::Auto);
    let request = if maximum_memory_pages == 0 {
        request.auto_memory()
    } else {
        request
    };
    ComputeGroup::open(&request)
        .map_err(|error| format!("probe admitted Linux vCPU capacity: {error}"))
}

fn explicit_hart_count(configured: u32) -> Result<Option<u32>, String> {
    if configured == 0 {
        return Ok(None);
    }
    if configured > MAX_HARTS as u32 {
        return Err(format!(
            "Linux hart count must be between 1 and {MAX_HARTS}, got {configured}"
        ));
    }
    Ok(Some(configured))
}

#[cfg(any(target_arch = "wasm32", test))]
fn auto_guest_hart_count(_admitted_parallelism: u32) -> u32 {
    // The current signed prewarm has one hart. Auto preserves its near-instant
    // turn-time restore instead of silently choosing a cold parallel boot.
    // Explicit topologies above one use exact concurrent compute workers.
    AUTO_PREWARM_HARTS
}

#[cfg(not(target_arch = "wasm32"))]
fn reference_hart_count(configured: u32) -> Result<u32, String> {
    if let Some(count) = explicit_hart_count(configured)? {
        return Ok(count);
    }
    let available = std::thread::available_parallelism()
        .map_or(1, std::num::NonZeroUsize::get)
        .clamp(1, MAX_HARTS);
    u32::try_from(available).map_err(|_| "host CPU count is not addressable".to_string())
}

#[cfg(any(target_arch = "wasm32", test))]
fn shared_memory_layout(ram_bytes: u64) -> Result<(u32, u32, u64), String> {
    // Rust's wasm allocator acquires heap segments with `memory.grow`; it does
    // not adopt unused bytes from the imported memory's initial extent. Keep
    // the signed worker's code/data and descriptor in the fixed 64-MiB base,
    // then reserve guest RAM plus bounded allocator headroom as the maximum.
    // Astrid reserves that maximum before the worker can grow itself.
    let worker_minimum =
        u64::try_from(protocol::WORKER_MIN_MEMORY_BYTES).expect("worker minimum fits u64");
    let worker_heap =
        u64::try_from(protocol::WORKER_HEAP_OVERHEAD_BYTES).expect("worker heap overhead fits u64");
    let wasm_page = u64::try_from(protocol::WASM_PAGE_BYTES).expect("Wasm page size fits u64");
    let maximum_required = worker_minimum
        .checked_add(ram_bytes)
        .and_then(|value| value.checked_add(worker_heap))
        .ok_or_else(|| "Linux vCPU shared-memory size overflow".to_string())?
        .max(worker_minimum);
    let maximum_pages = if ram_bytes == 0 {
        // Zero is the held compute-contract sentinel for host admission. The
        // effective value is read back from `group.info()` before VM init.
        0
    } else {
        maximum_required
            .checked_add(wasm_page - 1)
            .ok_or_else(|| "Linux vCPU shared-memory rounding overflow".to_string())?
            / wasm_page
    };
    let maximum_bytes = maximum_pages
        .checked_mul(wasm_page)
        .ok_or_else(|| "Linux vCPU shared-memory size overflow".to_string())?;
    if maximum_bytes > protocol::WORKER_MAX_MEMORY_BYTES {
        let maximum_guest_bytes = protocol::WORKER_MAX_MEMORY_BYTES
            .checked_sub(worker_minimum)
            .and_then(|bytes| bytes.checked_sub(worker_heap))
            .expect("signed worker maximum covers its declared regions");
        return Err(format!(
            "Linux guest RAM request is {ram_bytes} bytes, but the signed backend currently supports at most {maximum_guest_bytes} guest bytes ({maximum_bytes} required shared bytes, {} supported); lower this principal's `linux_memory_bytes` or install a wider signed worker",
            protocol::WORKER_MAX_MEMORY_BYTES
        ));
    }
    let initial_pages = protocol::WORKER_MIN_MEMORY_BYTES / protocol::WASM_PAGE_BYTES;
    let control_offset = protocol::control_offset(0)
        .ok_or_else(|| "Linux vCPU has no worker-zero control descriptor".to_string())?;
    Ok((
        u32::try_from(initial_pages)
            .map_err(|_| "Linux vCPU initial page count is too large".to_string())?,
        u32::try_from(maximum_pages)
            .map_err(|_| "Linux vCPU maximum page count is too large".to_string())?,
        control_offset,
    ))
}

#[cfg(any(target_arch = "wasm32", test))]
fn auto_guest_ram_bytes(maximum_memory_pages: u32) -> Result<u64, String> {
    const GUEST_PAGE_BYTES: u64 = 4096;
    let wasm_page = u64::try_from(protocol::WASM_PAGE_BYTES).expect("Wasm page size fits u64");
    let worker_minimum =
        u64::try_from(protocol::WORKER_MIN_MEMORY_BYTES).expect("worker minimum fits u64");
    let worker_heap =
        u64::try_from(protocol::WORKER_HEAP_OVERHEAD_BYTES).expect("worker heap overhead fits u64");
    let admitted = u64::from(maximum_memory_pages)
        .checked_mul(wasm_page)
        .ok_or_else(|| "admitted Linux vCPU memory exceeds this platform".to_string())?;
    let available = admitted
        .checked_sub(worker_minimum)
        .and_then(|bytes| bytes.checked_sub(worker_heap))
        .ok_or_else(|| "admitted Linux vCPU memory leaves no guest RAM".to_string())?;
    // This group maximum is already the intersection of the signed worker's
    // memory64 capability, remaining host capacity, operator policy, and the
    // invoking principal's budget. Guest RAM consumes that admitted envelope
    // minus only the worker's signed base and heap requirements. The controller
    // lives in its own governed Store and is not charged to this shared region.
    let aligned = available - (available % GUEST_PAGE_BYTES);
    if aligned < MIN_BOOT_RAM_BYTES {
        return Err(format!(
            "admitted Linux vCPU memory leaves {aligned} guest bytes; at least {MIN_BOOT_RAM_BYTES} are required"
        ));
    }
    Ok(aligned)
}

#[cfg(target_arch = "wasm32")]
fn field_len(header: &[u8], offset: usize) -> Result<usize, String> {
    required_u32(header, offset, "length").and_then(|value| {
        usize::try_from(value).map_err(|_| "length is not addressable".to_string())
    })
}

#[cfg(target_arch = "wasm32")]
fn required_u32(header: &[u8], offset: usize, name: &str) -> Result<u32, String> {
    protocol::read_u32(header, offset)
        .ok_or_else(|| format!("Linux vCPU response is missing {name}"))
}

#[cfg(target_arch = "wasm32")]
fn required_u64(header: &[u8], offset: usize, name: &str) -> Result<u64, String> {
    protocol::read_u64(header, offset)
        .ok_or_else(|| format!("Linux vCPU response is missing {name}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reference_backend_identity_is_explicit_before_boot() {
        let machine = LinuxMachine::new_reference(MachineConfig {
            ram_bytes: 16 * 1024 * 1024,
            max_console_bytes: 1024,
        })
        .expect("reference backend admission");

        assert_eq!(machine.backend_id(), DEFAULT_LINUX_BACKEND_ID);
        assert_eq!(machine.hart_count(), 1);
    }

    #[test]
    fn shared_memory_keeps_machine_and_control_region_disjoint() {
        let (initial_pages, maximum_pages, control_offset) =
            shared_memory_layout(512 * 1024 * 1024).expect("default layout");
        assert_eq!(initial_pages, 1024);
        assert_eq!(maximum_pages, 10_240);
        assert_eq!(control_offset, (56 * 1024 * 1024) as u64);

        let (initial_pages, maximum_pages, control_offset) =
            shared_memory_layout(8 * 1024 * 1024 * 1024).expect("memory64 Realm layout");
        assert_eq!(initial_pages, 1024);
        assert_eq!(maximum_pages, 133_120);
        assert_eq!(control_offset, (56 * 1024 * 1024) as u64);
    }

    #[test]
    fn auto_memory_uses_the_admitted_group_without_a_hidden_policy_cap() {
        let (initial_pages, maximum_pages, _) =
            shared_memory_layout(0).expect("auto layout request");
        assert_eq!(initial_pages, 1024);
        assert_eq!(maximum_pages, 0);

        assert_eq!(
            auto_guest_ram_bytes(16_384).expect("one GiB group"),
            896 * 1024 * 1024
        );
        assert_eq!(
            auto_guest_ram_bytes(262_144).expect("signed memory64 worker maximum"),
            15 * 1024 * 1024 * 1024 + 896 * 1024 * 1024
        );
        assert!(auto_guest_ram_bytes(2048).is_err());
    }

    #[test]
    fn explicit_memory_beyond_the_signed_worker_fails_with_capability_provenance() {
        let error = shared_memory_layout(168 * 1024 * 1024 * 1024)
            .expect_err("current signed worker cannot address 168 GiB");
        assert!(error.contains("signed backend currently supports at most"));
        assert!(error.contains("install a wider signed worker"));
    }

    #[test]
    fn explicit_and_native_auto_hart_topologies_are_bounded() {
        assert_eq!(explicit_hart_count(0).expect("auto"), None);
        assert_eq!(explicit_hart_count(8).expect("eight harts"), Some(8));
        assert!(explicit_hart_count((MAX_HARTS + 1) as u32).is_err());
        assert!(
            (1..=MAX_HARTS as u32)
                .contains(&reference_hart_count(0).expect("native auto topology"))
        );
        assert_eq!(auto_guest_hart_count(0), AUTO_PREWARM_HARTS);
        assert_eq!(auto_guest_hart_count(1), AUTO_PREWARM_HARTS);
        assert_eq!(auto_guest_hart_count(8), AUTO_PREWARM_HARTS);
        assert_eq!(auto_guest_hart_count(128), AUTO_PREWARM_HARTS);
    }

    #[test]
    fn automatic_topology_matches_the_signed_prewarm_envelope() {
        let prewarm = prewarm_spec().expect("signed prewarm metadata");
        assert_eq!(AUTO_PREWARM_HARTS, 1);
        assert_eq!(prewarm.hart_count, AUTO_PREWARM_HARTS);
        assert!(prewarm.binding.iter().any(|byte| *byte != 0));
        assert!(prewarm.matches(
            LinuxMachineConfig {
                ram_bytes: 1024 * 1024 * 1024,
                max_console_bytes: 64 * 1024,
            },
            auto_guest_hart_count(24),
        ));
        assert!(!prewarm.matches(
            LinuxMachineConfig {
                ram_bytes: 1024 * 1024 * 1024,
                max_console_bytes: 64 * 1024,
            },
            2,
        ));
    }

    #[test]
    fn multi_hart_machine_uses_the_explicit_topology() {
        let machine = LinuxMachine::new(
            LinuxMachineConfig {
                ram_bytes: 32 * 1024 * 1024,
                max_console_bytes: 64 * 1024,
            },
            2,
        )
        .expect("two-hart cold machine");

        assert_eq!(machine.hart_count(), 2);
        assert_eq!(machine.ram_bytes(), 32 * 1024 * 1024);
    }
}
