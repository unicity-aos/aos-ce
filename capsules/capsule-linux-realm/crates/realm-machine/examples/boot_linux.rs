use aos_realm_machine::{Csr, Machine, MachineConfig, SliceOutcome};
use std::{
    env, fs,
    io::Write,
    process::ExitCode,
    time::{Duration, Instant},
};

const DEFAULT_RAM_MIB: usize = 32;
const MIB: usize = 1024 * 1024;
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
    let image_path = args.next().ok_or_else(usage)?;
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
    let hart_count = args
        .next()
        .map(|value| {
            value
                .to_str()
                .ok_or_else(|| "HARTS must be UTF-8".to_string())?
                .parse::<usize>()
                .map_err(|error| format!("invalid HARTS: {error}"))
        })
        .transpose()?
        .unwrap_or(1);
    let ram_mib = args
        .next()
        .map(|value| {
            value
                .to_str()
                .ok_or_else(|| "RAM_MIB must be UTF-8".to_string())?
                .parse::<usize>()
                .map_err(|error| format!("invalid RAM_MIB: {error}"))
        })
        .transpose()?
        .unwrap_or(DEFAULT_RAM_MIB);
    let ram_bytes = ram_bytes_from_mib(ram_mib)?;
    if args.next().is_some() {
        return Err(usage());
    }

    let image =
        fs::read(&image_path).map_err(|error| format!("could not read {image_path:?}: {error}"))?;
    let mut machine = Machine::new_with_harts(
        MachineConfig {
            ram_bytes,
            max_console_bytes: CONSOLE_BYTES,
        },
        hart_count,
    )
    .map_err(|error| format!("could not admit Linux machine: {error}"))?;
    let wall_time_seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| "host wall clock is before the Unix epoch".to_string())?
        .as_secs();
    let bootargs =
        format!("earlycon=sbi console=hvc0 init=/init panic=-1 aos.wall_time={wall_time_seconds}");
    machine
        .boot_linux(&image, &[], &bootargs)
        .map_err(|error| format!("could not admit Linux image: {error}"))?;

    let started = Instant::now();
    let mut serial = Vec::new();
    let mut total_steps = 0;
    while total_steps < max_steps {
        let remaining = max_steps.saturating_sub(total_steps);
        let slice_start_pc = machine.pc();
        let report = machine.run_slice(remaining.min(SLICE_STEPS));
        total_steps = report.total_steps_executed;
        std::io::stdout()
            .write_all(&report.console)
            .map_err(|error| format!("could not write serial output: {error}"))?;
        serial.extend_from_slice(&report.console);
        if machine.pc() == 0 && slice_start_pc != 0 {
            return Err(format!(
                "Linux fell through its unset trap vector during the slice starting at {slice_start_pc:#x}: mcause={:#x}, mepc={:#x}, mtval={:#x}",
                machine.csr(Csr::Mcause),
                machine.csr(Csr::Mepc),
                machine.csr(Csr::Mtval),
            ));
        }
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
                if !serial
                    .windows(INIT_MARKER.len())
                    .any(|bytes| bytes == INIT_MARKER)
                {
                    return Err(format!(
                        "Linux requested host service {} before /init after {} steps",
                        request.id.get(),
                        report.total_steps_executed
                    ));
                }
                if hart_count > 1 && !linux_reported_cpu_count(&serial, hart_count) {
                    return Err(format!(
                        "Linux reached /init without reporting {hart_count} active CPUs"
                    ));
                }
                eprintln!(
                    "AOS Linux boot reached its first governed host request with {hart_count} hart(s) and {ram_mib} MiB RAM: {} retired instructions; {}",
                    report.total_instructions_retired,
                    performance_summary(&machine, report.total_steps_executed, started.elapsed())
                );
                return Ok(());
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

fn usage() -> String {
    "usage: boot_linux IMAGE [MAX_STEPS] [HARTS] [RAM_MIB]".to_string()
}

fn ram_bytes_from_mib(ram_mib: usize) -> Result<usize, String> {
    if ram_mib == 0 {
        return Err("RAM_MIB must be greater than zero".to_string());
    }
    ram_mib
        .checked_mul(MIB)
        .ok_or_else(|| format!("RAM_MIB {ram_mib} overflows the host address space"))
}

fn linux_reported_cpu_count(serial: &[u8], hart_count: usize) -> bool {
    let markers = [
        format!("{hart_count} CPUs"),
        format!("{hart_count} processors activated"),
    ];
    markers.iter().any(|marker| {
        serial
            .windows(marker.len())
            .any(|window| window == marker.as_bytes())
    })
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ram_mebibytes_are_checked_before_machine_admission() {
        assert_eq!(ram_bytes_from_mib(32), Ok(32 * MIB));
        assert_eq!(
            ram_bytes_from_mib(0),
            Err("RAM_MIB must be greater than zero".to_string())
        );

        let overflowing = usize::MAX / MIB + 1;
        assert_eq!(
            ram_bytes_from_mib(overflowing),
            Err(format!(
                "RAM_MIB {overflowing} overflows the host address space"
            ))
        );
    }

    #[test]
    fn smp_boot_markers_accept_both_linux_formats() {
        assert!(linux_reported_cpu_count(b"SMP: 8 CPUs online", 8));
        assert!(linux_reported_cpu_count(
            b"Brought up 4 nodes, 4 processors activated",
            4
        ));
        assert!(!linux_reported_cpu_count(b"SMP: 2 CPUs online", 8));
    }
}
