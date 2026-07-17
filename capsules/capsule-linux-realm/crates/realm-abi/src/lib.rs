#![no_std]
#![deny(unsafe_code)]

//! Private, versioned guest ABI constants for AOS Realm.
//!
//! This is not a public Astrid WIT contract. It is the narrow boundary between
//! a process module and the realm runtime contained by the same capsule.

/// Import module used by the first realm guest ABI.
pub const IMPORT_MODULE_V0: &str = "aos_realm_v0";

/// Guest file descriptor for standard output.
pub const STDOUT_FD: i32 = 1;

/// Guest file descriptor for standard error.
pub const STDERR_FD: i32 = 2;

/// First descriptor available to guest-opened files.
pub const FIRST_FILE_FD: i32 = 3;

/// Guest `open` mode for an existing read-only file.
pub const OPEN_READ: i32 = 0;

/// Guest `open` mode for a truncate-or-create writable file.
pub const OPEN_WRITE_TRUNCATE: i32 = 1;

/// Maximum UTF-8 path size admitted by the private seed ABI.
pub const MAX_PATH_BYTES: usize = 4096;

/// Maximum combined UTF-8 argument bytes admitted for one process.
pub const MAX_ARGUMENT_BYTES: usize = 32 * 1024;

/// Realm identifier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct RealmId(u64);

impl RealmId {
    /// Creates an identifier from its stable realm-local representation.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the realm-local representation.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Process identifier, unique within one realm generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ProcessId(u64);

impl ProcessId {
    /// Creates an identifier from its realm-local representation.
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the realm-local representation.
    pub const fn get(self) -> u64 {
        self.0
    }
}

/// Descriptor number in a single process descriptor table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Descriptor(i32);

impl Descriptor {
    /// Standard output.
    pub const STDOUT: Self = Self(STDOUT_FD);

    /// Standard error.
    pub const STDERR: Self = Self(STDERR_FD);

    /// Creates a descriptor from its guest representation.
    pub const fn new(value: i32) -> Self {
        Self(value)
    }

    /// Returns the guest representation.
    pub const fn get(self) -> i32 {
        self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn domain_identifiers_keep_their_types() {
        let realm = RealmId::new(7);
        let process = ProcessId::new(7);

        assert_eq!(realm.get(), process.get());
        assert_eq!(Descriptor::STDOUT.get(), STDOUT_FD);
        assert_eq!(Descriptor::STDERR.get(), STDERR_FD);
        assert_eq!(Descriptor::new(FIRST_FILE_FD).get(), FIRST_FILE_FD);
    }
}
