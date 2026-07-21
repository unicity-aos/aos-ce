use aos_realm_machine::{Machine, MachineConfig, SliceOutcome};
use std::{
    env, fs,
    io::Write,
    process::ExitCode,
    time::{Duration, Instant},
};

const RAM_BYTES: usize = 32 * 1024 * 1024;
const CONSOLE_BYTES: usize = 4 * 1024 * 1024;
const SLICE_STEPS: u64 = 100_000;
const DEFAULT_MAX_STEPS: u64 = 250_000_000;
const INIT_MARKER: &[u8] = b"AOS LINUX /init";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let mut args = env::args_os().skip(1);
    let image_path = args
        .next()
        .ok_or_else(|| "usage: boot_linux IMAGE [MAX_STEPS]".to_string())?;
    let max_steps = args
        .next()
        .map(|value| {
            value
                .to_str()
                .ok_or_else(|| "MAX_STEPS must be UTF-8".to_string())?
                .parse::<u64>()
                .map_err(|error| format!("invalid MAX_STEPS: {error}"))
        })
        .transpose()?
        .unwrap_or(DEFAULT_MAX_STEPS);
    if args.next().is_some() {
        return Err("usage: boot_linux IMAGE [MAX_STEPS]".to_string());
    }

    let image =
        fs::read(&image_path).map_err(|error| format!("could not read {image_path:?}: {error}"))?;
    let mut machine = Machine::new(MachineConfig {
        ram_bytes: RAM_BYTES,
        max_console_bytes: CONSOLE_BYTES,
    })
    .map_err(|error| format!("could not admit Linux machine: {error}"))?;
    machine
        .boot_linux(&image, &[], "earlycon=sbi console=hvc0 init=/init panic=-1")
        .map_err(|error| format!("could not admit Linux image: {error}"))?;

    let started = Instant::now();
    let mut serial = Vec::new();
    let mut total_steps = 0;
    while total_steps < max_steps {
        let remaining = max_steps.saturating_sub(total_steps);
        let report = machine.run_slice(remaining.min(SLICE_STEPS));
        total_steps = report.total_steps_executed;
        std::io::stdout()
            .write_all(&report.console)
            .map_err(|error| format!("could not write serial output: {error}"))?;
        serial.extend_from_slice(&report.console);
        match report.outcome {
            SliceOutcome::Yielded => {}
            SliceOutcome::Halted(status) => {
                if !serial
                    .windows(INIT_MARKER.len())
                    .any(|bytes| bytes == INIT_MARKER)
                {
                    return Err(format!(
                        "Linux halted before /init marker after {} steps (status {status:?})",
                        report.total_steps_executed
                    ));
                }
                if !status.passed {
                    return Err(format!(
                        "Linux /init halted with failure status {status:?} after {} steps",
                        report.total_steps_executed
                    ));
                }
                eprintln!(
                    "AOS Linux boot passed: {} retired instructions; {}",
                    report.total_instructions_retired,
                    performance_summary(&machine, report.total_steps_executed, started.elapsed())
                );
                return Ok(());
            }
            SliceOutcome::HostRequest(request) => {
                return Err(format!(
                    "Linux requested unwired 9P host service {} at pc {:#x}; {}",
                    request.id.get(),
                    machine.pc(),
                    performance_summary(&machine, report.total_steps_executed, started.elapsed())
                ));
            }
            SliceOutcome::Trapped(trap) => {
                return Err(format!(
                    "Linux crossed the machine boundary at pc {:#x}: {trap}; {}",
                    machine.pc(),
                    performance_summary(&machine, report.total_steps_executed, started.elapsed())
                ));
            }
        }
    }

    Err(format!(
        "Linux did not halt within {max_steps} admitted steps; pc={:#x}, privilege={:?}; {}",
        machine.pc(),
        machine.privilege(),
        performance_summary(&machine, total_steps, started.elapsed())
    ))
}

fn performance_summary(machine: &Machine, steps: u64, elapsed: Duration) -> String {
    let metrics = machine.metrics();
    let steps_per_second = steps as f64 / elapsed.as_secs_f64().max(f64::MIN_POSITIVE);
    format!(
        "{steps} steps in {:.3}s ({steps_per_second:.0} steps/s), translations={} (instruction={}, load={}, store={}), translation cache hits={}, misses={}, flushes={}, Sv39 walks={}, PTE reads={}, PTE writes={}",
        elapsed.as_secs_f64(),
        metrics.translations(),
        metrics.instruction_translations,
        metrics.load_translations,
        metrics.store_translations,
        metrics.translation_cache_hits,
        metrics.translation_cache_misses,
        metrics.translation_cache_flushes,
        metrics.sv39_walks,
        metrics.page_table_entries_read,
        metrics.page_table_entries_written
    )
}
