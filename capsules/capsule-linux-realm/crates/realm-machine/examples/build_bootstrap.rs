use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

const CPIO_MAGIC: &str = "070701";
const CPIO_HEADER_BYTES: usize = 110;
const RISCV_ELF_MACHINE: u16 = 243;
const MAX_INIT_BYTES: usize = 1024 * 1024;

const DIRECTORIES: [(&str, u32); 10] = [
    (".", 0o755),
    ("./dev", 0o755),
    ("./home", 0o755),
    ("./home/agent", 0o700),
    ("./proc", 0o755),
    ("./run", 0o755),
    ("./sys", 0o755),
    ("./system", 0o755),
    ("./tmp", 0o1777),
    ("./workspace", 0o700),
];

fn main() -> ExitCode {
    match run(std::env::args_os().skip(1)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            ExitCode::FAILURE
        }
    }
}

fn run(mut args: impl Iterator<Item = OsString>) -> Result<(), String> {
    let init_path = args.next().ok_or_else(usage)?;
    let output_path = args.next().ok_or_else(usage)?;
    if args.next().is_some() {
        return Err(usage());
    }

    let init = fs::read(&init_path)
        .map_err(|error| format!("could not read bootstrap init {init_path:?}: {error}"))?;
    validate_riscv_init(&init)?;
    let archive = build_archive(&init)?;
    atomic_write(Path::new(&output_path), &archive)?;
    println!(
        "wrote {} bytes to {:?}",
        archive.len(),
        Path::new(&output_path)
    );
    Ok(())
}

fn usage() -> String {
    "usage: build_bootstrap INIT_RISCV64 OUTPUT_CPIO".to_string()
}

fn validate_riscv_init(init: &[u8]) -> Result<(), String> {
    if init.len() < 20 || &init[..4] != b"\x7fELF" {
        return Err("bootstrap init must be an ELF executable".to_string());
    }
    if init.len() > MAX_INIT_BYTES {
        return Err(format!(
            "bootstrap init exceeds the {MAX_INIT_BYTES}-byte build ceiling"
        ));
    }
    if init[5] != 1 {
        return Err("bootstrap init must be little-endian".to_string());
    }
    if u16::from_le_bytes([init[18], init[19]]) != RISCV_ELF_MACHINE {
        return Err("bootstrap init must target RISC-V".to_string());
    }
    Ok(())
}

fn build_archive(init: &[u8]) -> Result<Vec<u8>, String> {
    let mut archive = Vec::with_capacity(init.len() + 4096);
    let mut inode = 1_u32;
    for (name, permissions) in DIRECTORIES {
        append_entry(&mut archive, inode, name, 0o040000 | permissions, 2, &[])?;
        inode += 1;
    }
    append_entry(&mut archive, inode, "./init", 0o100755, 1, init)?;
    append_entry(&mut archive, 0, "TRAILER!!!", 0, 1, &[])?;
    archive.resize(archive.len().next_multiple_of(512), 0);
    Ok(archive)
}

fn append_entry(
    archive: &mut Vec<u8>,
    inode: u32,
    name: &str,
    mode: u32,
    links: u32,
    contents: &[u8],
) -> Result<(), String> {
    let name_bytes = name.as_bytes();
    let name_size =
        u32::try_from(name_bytes.len() + 1).map_err(|_| "cpio name is too long".to_string())?;
    let file_size =
        u32::try_from(contents.len()).map_err(|_| "cpio entry is too large".to_string())?;
    let header = format!(
        "{CPIO_MAGIC}{inode:08x}{mode:08x}{:08x}{:08x}{links:08x}{:08x}{file_size:08x}{:08x}{:08x}{:08x}{:08x}{name_size:08x}{:08x}",
        0, 0, 0, 0, 0, 0, 0, 0
    );
    if header.len() != CPIO_HEADER_BYTES {
        return Err("internal newc header length mismatch".to_string());
    }

    archive.extend_from_slice(header.as_bytes());
    archive.extend_from_slice(name_bytes);
    archive.push(0);
    pad_four(archive);
    archive.extend_from_slice(contents);
    pad_four(archive);
    Ok(())
}

fn pad_four(bytes: &mut Vec<u8>) {
    bytes.resize(bytes.len().next_multiple_of(4), 0);
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    fs::create_dir_all(parent)
        .map_err(|error| format!("could not create output directory {parent:?}: {error}"))?;
    let temp = temporary_path(path);
    fs::write(&temp, bytes)
        .map_err(|error| format!("could not write temporary archive {temp:?}: {error}"))?;
    fs::rename(&temp, path).map_err(|error| {
        let _ = fs::remove_file(&temp);
        format!("could not install bootstrap archive {path:?}: {error}")
    })
}

fn temporary_path(path: &Path) -> PathBuf {
    let mut name = path
        .file_name()
        .map(OsString::from)
        .unwrap_or_else(|| OsString::from("bootstrap.cpio"));
    name.push(format!(".tmp.{}", std::process::id()));
    path.with_file_name(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fake_riscv_elf() -> Vec<u8> {
        let mut elf = vec![0_u8; 128];
        elf[..4].copy_from_slice(b"\x7fELF");
        elf[4] = 2;
        elf[5] = 1;
        elf[18..20].copy_from_slice(&RISCV_ELF_MACHINE.to_le_bytes());
        elf
    }

    #[test]
    fn archive_is_deterministic_aligned_newc() {
        let first = build_archive(&fake_riscv_elf()).expect("archive");
        let second = build_archive(&fake_riscv_elf()).expect("archive");
        assert_eq!(first, second);
        assert!(first.starts_with(CPIO_MAGIC.as_bytes()));
        assert!(first.windows(11).any(|window| window == b"TRAILER!!!\0"));
        assert!(first.len().is_multiple_of(512));
    }

    #[test]
    fn rejects_non_riscv_and_oversized_inputs() {
        let mut wrong_machine = fake_riscv_elf();
        wrong_machine[18..20].copy_from_slice(&62_u16.to_le_bytes());
        assert!(validate_riscv_init(&wrong_machine).is_err());

        let mut oversized = fake_riscv_elf();
        oversized.resize(MAX_INIT_BYTES + 1, 0);
        assert!(validate_riscv_init(&oversized).is_err());
    }
}
