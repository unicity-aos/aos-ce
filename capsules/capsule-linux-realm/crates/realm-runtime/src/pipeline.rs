//! Resumable two-process execution over the realm-core scheduler and one pipe.

use super::*;
use wasmi::{TypedResumableCall, TypedResumableCallHostTrap};

/// One completed process from a pipeline execution.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipelineProcessReport {
    /// Realm-local process identity for this machine run.
    pub process_id: ProcessId,
    /// Terminal result and resource accounting.
    pub execution: ExecutionReport,
}

/// Completed producer and consumer reports from a two-stage pipeline.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PipelineReport {
    /// Process whose standard output was connected to the pipe writer.
    pub producer: PipelineProcessReport,
    /// Process whose standard input was connected to the pipe reader.
    pub consumer: PipelineProcessReport,
}

/// Failure to construct or drive the bounded pipeline machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PipelineError {
    /// A process module failed ordinary runtime admission.
    Launch(LaunchError),
    /// The process semantic kernel rejected setup or a transition.
    Kernel(KernelError),
    /// Wasmi could not resume a previously parked host call.
    Resume(String),
    /// No process was runnable while at least one process remained incomplete.
    Deadlock,
    /// A process completed without producing its accounting report.
    MissingReport(ProcessId),
}

impl fmt::Display for PipelineError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Launch(error) => error.fmt(formatter),
            Self::Kernel(error) => error.fmt(formatter),
            Self::Resume(message) => write!(formatter, "pipeline resume failed: {message}"),
            Self::Deadlock => formatter.write_str("realm pipeline deadlocked"),
            Self::MissingReport(process) => write!(
                formatter,
                "realm process {} completed without a report",
                process.get()
            ),
        }
    }
}

impl std::error::Error for PipelineError {}

impl From<LaunchError> for PipelineError {
    fn from(error: LaunchError) -> Self {
        Self::Launch(error)
    }
}

impl From<KernelError> for PipelineError {
    fn from(error: KernelError) -> Self {
        Self::Kernel(error)
    }
}

enum InvocationState {
    Start(TypedFunc<(), ()>),
    Suspended(TypedResumableCallHostTrap<()>),
    Complete,
}

struct ProcessSlot {
    store: Store<HostState>,
    invocation: InvocationState,
    limits: RunLimits,
    report: Option<ExecutionReport>,
}

enum PendingCompletion {
    Resume(i32),
    Reparked,
    BrokenPipe,
}

impl RealmRuntime {
    /// Execute two nested modules connected by one bounded stdout-to-stdin pipe.
    ///
    /// The consumer is admitted first so its initial empty read proves real
    /// suspension and resumption. Both process stores have independent memories.
    pub fn execute_pipeline(
        &self,
        producer_wasm: &[u8],
        producer: ProcessConfig,
        consumer_wasm: &[u8],
        consumer: ProcessConfig,
        limits: RunLimits,
        pipe_capacity: usize,
    ) -> Result<PipelineReport, PipelineError> {
        let producer_limits = RunLimits {
            fuel: limits.fuel / 2,
            memory_bytes: limits.memory_bytes,
            output_bytes: limits.output_bytes / 2,
        };
        let consumer_limits = RunLimits {
            fuel: limits.fuel.saturating_sub(producer_limits.fuel),
            memory_bytes: limits.memory_bytes,
            output_bytes: limits
                .output_bytes
                .saturating_sub(producer_limits.output_bytes),
        };
        let mut kernel = RealmKernel::new(RealmLimits {
            max_processes: 3,
            max_pipes: 1,
            max_pipe_bytes: MAX_IO_BYTES,
            max_total_pipe_bytes: MAX_IO_BYTES,
            max_descriptors_per_process: 4,
        });
        let supervisor =
            kernel.spawn_root(ProcessSpec::new(ExecutableId::REALM_SUPERVISOR, "/"))?;
        kernel.admit(supervisor)?;
        if kernel.dispatch_next() != Some(supervisor) {
            return Err(PipelineError::Deadlock);
        }
        let ends = kernel.create_pipe(supervisor, pipe_capacity)?;
        let consumer_id = kernel.spawn_child(
            supervisor,
            process_spec(consumer_wasm, &consumer),
            &[DescriptorBinding {
                source: ends.read,
                target: Descriptor::STDIN,
            }],
        )?;
        let producer_id = kernel.spawn_child(
            supervisor,
            process_spec(producer_wasm, &producer),
            &[DescriptorBinding {
                source: ends.write,
                target: Descriptor::STDOUT,
            }],
        )?;
        kernel.close_descriptor(supervisor, ends.read)?;
        kernel.close_descriptor(supervisor, ends.write)?;
        kernel.admit(consumer_id)?;
        kernel.admit(producer_id)?;
        kernel.exit(supervisor, 0)?;
        if kernel.reap_root(supervisor)? != Termination::Exited(0) {
            return Err(PipelineError::Deadlock);
        }

        let kernel = Rc::new(RefCell::new(kernel));
        let mut slots = BTreeMap::new();
        slots.insert(
            consumer_id,
            self.pipeline_slot(
                consumer_wasm,
                consumer,
                consumer_limits,
                consumer_id,
                &kernel,
            )?,
        );
        slots.insert(
            producer_id,
            self.pipeline_slot(
                producer_wasm,
                producer,
                producer_limits,
                producer_id,
                &kernel,
            )?,
        );

        while slots.values().any(|slot| slot.report.is_none()) {
            let process = kernel
                .borrow_mut()
                .dispatch_next()
                .ok_or(PipelineError::Deadlock)?;
            let slot = slots
                .get_mut(&process)
                .ok_or(PipelineError::MissingReport(process))?;
            drive_slot(process, slot, &kernel)?;
        }

        Ok(PipelineReport {
            producer: PipelineProcessReport {
                process_id: producer_id,
                execution: take_report(&mut slots, producer_id)?,
            },
            consumer: PipelineProcessReport {
                process_id: consumer_id,
                execution: take_report(&mut slots, consumer_id)?,
            },
        })
    }

    fn pipeline_slot(
        &self,
        wasm: &[u8],
        process: ProcessConfig,
        limits: RunLimits,
        process_id: ProcessId,
        kernel: &Rc<RefCell<RealmKernel>>,
    ) -> Result<ProcessSlot, PipelineError> {
        let context = ProcessContext {
            process: process_id,
            kernel: Rc::clone(kernel),
        };
        let (store, start) = self.prepare_process(
            wasm,
            process,
            limits,
            Box::<DenyRealmHost>::default(),
            Some(context),
        )?;
        Ok(ProcessSlot {
            store,
            invocation: InvocationState::Start(start),
            limits,
            report: None,
        })
    }
}

fn process_spec(wasm: &[u8], process: &ProcessConfig) -> ProcessSpec {
    ProcessSpec::new(
        ExecutableId::new(*blake3::hash(wasm).as_bytes()),
        process.cwd.clone(),
    )
}

fn drive_slot(
    process: ProcessId,
    slot: &mut ProcessSlot,
    kernel: &Rc<RefCell<RealmKernel>>,
) -> Result<(), PipelineError> {
    let invocation = std::mem::replace(&mut slot.invocation, InvocationState::Complete);
    let call = match invocation {
        InvocationState::Start(start) => match start.call_resumable(&mut slot.store, ()) {
            Ok(call) => call,
            Err(error) => {
                let outcome = classify_process_error(&error);
                finish_process(process, slot, kernel, outcome)?;
                return Ok(());
            }
        },
        InvocationState::Suspended(suspended) => {
            let value = match complete_pending_io(process, slot, kernel)? {
                PendingCompletion::Resume(value) => value,
                PendingCompletion::Reparked => {
                    slot.invocation = InvocationState::Suspended(suspended);
                    return Ok(());
                }
                PendingCompletion::BrokenPipe => {
                    finish_process(
                        process,
                        slot,
                        kernel,
                        ProcessOutcome::HostFault(HostFault::BrokenPipe),
                    )?;
                    return Ok(());
                }
            };
            match suspended.resume(&mut slot.store, &[Val::I32(value)]) {
                Ok(call) => call,
                Err(error) => {
                    let outcome = classify_process_error(&error);
                    finish_process(process, slot, kernel, outcome)?;
                    return Ok(());
                }
            }
        }
        InvocationState::Complete => return Err(PipelineError::MissingReport(process)),
    };
    handle_resumable_call(process, slot, kernel, call)
}

fn handle_resumable_call(
    process: ProcessId,
    slot: &mut ProcessSlot,
    kernel: &Rc<RefCell<RealmKernel>>,
    call: TypedResumableCall<()>,
) -> Result<(), PipelineError> {
    match call {
        TypedResumableCall::Finished(()) => {
            finish_process(process, slot, kernel, ProcessOutcome::Exited(0))
        }
        TypedResumableCall::OutOfFuel(_) => {
            finish_process(process, slot, kernel, ProcessOutcome::FuelExhausted)
        }
        TypedResumableCall::HostTrap(suspended) => {
            if suspended
                .host_error()
                .downcast_ref::<ProcessSuspended>()
                .is_some()
            {
                if slot.store.data().pending_io.is_none() {
                    return Err(PipelineError::Resume(
                        "suspended process has no pending I/O".to_string(),
                    ));
                }
                slot.invocation = InvocationState::Suspended(suspended);
                Ok(())
            } else {
                let outcome = classify_process_error(suspended.host_error());
                finish_process(process, slot, kernel, outcome)
            }
        }
    }
}

fn complete_pending_io(
    process: ProcessId,
    slot: &mut ProcessSlot,
    kernel: &Rc<RefCell<RealmKernel>>,
) -> Result<PendingCompletion, PipelineError> {
    let pending = slot
        .store
        .data_mut()
        .pending_io
        .take()
        .ok_or_else(|| PipelineError::Resume("pending I/O disappeared".to_string()))?;
    match pending {
        PendingIo::Read {
            descriptor,
            pointer,
            capacity,
        } => match {
            kernel
                .borrow_mut()
                .read_pipe(process, descriptor, capacity)?
        } {
            PipeReadResult::Data(bytes) => {
                let memory = slot.store.data().memory.ok_or_else(|| {
                    PipelineError::Resume("pending read process has no memory".to_string())
                })?;
                memory
                    .write(&mut slot.store, pointer, &bytes)
                    .map_err(|error| PipelineError::Resume(error.to_string()))?;
                let read = i32::try_from(bytes.len())
                    .map_err(|_| PipelineError::Resume("read count overflow".to_string()))?;
                Ok(PendingCompletion::Resume(read))
            }
            PipeReadResult::Eof => Ok(PendingCompletion::Resume(0)),
            PipeReadResult::WouldBlock => {
                if kernel.borrow_mut().park_pipe_read(process, descriptor)? != ParkResult::Parked {
                    return Err(PipelineError::Resume(
                        "spurious pipe-read wake could not repark".to_string(),
                    ));
                }
                slot.store.data_mut().pending_io = Some(PendingIo::Read {
                    descriptor,
                    pointer,
                    capacity,
                });
                Ok(PendingCompletion::Reparked)
            }
        },
        PendingIo::Write { descriptor, bytes } => {
            match {
                kernel
                    .borrow_mut()
                    .write_pipe(process, descriptor, &bytes)?
            } {
                PipeWriteResult::Written(written) => {
                    let written = i32::try_from(written)
                        .map_err(|_| PipelineError::Resume("write count overflow".to_string()))?;
                    Ok(PendingCompletion::Resume(written))
                }
                PipeWriteResult::BrokenPipe => Ok(PendingCompletion::BrokenPipe),
                PipeWriteResult::WouldBlock => {
                    if kernel.borrow_mut().park_pipe_write(process, descriptor)?
                        != ParkResult::Parked
                    {
                        return Err(PipelineError::Resume(
                            "spurious pipe-write wake could not repark".to_string(),
                        ));
                    }
                    slot.store.data_mut().pending_io = Some(PendingIo::Write { descriptor, bytes });
                    Ok(PendingCompletion::Reparked)
                }
            }
        }
    }
}

fn finish_process(
    process: ProcessId,
    slot: &mut ProcessSlot,
    kernel: &Rc<RefCell<RealmKernel>>,
    outcome: ProcessOutcome,
) -> Result<(), PipelineError> {
    match &outcome {
        ProcessOutcome::Exited(status) => kernel.borrow_mut().exit(process, *status)?,
        ProcessOutcome::HostFault(HostFault::BrokenPipe) => {
            kernel.borrow_mut().signal(process, Signal::Pipe)?;
        }
        ProcessOutcome::FuelExhausted
        | ProcessOutcome::HostFault(_)
        | ProcessOutcome::Trapped(_) => {
            kernel.borrow_mut().signal(process, Signal::Kill)?;
        }
    }
    slot.report = Some(execution_report(&slot.store, slot.limits, outcome));
    slot.invocation = InvocationState::Complete;
    Ok(())
}

fn take_report(
    slots: &mut BTreeMap<ProcessId, ProcessSlot>,
    process: ProcessId,
) -> Result<ExecutionReport, PipelineError> {
    slots
        .remove(&process)
        .and_then(|slot| slot.report)
        .ok_or(PipelineError::MissingReport(process))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn two_isolated_guests_stream_through_a_resumable_bounded_pipe() {
        let report = RealmRuntime::default()
            .execute_pipeline(
                ECHO_GUEST,
                ProcessConfig {
                    argv: vec!["echo".to_string(), "hello pipeline".to_string()],
                    cwd: "/workspace".to_string(),
                },
                STDIN_CAT_GUEST,
                ProcessConfig {
                    argv: vec!["stdin-cat".to_string()],
                    cwd: "/workspace".to_string(),
                },
                RunLimits::default(),
                4,
            )
            .expect("pipeline completes");

        assert_ne!(report.producer.process_id, report.consumer.process_id);
        assert_eq!(report.producer.execution.outcome, ProcessOutcome::Exited(0));
        assert!(report.producer.execution.stdout.is_empty());
        assert_eq!(report.consumer.execution.outcome, ProcessOutcome::Exited(0));
        assert_eq!(report.consumer.execution.stdout, b"hello pipeline\n");
        assert!(report.producer.execution.fuel_consumed > 0);
        assert!(report.consumer.execution.fuel_consumed > 0);
        assert!(report.producer.execution.suspensions > 0);
        assert!(report.consumer.execution.suspensions > 0);
    }

    #[test]
    fn zero_capacity_fails_before_any_guest_runs() {
        let error = RealmRuntime::default()
            .execute_pipeline(
                ECHO_GUEST,
                ProcessConfig {
                    argv: vec!["echo".to_string(), "x".to_string()],
                    cwd: "/workspace".to_string(),
                },
                STDIN_CAT_GUEST,
                ProcessConfig {
                    argv: vec!["stdin-cat".to_string()],
                    cwd: "/workspace".to_string(),
                },
                RunLimits::default(),
                0,
            )
            .expect_err("zero-capacity pipe is rejected");

        assert_eq!(
            error,
            PipelineError::Kernel(KernelError::InvalidPipeCapacity)
        );
    }

    #[test]
    fn producer_gets_broken_pipe_when_consumer_exits_without_reading() {
        let immediate_exit =
            wat::parse_str(r#"(module (func (export "_start")))"#).expect("exit guest compiles");
        let report = RealmRuntime::default()
            .execute_pipeline(
                ECHO_GUEST,
                ProcessConfig {
                    argv: vec!["echo".to_string(), "unread".to_string()],
                    cwd: "/workspace".to_string(),
                },
                &immediate_exit,
                ProcessConfig {
                    argv: vec!["exit".to_string()],
                    cwd: "/workspace".to_string(),
                },
                RunLimits::default(),
                4,
            )
            .expect("pipeline reaches terminal states");

        assert_eq!(report.consumer.execution.outcome, ProcessOutcome::Exited(0));
        assert_eq!(
            report.producer.execution.outcome,
            ProcessOutcome::HostFault(HostFault::BrokenPipe)
        );
    }
}
