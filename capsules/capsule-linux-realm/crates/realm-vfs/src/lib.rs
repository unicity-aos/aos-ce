#![deny(unsafe_code)]
#![deny(clippy::all)]
#![deny(unreachable_pub)]

//! Versioned metadata and content-addressed storage for the AOS Realm home.
//!
//! The bulk bytes live behind [`RealmStore::put_blob`]. A single raw head value
//! lives in principal-scoped KV and moves with atomic compare-and-swap. Manifests
//! and file contents are immutable blobs, so a failed or interrupted commit can
//! leave unreachable objects but cannot expose a half-selected generation.

use serde::{Deserialize, Deserializer, Serialize, Serializer, de};
use std::{collections::BTreeMap, fmt};

/// On-disk metadata format understood by this implementation.
pub const FORMAT_VERSION: u32 = 1;

/// Maximum bytes in one file admitted by the current command seed.
pub const MAX_FILE_BYTES: usize = 64 * 1024;

/// Maximum serialized manifest size admitted by the seed.
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

/// Number of optimistic head-swap attempts before reporting contention.
pub const CAS_RETRY_LIMIT: usize = 8;

/// BLAKE3 identity of one immutable blob.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct BlobDigest(String);

impl BlobDigest {
    /// Hash bytes into their canonical lowercase BLAKE3 identity.
    #[must_use]
    pub fn for_bytes(bytes: &[u8]) -> Self {
        Self(blake3::hash(bytes).to_hex().to_string())
    }

    /// Validate and construct a digest received from stored metadata.
    pub fn parse(value: String) -> Result<Self, FsError> {
        if value.len() == 64
            && value
                .bytes()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
        {
            Ok(Self(value))
        } else {
            Err(FsError::Corrupt(
                "blob digest is not 64 lowercase hexadecimal characters".to_string(),
            ))
        }
    }

    /// Return the canonical digest text.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Serialize for BlobDigest {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for BlobDigest {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(value).map_err(de::Error::custom)
    }
}

/// Stable storage failures exposed by a realm store adapter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StoreError {
    /// The outer store denied this operation.
    Denied,
    /// A store quota or configured size bound was exceeded.
    TooLarge,
    /// Stored bytes do not match their content identity.
    Corrupt(String),
    /// Another storage failure.
    Io(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Denied => formatter.write_str("store access denied"),
            Self::TooLarge => formatter.write_str("store value is too large"),
            Self::Corrupt(message) => write!(formatter, "store corruption: {message}"),
            Self::Io(message) => write!(formatter, "store I/O failure: {message}"),
        }
    }
}

impl std::error::Error for StoreError {}

/// Metadata-layer failure.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FsError {
    /// The named file is absent from the selected generation.
    NotFound,
    /// The relative realm-home path is malformed.
    InvalidPath,
    /// A file or manifest exceeds a configured bound.
    TooLarge,
    /// Stored metadata or content failed validation.
    Corrupt(String),
    /// Concurrent writers exceeded the bounded retry policy.
    Contended,
    /// The outer store failed.
    Store(StoreError),
    /// Metadata serialization or deserialization failed.
    Serialization(String),
}

impl fmt::Display for FsError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotFound => formatter.write_str("file not found"),
            Self::InvalidPath => formatter.write_str("invalid realm-home path"),
            Self::TooLarge => formatter.write_str("realm filesystem value is too large"),
            Self::Corrupt(message) => write!(formatter, "realm filesystem corruption: {message}"),
            Self::Contended => formatter.write_str("realm filesystem head remained contended"),
            Self::Store(error) => error.fmt(formatter),
            Self::Serialization(message) => {
                write!(formatter, "realm metadata serialization failed: {message}")
            }
        }
    }
}

impl std::error::Error for FsError {}

impl From<StoreError> for FsError {
    fn from(error: StoreError) -> Self {
        Self::Store(error)
    }
}

/// Store boundary required by the versioned filesystem.
pub trait RealmStore {
    /// Read the exact raw head bytes used as a future CAS expectation.
    fn read_head(&self) -> Result<Option<Vec<u8>>, StoreError>;

    /// Replace the raw head iff it still equals `expected`.
    fn compare_and_swap_head(
        &mut self,
        expected: Option<&[u8]>,
        new: &[u8],
    ) -> Result<bool, StoreError>;

    /// Read an immutable blob by content identity.
    fn get_blob(&self, digest: &BlobDigest) -> Result<Option<Vec<u8>>, StoreError>;

    /// Idempotently materialize an immutable blob.
    fn put_blob(&mut self, digest: &BlobDigest, bytes: &[u8]) -> Result<(), StoreError>;
}

/// One file selected by a manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct FileRecord {
    blob: BlobDigest,
    bytes: u64,
}

/// Immutable snapshot manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct Manifest {
    format: u32,
    generation: u64,
    parent_manifest: Option<BlobDigest>,
    files: BTreeMap<String, FileRecord>,
}

impl Manifest {
    fn empty() -> Self {
        Self {
            format: FORMAT_VERSION,
            generation: 0,
            parent_manifest: None,
            files: BTreeMap::new(),
        }
    }
}

/// The sole mutable filesystem metadata value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
struct HeadRecord {
    format: u32,
    generation: u64,
    manifest: BlobDigest,
}

struct LoadedSnapshot {
    raw_head: Option<Vec<u8>>,
    manifest_digest: Option<BlobDigest>,
    manifest: Manifest,
}

/// Observable metadata for the current selected generation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct FsStatus {
    /// Metadata format version.
    pub format: u32,
    /// Monotonic selected generation number.
    pub generation: u64,
    /// Number of files in the selected manifest.
    pub files: usize,
    /// Content identity of the selected manifest, absent for the empty genesis.
    pub manifest: Option<BlobDigest>,
}

/// Receipt for one successful head transition.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct CommitReceipt {
    /// New selected generation.
    pub generation: u64,
    /// New selected manifest identity.
    pub manifest: BlobDigest,
    /// File-content identity selected by this write.
    pub file_blob: BlobDigest,
}

/// Versioned filesystem over a caller-supplied principal store.
pub struct RealmFs<S> {
    store: S,
}

impl<S: RealmStore> RealmFs<S> {
    /// Bind a filesystem instance to a store already scoped to one principal.
    pub const fn new(store: S) -> Self {
        Self { store }
    }

    /// Read one file from the currently selected manifest.
    pub fn read_file(&self, path: &str) -> Result<Vec<u8>, FsError> {
        validate_relative_path(path)?;
        let snapshot = self.load_snapshot()?;
        let record = snapshot.manifest.files.get(path).ok_or(FsError::NotFound)?;
        let bytes = self.store.get_blob(&record.blob)?.ok_or_else(|| {
            FsError::Corrupt(format!("missing file blob {}", record.blob.as_str()))
        })?;
        verify_blob(&record.blob, &bytes)?;
        if u64::try_from(bytes.len()).map_err(|_| FsError::TooLarge)? != record.bytes {
            return Err(FsError::Corrupt(format!(
                "file length does not match manifest for {path}"
            )));
        }
        Ok(bytes)
    }

    /// Commit a create-or-truncate file replacement as one new generation.
    pub fn write_file(&mut self, path: &str, bytes: &[u8]) -> Result<CommitReceipt, FsError> {
        validate_relative_path(path)?;
        if bytes.len() > MAX_FILE_BYTES {
            return Err(FsError::TooLarge);
        }

        let file_blob = BlobDigest::for_bytes(bytes);
        self.put_verified_blob(&file_blob, bytes)?;

        for _ in 0..CAS_RETRY_LIMIT {
            let snapshot = self.load_snapshot()?;
            let generation = snapshot
                .manifest
                .generation
                .checked_add(1)
                .ok_or(FsError::TooLarge)?;
            let mut manifest = snapshot.manifest;
            manifest.generation = generation;
            manifest.parent_manifest = snapshot.manifest_digest;
            manifest.files.insert(
                path.to_string(),
                FileRecord {
                    blob: file_blob.clone(),
                    bytes: u64::try_from(bytes.len()).map_err(|_| FsError::TooLarge)?,
                },
            );
            let manifest_bytes = encode(&manifest)?;
            if manifest_bytes.len() > MAX_MANIFEST_BYTES {
                return Err(FsError::TooLarge);
            }
            let manifest_digest = BlobDigest::for_bytes(&manifest_bytes);
            self.put_verified_blob(&manifest_digest, &manifest_bytes)?;

            let head = HeadRecord {
                format: FORMAT_VERSION,
                generation,
                manifest: manifest_digest.clone(),
            };
            let head_bytes = encode(&head)?;
            if self
                .store
                .compare_and_swap_head(snapshot.raw_head.as_deref(), &head_bytes)?
            {
                return Ok(CommitReceipt {
                    generation,
                    manifest: manifest_digest,
                    file_blob,
                });
            }
        }

        Err(FsError::Contended)
    }

    /// Inspect the selected generation without mutating the store.
    pub fn status(&self) -> Result<FsStatus, FsError> {
        let snapshot = self.load_snapshot()?;
        Ok(FsStatus {
            format: FORMAT_VERSION,
            generation: snapshot.manifest.generation,
            files: snapshot.manifest.files.len(),
            manifest: snapshot.manifest_digest,
        })
    }

    fn put_verified_blob(&mut self, digest: &BlobDigest, bytes: &[u8]) -> Result<(), FsError> {
        self.store.put_blob(digest, bytes)?;
        let stored = self.store.get_blob(digest)?.ok_or_else(|| {
            FsError::Corrupt(format!("blob {} vanished after write", digest.as_str()))
        })?;
        verify_blob(digest, &stored)
    }

    fn load_snapshot(&self) -> Result<LoadedSnapshot, FsError> {
        let Some(raw_head) = self.store.read_head()? else {
            return Ok(LoadedSnapshot {
                raw_head: None,
                manifest_digest: None,
                manifest: Manifest::empty(),
            });
        };
        let head: HeadRecord = decode(&raw_head)?;
        if head.format != FORMAT_VERSION {
            return Err(FsError::Corrupt(format!(
                "unsupported head format {}",
                head.format
            )));
        }
        let manifest_bytes = self
            .store
            .get_blob(&head.manifest)?
            .ok_or_else(|| FsError::Corrupt("selected manifest blob is missing".to_string()))?;
        verify_blob(&head.manifest, &manifest_bytes)?;
        let manifest: Manifest = decode(&manifest_bytes)?;
        if manifest.format != FORMAT_VERSION || manifest.generation != head.generation {
            return Err(FsError::Corrupt(
                "head and selected manifest metadata disagree".to_string(),
            ));
        }
        Ok(LoadedSnapshot {
            raw_head: Some(raw_head),
            manifest_digest: Some(head.manifest),
            manifest,
        })
    }
}

fn validate_relative_path(path: &str) -> Result<(), FsError> {
    if path.is_empty()
        || path.len() > 4096
        || path.starts_with('/')
        || path.contains('\\')
        || path.split('/').any(|component| {
            component.is_empty()
                || component == "."
                || component == ".."
                || component.chars().any(char::is_control)
        })
    {
        Err(FsError::InvalidPath)
    } else {
        Ok(())
    }
}

fn verify_blob(expected: &BlobDigest, bytes: &[u8]) -> Result<(), FsError> {
    let actual = BlobDigest::for_bytes(bytes);
    if &actual == expected {
        Ok(())
    } else {
        Err(FsError::Corrupt(format!(
            "blob {} contains bytes for {}",
            expected.as_str(),
            actual.as_str()
        )))
    }
}

fn encode<T: Serialize>(value: &T) -> Result<Vec<u8>, FsError> {
    serde_json::to_vec(value).map_err(|error| FsError::Serialization(error.to_string()))
}

fn decode<'a, T: Deserialize<'a>>(bytes: &'a [u8]) -> Result<T, FsError> {
    serde_json::from_slice(bytes).map_err(|error| FsError::Serialization(error.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, rc::Rc};

    #[derive(Clone, Default)]
    struct MemoryStore {
        inner: Rc<RefCell<MemoryState>>,
    }

    #[derive(Default)]
    struct MemoryState {
        head: Option<Vec<u8>>,
        blobs: BTreeMap<BlobDigest, Vec<u8>>,
        forced_cas_misses: usize,
        competing_head_on_next_cas: Option<Vec<u8>>,
    }

    impl MemoryStore {
        fn force_cas_misses(&self, count: usize) {
            self.inner.borrow_mut().forced_cas_misses = count;
        }

        fn replace_blob(&self, digest: BlobDigest, bytes: Vec<u8>) {
            self.inner.borrow_mut().blobs.insert(digest, bytes);
        }

        fn stage_competing_file(&self, path: &str, bytes: &[u8]) -> BlobDigest {
            let file_blob = BlobDigest::for_bytes(bytes);
            let mut files = BTreeMap::new();
            files.insert(
                path.to_string(),
                FileRecord {
                    blob: file_blob.clone(),
                    bytes: u64::try_from(bytes.len()).expect("test file length fits"),
                },
            );
            let manifest = Manifest {
                format: FORMAT_VERSION,
                generation: 1,
                parent_manifest: None,
                files,
            };
            let manifest_bytes = encode(&manifest).expect("competitor manifest encodes");
            let manifest_digest = BlobDigest::for_bytes(&manifest_bytes);
            let head = HeadRecord {
                format: FORMAT_VERSION,
                generation: 1,
                manifest: manifest_digest.clone(),
            };
            let mut state = self.inner.borrow_mut();
            state.blobs.insert(file_blob, bytes.to_vec());
            state.blobs.insert(manifest_digest, manifest_bytes);
            state.competing_head_on_next_cas =
                Some(encode(&head).expect("competitor head encodes"));
            head.manifest
        }
    }

    impl RealmStore for MemoryStore {
        fn read_head(&self) -> Result<Option<Vec<u8>>, StoreError> {
            Ok(self.inner.borrow().head.clone())
        }

        fn compare_and_swap_head(
            &mut self,
            expected: Option<&[u8]>,
            new: &[u8],
        ) -> Result<bool, StoreError> {
            let mut state = self.inner.borrow_mut();
            if let Some(competing_head) = state.competing_head_on_next_cas.take() {
                state.head = Some(competing_head);
                return Ok(false);
            }
            if state.forced_cas_misses > 0 {
                state.forced_cas_misses -= 1;
                return Ok(false);
            }
            if state.head.as_deref() == expected {
                state.head = Some(new.to_vec());
                Ok(true)
            } else {
                Ok(false)
            }
        }

        fn get_blob(&self, digest: &BlobDigest) -> Result<Option<Vec<u8>>, StoreError> {
            Ok(self.inner.borrow().blobs.get(digest).cloned())
        }

        fn put_blob(&mut self, digest: &BlobDigest, bytes: &[u8]) -> Result<(), StoreError> {
            self.inner
                .borrow_mut()
                .blobs
                .entry(digest.clone())
                .or_insert_with(|| bytes.to_vec());
            Ok(())
        }
    }

    #[test]
    fn write_selects_content_and_manifest_with_one_head_swap() {
        let store = MemoryStore::default();
        let mut filesystem = RealmFs::new(store.clone());

        let receipt = filesystem
            .write_file("notes.txt", b"hello")
            .expect("write commits");

        assert_eq!(receipt.generation, 1);
        assert_eq!(filesystem.read_file("notes.txt"), Ok(b"hello".to_vec()));
        assert_eq!(
            filesystem.status(),
            Ok(FsStatus {
                format: FORMAT_VERSION,
                generation: 1,
                files: 1,
                manifest: Some(receipt.manifest),
            })
        );
    }

    #[test]
    fn generations_preserve_other_files_and_form_a_parent_chain() {
        let store = MemoryStore::default();
        let mut filesystem = RealmFs::new(store.clone());
        let first = filesystem.write_file("a", b"one").expect("first write");
        let second = filesystem.write_file("b", b"two").expect("second write");

        assert_eq!(second.generation, 2);
        assert_eq!(filesystem.read_file("a"), Ok(b"one".to_vec()));
        assert_eq!(filesystem.read_file("b"), Ok(b"two".to_vec()));

        let second_manifest_bytes = store
            .get_blob(&second.manifest)
            .expect("store read")
            .expect("manifest exists");
        let second_manifest: Manifest = decode(&second_manifest_bytes).expect("manifest decodes");
        assert_eq!(second_manifest.parent_manifest, Some(first.manifest));
    }

    #[test]
    fn a_new_filesystem_instance_reconstructs_the_selected_generation() {
        let store = MemoryStore::default();
        let mut before_restart = RealmFs::new(store.clone());
        let receipt = before_restart
            .write_file("state/session.json", br#"{"cwd":"/workspace"}"#)
            .expect("state commits");
        drop(before_restart);

        let after_restart = RealmFs::new(store);

        assert_eq!(
            after_restart.read_file("state/session.json"),
            Ok(br#"{"cwd":"/workspace"}"#.to_vec())
        );
        assert_eq!(
            after_restart.status().expect("status after restart"),
            FsStatus {
                format: FORMAT_VERSION,
                generation: 1,
                files: 1,
                manifest: Some(receipt.manifest),
            }
        );
    }

    #[test]
    fn lost_head_race_reloads_and_merges_the_winning_generation() {
        let store = MemoryStore::default();
        let competing_manifest = store.stage_competing_file("other.txt", b"other writer");
        let mut filesystem = RealmFs::new(store.clone());

        let receipt = filesystem
            .write_file("race.txt", b"winner")
            .expect("bounded retry succeeds");

        assert_eq!(receipt.generation, 2);
        assert_eq!(
            filesystem.read_file("other.txt"),
            Ok(b"other writer".to_vec())
        );
        assert_eq!(filesystem.read_file("race.txt"), Ok(b"winner".to_vec()));
        let manifest_bytes = store
            .get_blob(&receipt.manifest)
            .expect("store read")
            .expect("manifest exists");
        let manifest: Manifest = decode(&manifest_bytes).expect("manifest decodes");
        assert_eq!(manifest.parent_manifest, Some(competing_manifest));
    }

    #[test]
    fn persistent_head_contention_is_bounded_and_selects_nothing() {
        let store = MemoryStore::default();
        store.force_cas_misses(CAS_RETRY_LIMIT);
        let mut filesystem = RealmFs::new(store.clone());

        assert_eq!(
            filesystem.write_file("race.txt", b"never selected"),
            Err(FsError::Contended)
        );
        assert_eq!(filesystem.status().expect("status").generation, 0);
        assert!(store.read_head().expect("head reads").is_none());
    }

    #[test]
    fn unselected_orphan_blob_does_not_change_the_visible_generation() {
        let store = MemoryStore::default();
        let orphan = BlobDigest::for_bytes(b"orphan");
        store
            .clone()
            .put_blob(&orphan, b"orphan")
            .expect("orphan materializes");
        let filesystem = RealmFs::new(store);

        assert_eq!(filesystem.read_file("orphan"), Err(FsError::NotFound));
        assert_eq!(filesystem.status().expect("status").generation, 0);
    }

    #[test]
    fn corrupted_selected_blob_fails_closed() {
        let store = MemoryStore::default();
        let mut filesystem = RealmFs::new(store.clone());
        let receipt = filesystem
            .write_file("important", b"correct")
            .expect("write commits");
        store.replace_blob(receipt.file_blob, b"tampered".to_vec());

        assert!(matches!(
            filesystem.read_file("important"),
            Err(FsError::Corrupt(_))
        ));
    }

    #[test]
    fn a_missing_selected_manifest_fails_closed() {
        let store = MemoryStore::default();
        let missing = BlobDigest::for_bytes(b"missing manifest");
        store.inner.borrow_mut().head = Some(
            encode(&HeadRecord {
                format: FORMAT_VERSION,
                generation: 1,
                manifest: missing,
            })
            .expect("head encodes"),
        );
        let filesystem = RealmFs::new(store);

        assert!(matches!(filesystem.status(), Err(FsError::Corrupt(_))));
    }

    #[test]
    fn path_and_file_bounds_fail_before_head_mutation() {
        let store = MemoryStore::default();
        let mut filesystem = RealmFs::new(store.clone());

        assert_eq!(
            filesystem.write_file("../escape", b"x"),
            Err(FsError::InvalidPath)
        );
        assert_eq!(
            filesystem.write_file("large", &vec![0; MAX_FILE_BYTES + 1]),
            Err(FsError::TooLarge)
        );
        assert!(store.read_head().expect("head reads").is_none());
    }

    #[test]
    fn newer_or_malformed_metadata_is_never_interpreted() {
        let store = MemoryStore::default();
        store.inner.borrow_mut().head = Some(
            br#"{"format":2,"generation":1,"manifest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"}"#
                .to_vec(),
        );
        let filesystem = RealmFs::new(store);

        assert!(matches!(filesystem.status(), Err(FsError::Corrupt(_))));
        assert!(BlobDigest::parse("A".repeat(64)).is_err());
    }
}
