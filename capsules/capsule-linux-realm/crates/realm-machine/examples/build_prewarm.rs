use aos_realm_machine::{
    CheckpointBinding, CheckpointDigest, MAX_HARTS, Machine, MachineConfig, SliceOutcome,
};
use std::{env, fs, process::ExitCode, time::Instant};

const RAM_BYTES: usize = 1024 * 1024 * 1024;
const DEFAULT_HART_COUNT: usize = 1;
const CONSOLE_BYTES: usize = 64 * 1024;
const SLICE_STEPS: u64 = 10_000_000;
const MAX_STEPS: u64 = 2_000_000_000;
const PREWARM_WALL_SECONDS: u64 = 1_784_143_900;
const HOME_9P_CHANNEL: u32 = 1;
const SYSTEM_BLOCK_CHANNEL: u32 = 3;

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
    let system_path = args.next().ok_or_else(usage)?;
    let output_path = args.next().ok_or_else(usage)?;
    let hart_count = args
        .next()
        .map(|value| parse_hart_count(&value))
        .transpose()?
        .unwrap_or(DEFAULT_HART_COUNT);
    if args.next().is_some() {
        return Err(usage());
    }

    let image = fs::read(&image_path)
        .map_err(|error| format!("could not read Linux image {image_path:?}: {error}"))?;
    let system = fs::read(&system_path)
        .map_err(|error| format!("could not read system image {system_path:?}: {error}"))?;
    if system.len() < 4096 || !system.len().is_multiple_of(512) {
        return Err("system image must be a sector-aligned SquashFS artifact".to_string());
    }
    let binding = CheckpointBinding::new(
        CheckpointDigest::hash(&image),
        CheckpointDigest::hash(&system),
    );
    let mut machine = Machine::new_with_harts(
        MachineConfig {
            ram_bytes: RAM_BYTES,
            max_console_bytes: CONSOLE_BYTES,
        },
        hart_count,
    )
    .map_err(|error| format!("could not admit prewarm machine: {error}"))?;
    let bootargs = format!(
        "earlycon=sbi console=hvc0 init=/init panic=-1 aos.wall_time={PREWARM_WALL_SECONDS} aos.system_bytes={}",
        system.len()
    );
    machine
        .boot_linux(&image, &[], &bootargs)
        .map_err(|error| format!("could not admit Linux image: {error}"))?;

    let started = Instant::now();
    let mut total_steps = 0_u64;
    let mut console = Vec::new();
    let pending = loop {
        if total_steps == MAX_STEPS {
            return Err(format!(
                "Linux did not reach the prewarm suspension within {MAX_STEPS} steps"
            ));
        }
        let report = machine.run_slice((MAX_STEPS - total_steps).min(SLICE_STEPS));
        total_steps = report.total_steps_executed;
        console.extend_from_slice(&report.console);
        match report.outcome {
            SliceOutcome::Yielded => {}
            SliceOutcome::HostRequest(request) if request.channel == SYSTEM_BLOCK_CHANNEL => {
                let offset = request
                    .message
                    .as_slice()
                    .try_into()
                    .map(u64::from_le_bytes)
                    .map_err(|_| "system block request has an invalid offset".to_string())?;
                let offset = usize::try_from(offset)
                    .map_err(|_| "system block offset is not addressable".to_string())?;
                let end = offset
                    .checked_add(request.max_response_bytes)
                    .filter(|end| *end <= system.len())
                    .ok_or_else(|| "system block request exceeds the image".to_string())?;
                machine
                    .complete_9p_request(request.id, &system[offset..end])
                    .map_err(|error| format!("could not complete system block read: {error}"))?;
            }
            SliceOutcome::HostRequest(request) => break request,
            outcome => {
                return Err(format!(
                    "Linux reached {outcome:?} before the prewarm host suspension:\n{}",
                    String::from_utf8_lossy(&console)
                ));
            }
        }
    };
    if pending.channel != HOME_9P_CHANNEL {
        return Err(format!(
            "first Linux host request used channel {}, expected home channel {HOME_9P_CHANNEL}",
            pending.channel
        ));
    }
    if !console
        .windows(15)
        .any(|window| window == b"AOS LINUX /init")
    {
        return Err("Linux reached host suspension without the audited init marker".to_string());
    }

    let checkpoint = machine
        .checkpoint_host_suspension()
        .map_err(|error| format!("Linux suspension is not checkpoint-safe: {error}"))?;
    let encoded = checkpoint.encode(binding);
    fs::write(&output_path, &encoded)
        .map_err(|error| format!("could not write checkpoint {output_path:?}: {error}"))?;
    eprintln!(
        "AOS prewarm checkpoint: {total_steps} steps in {:.3}s, {} bytes, harts={hart_count}, image={}, distribution={}",
        started.elapsed().as_secs_f64(),
        encoded.len(),
        hex(binding.linux_image().as_bytes()),
        hex(binding.distribution().as_bytes())
    );
    Ok(())
}

fn usage() -> String {
    "usage: build_prewarm IMAGE SYSTEM_SQUASHFS OUTPUT [HART_COUNT]".to_string()
}

fn parse_hart_count(value: &std::ffi::OsStr) -> Result<usize, String> {
    let count = value
        .to_str()
        .ok_or_else(|| "HART_COUNT must be UTF-8".to_string())?
        .parse::<usize>()
        .map_err(|error| format!("invalid HART_COUNT: {error}"))?;
    if !(1..=MAX_HARTS).contains(&count) {
        return Err(format!(
            "HART_COUNT must be between 1 and {MAX_HARTS}, got {count}"
        ));
    }
    Ok(count)
}

fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut text = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        text.push(DIGITS[(byte >> 4) as usize] as char);
        text.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prewarm_topology_is_bounded_before_machine_allocation() {
        assert_eq!(DEFAULT_HART_COUNT, 1);
        assert_eq!(parse_hart_count(std::ffi::OsStr::new("1")), Ok(1));
        assert_eq!(
            parse_hart_count(std::ffi::OsStr::new("0")),
            Err(format!(
                "HART_COUNT must be between 1 and {MAX_HARTS}, got 0"
            ))
        );
        let excessive = (MAX_HARTS + 1).to_string();
        assert_eq!(
            parse_hart_count(std::ffi::OsStr::new(&excessive)),
            Err(format!(
                "HART_COUNT must be between 1 and {MAX_HARTS}, got {excessive}"
            ))
        );
    }
}
