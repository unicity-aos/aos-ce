//! Read-only hashing for product installation and capsule identity verification.

use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::Path;
use std::process::ExitCode;

fn digest(mut reader: impl Read) -> io::Result<String> {
    let mut hasher = blake3::Hasher::new();
    let mut buffer = [0_u8; 64 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            return Ok(hasher.finalize().to_hex().to_string());
        }
        hasher.update(&buffer[..count]);
    }
}

fn open_regular(path: &Path) -> io::Result<File> {
    let mut options = OpenOptions::new();
    options.read(true);
    // A raced-in FIFO must not block an installer waiting for another writer.
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(rustix::fs::OFlags::NONBLOCK.bits() as i32);
    }
    let file = options.open(path)?;
    if !file.metadata()?.is_file() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "checksum requires a regular file",
        ));
    }
    Ok(file)
}

pub(crate) fn run(path: &Path) -> ExitCode {
    match open_regular(path).and_then(digest) {
        Ok(hash) => {
            println!("{hash}");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("aos: checksum: {}: {error}", path.display());
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn standard_vectors_and_multiple_chunks() {
        assert_eq!(
            digest(&b""[..]).unwrap(),
            "af1349b9f5f9a1a6a0404dea36dcc9499bcb25c9adc112b7cc9a93cae41f3262"
        );
        assert_eq!(
            digest(&b"abc"[..]).unwrap(),
            "6437b3ac38465133ffb63b75273a8db548c558465d79db03fd359c6cd5bd9d85"
        );
        let bytes = vec![42; 200_001];
        assert_eq!(
            digest(bytes.as_slice()).unwrap(),
            blake3::hash(&bytes).to_hex().to_string()
        );
    }

    #[test]
    fn read_failure_is_not_a_digest() {
        struct Failing;
        impl Read for Failing {
            fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
                Err(io::Error::other("injected read failure"))
            }
        }
        assert!(digest(Failing).is_err());
    }

    #[test]
    fn directory_is_refused() {
        assert!(open_regular(Path::new(".")).is_err());
    }
}
