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
pub const FORMAT_VERSION: u32 = 3;

const LEGACY_FORMAT_VERSION: u32 = 1;
const WHOLE_BLOB_FORMAT_VERSION: u32 = 2;
const DEFAULT_FILE_MODE: u32 = 0o644;
const DEFAULT_DIRECTORY_MODE: u32 = 0o755;
const ROOT_DIRECTORY_MODE: u32 = 0o700;

/// Bytes in one immutable file-content chunk.
///
/// This is an internal storage and 9P transfer unit, not a logical file-size
/// policy. Sparse files and chunk trees can address much larger files.
pub const FILE_CHUNK_BYTES: usize = 64 * 1024;

const CHUNK_TREE_FANOUT: u64 = 256;
const CHUNK_TREE_LEVELS: u8 = 3;

/// Maximum logical file size addressable by the fixed-depth chunk tree.
///
/// Aggregate durable bytes remain governed by the outer principal storage
/// ledger. This ceiling protects offset arithmetic and the persistent format;
/// it is not the default allocation granted to a Realm.
pub const MAX_LOGICAL_FILE_BYTES: u64 =
    FILE_CHUNK_BYTES as u64 * CHUNK_TREE_FANOUT * CHUNK_TREE_FANOUT * CHUNK_TREE_FANOUT;

/// Maximum serialized manifest or chunk-node size accepted from storage.
pub const MAX_MANIFEST_BYTES: usize = 1024 * 1024;

const MAX_HEAD_BYTES: usize = 1024;

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
    /// A create or rename destination already exists.
    AlreadyExists,
    /// A file operation selected a directory.
    IsDirectory,
    /// A directory operation selected a regular file or missing parent.
    NotDirectory,
    /// A directory removal or replacement selected a non-empty directory.
    NotEmpty,
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
            Self::AlreadyExists => formatter.write_str("filesystem node already exists"),
            Self::IsDirectory => formatter.write_str("filesystem node is a directory"),
            Self::NotDirectory => formatter.write_str("filesystem node is not a directory"),
            Self::NotEmpty => formatter.write_str("directory is not empty"),
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
#[serde(deny_unknown_fields)]
struct FileRecord {
    /// Format-1/2 whole-file content. Format 3 retains this only while a
    /// pre-existing file has not yet been mutated.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    blob: Option<BlobDigest>,
    /// Root of the sparse format-3 content tree. Absence denotes all zeroes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    tree: Option<BlobDigest>,
    bytes: u64,
    #[serde(default = "default_file_mode")]
    mode: u32,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ChunkNode {
    level: u8,
    children: BTreeMap<u8, BlobDigest>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct DirectoryRecord {
    #[serde(default = "default_directory_mode")]
    mode: u32,
}

/// Immutable snapshot manifest.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Manifest {
    format: u32,
    generation: u64,
    parent_manifest: Option<BlobDigest>,
    files: BTreeMap<String, FileRecord>,
    #[serde(default)]
    directories: BTreeMap<String, DirectoryRecord>,
}

impl Manifest {
    fn empty() -> Self {
        Self {
            format: FORMAT_VERSION,
            generation: 0,
            parent_manifest: None,
            files: BTreeMap::new(),
            directories: BTreeMap::new(),
        }
    }
}

const fn default_file_mode() -> u32 {
    DEFAULT_FILE_MODE
}

const fn default_directory_mode() -> u32 {
    DEFAULT_DIRECTORY_MODE
}

/// Node category stored in one selected Realm generation.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FsNodeKind {
    /// Regular byte-addressable file.
    File,
    /// Directory containing files or other directories.
    Directory,
}

/// Metadata for one node in the selected Realm generation.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsMetadata {
    /// Stored node category.
    pub kind: FsNodeKind,
    /// File byte length, or zero for a directory.
    pub bytes: u64,
    /// Persisted Unix permission bits.
    pub mode: u32,
    /// Selected generation that supplied this metadata.
    pub generation: u64,
}

/// One immediate child of a selected directory.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FsDirectoryEntry {
    /// Single normalized path component.
    pub name: String,
    /// Metadata captured from the same selected generation.
    pub metadata: FsMetadata,
}

/// The sole mutable filesystem metadata value.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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

type StagedBlob = (BlobDigest, Vec<u8>);
type StagedBlobs = Vec<StagedBlob>;
type BuiltTree = (Option<BlobDigest>, StagedBlobs);

enum ManifestChange<T> {
    Unchanged(T),
    Changed { value: T, blobs: StagedBlobs },
}

struct MutationResult<T> {
    value: T,
    generation: u64,
    manifest: Option<BlobDigest>,
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
    /// Root content identity selected by this write.
    ///
    /// `None` denotes an empty or entirely sparse-zero file.
    pub content_root: Option<BlobDigest>,
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

    /// Inspect one file or directory in the currently selected generation.
    /// The empty path denotes the export root.
    pub fn metadata(&self, path: &str) -> Result<FsMetadata, FsError> {
        if !path.is_empty() {
            validate_relative_path(path)?;
        }
        let snapshot = self.load_snapshot()?;
        metadata_in(&snapshot.manifest, path)
    }

    /// Enumerate one directory in stable name order.
    pub fn read_dir(&self, path: &str) -> Result<Vec<FsDirectoryEntry>, FsError> {
        if !path.is_empty() {
            validate_relative_path(path)?;
        }
        let snapshot = self.load_snapshot()?;
        if metadata_in(&snapshot.manifest, path)?.kind != FsNodeKind::Directory {
            return Err(FsError::NotDirectory);
        }
        let mut entries = BTreeMap::<String, FsMetadata>::new();
        for candidate in snapshot
            .manifest
            .directories
            .keys()
            .chain(snapshot.manifest.files.keys())
        {
            let Some(name) = immediate_child(path, candidate) else {
                continue;
            };
            let child = if path.is_empty() {
                name.to_string()
            } else {
                format!("{path}/{name}")
            };
            entries
                .entry(name.to_string())
                .or_insert(metadata_in(&snapshot.manifest, &child)?);
        }
        Ok(entries
            .into_iter()
            .map(|(name, metadata)| FsDirectoryEntry { name, metadata })
            .collect())
    }

    /// Read one file from the currently selected manifest.
    pub fn read_file(&self, path: &str) -> Result<Vec<u8>, FsError> {
        validate_relative_path(path)?;
        let snapshot = self.load_snapshot()?;
        if snapshot.manifest.directories.contains_key(path) {
            return Err(FsError::IsDirectory);
        }
        let record = snapshot.manifest.files.get(path).ok_or(FsError::NotFound)?;
        read_record_bytes(&self.store, path, record)
    }

    /// Read at most `count` bytes from one file without materializing the rest
    /// of its content.
    pub fn read_at(&self, path: &str, offset: u64, count: usize) -> Result<Vec<u8>, FsError> {
        validate_relative_path(path)?;
        let snapshot = self.load_snapshot()?;
        if snapshot.manifest.directories.contains_key(path) {
            return Err(FsError::IsDirectory);
        }
        let record = snapshot.manifest.files.get(path).ok_or(FsError::NotFound)?;
        read_record_at(&self.store, path, record, offset, count)
    }

    /// Commit a create-or-truncate file replacement as one new generation.
    pub fn write_file(&mut self, path: &str, bytes: &[u8]) -> Result<CommitReceipt, FsError> {
        validate_relative_path(path)?;
        let byte_len = u64::try_from(bytes.len()).map_err(|_| FsError::TooLarge)?;
        if byte_len > MAX_LOGICAL_FILE_BYTES {
            return Err(FsError::TooLarge);
        }

        let result = self.mutate(|store, manifest| {
            if manifest.directories.contains_key(path) {
                return Err(FsError::IsDirectory);
            }
            ensure_parent_directories(manifest, path)?;
            let mode = manifest
                .files
                .get(path)
                .map_or(DEFAULT_FILE_MODE, |record| record.mode);
            let (tree, blobs) = build_tree(store, bytes)?;
            manifest.files.insert(
                path.to_string(),
                FileRecord {
                    blob: None,
                    tree: tree.clone(),
                    bytes: byte_len,
                    mode,
                },
            );
            Ok(ManifestChange::Changed { value: tree, blobs })
        })?;
        Ok(CommitReceipt {
            generation: result.generation,
            manifest: result.manifest.ok_or_else(|| {
                FsError::Corrupt("a file replacement selected no manifest".to_string())
            })?,
            content_root: result.value,
        })
    }

    /// Replace a byte range and atomically select the resulting file.
    pub fn write_at(&mut self, path: &str, offset: u64, data: &[u8]) -> Result<u32, FsError> {
        validate_relative_path(path)?;
        let data_len = u64::try_from(data.len()).map_err(|_| FsError::TooLarge)?;
        let end = offset.checked_add(data_len).ok_or(FsError::TooLarge)?;
        if end > MAX_LOGICAL_FILE_BYTES {
            return Err(FsError::TooLarge);
        }
        let result = self.mutate(|store, manifest| {
            if manifest.directories.contains_key(path) {
                return Err(FsError::IsDirectory);
            }
            let record = manifest.files.get(path).ok_or(FsError::NotFound)?.clone();
            if data.is_empty() {
                return Ok(ManifestChange::Unchanged(0));
            }
            let (mut tree, mut blobs) = tree_for_mutation(store, path, &record)?;
            let mut source_offset = 0_usize;
            let first_chunk = offset / FILE_CHUNK_BYTES as u64;
            let last_chunk = (end - 1) / FILE_CHUNK_BYTES as u64;
            for chunk_index in first_chunk..=last_chunk {
                let mut chunk = read_chunk_with_pending(store, tree.as_ref(), chunk_index, &blobs)?;
                let chunk_start = chunk_index * FILE_CHUNK_BYTES as u64;
                let write_start = offset.max(chunk_start) - chunk_start;
                let write_end = end.min(chunk_start + FILE_CHUNK_BYTES as u64) - chunk_start;
                let write_len =
                    usize::try_from(write_end - write_start).map_err(|_| FsError::TooLarge)?;
                let destination = usize::try_from(write_start).map_err(|_| FsError::TooLarge)?;
                chunk[destination..destination + write_len]
                    .copy_from_slice(&data[source_offset..source_offset + write_len]);
                source_offset += write_len;
                tree = update_chunk(store, tree.as_ref(), chunk_index, &chunk, &mut blobs)?;
            }
            manifest.files.insert(
                path.to_string(),
                FileRecord {
                    blob: None,
                    tree,
                    bytes: record.bytes.max(end),
                    mode: record.mode,
                },
            );
            Ok(ManifestChange::Changed {
                value: u32::try_from(data.len()).map_err(|_| FsError::TooLarge)?,
                blobs,
            })
        })?;
        Ok(result.value)
    }

    /// Create a regular file, applying exclusive and truncate semantics in one
    /// selected generation.
    pub fn create_file(
        &mut self,
        path: &str,
        mode: u32,
        exclusive: bool,
        truncate: bool,
    ) -> Result<(), FsError> {
        validate_relative_path(path)?;
        self.mutate(|_, manifest| {
            require_parent_directory(manifest, path)?;
            if manifest.directories.contains_key(path) {
                return Err(FsError::IsDirectory);
            }
            if let Some(record) = manifest.files.get(path) {
                if exclusive {
                    return Err(FsError::AlreadyExists);
                }
                if !truncate {
                    return Ok(ManifestChange::Unchanged(()));
                }
                let retained_mode = record.mode;
                manifest.files.insert(
                    path.to_string(),
                    FileRecord {
                        blob: None,
                        tree: None,
                        bytes: 0,
                        mode: retained_mode,
                    },
                );
            } else {
                manifest.files.insert(
                    path.to_string(),
                    FileRecord {
                        blob: None,
                        tree: None,
                        bytes: 0,
                        mode: mode & 0o7777,
                    },
                );
            }
            Ok(ManifestChange::Changed {
                value: (),
                blobs: Vec::new(),
            })
        })?;
        Ok(())
    }

    /// Create one empty directory beneath an existing directory.
    pub fn create_dir(&mut self, path: &str, mode: u32) -> Result<(), FsError> {
        validate_relative_path(path)?;
        self.mutate(|_, manifest| {
            require_parent_directory(manifest, path)?;
            if node_exists(manifest, path) {
                return Err(FsError::AlreadyExists);
            }
            manifest.directories.insert(
                path.to_string(),
                DirectoryRecord {
                    mode: mode & 0o7777,
                },
            );
            Ok(ManifestChange::Changed {
                value: (),
                blobs: Vec::new(),
            })
        })?;
        Ok(())
    }

    /// Truncate or zero-extend one regular file atomically.
    pub fn set_len(&mut self, path: &str, len: u64) -> Result<(), FsError> {
        validate_relative_path(path)?;
        if len > MAX_LOGICAL_FILE_BYTES {
            return Err(FsError::TooLarge);
        }
        self.mutate(|store, manifest| {
            if manifest.directories.contains_key(path) {
                return Err(FsError::IsDirectory);
            }
            let record = manifest.files.get(path).ok_or(FsError::NotFound)?.clone();
            if record.bytes == len {
                return Ok(ManifestChange::Unchanged(()));
            }
            let (mut tree, mut blobs) = tree_for_mutation(store, path, &record)?;
            if len < record.bytes {
                let retained_chunks = len.div_ceil(FILE_CHUNK_BYTES as u64);
                tree = prune_tree(store, tree.as_ref(), retained_chunks, &mut blobs)?;
                let tail = len % FILE_CHUNK_BYTES as u64;
                if tail != 0 {
                    let chunk_index = len / FILE_CHUNK_BYTES as u64;
                    let mut chunk =
                        read_chunk_with_pending(store, tree.as_ref(), chunk_index, &blobs)?;
                    let tail = usize::try_from(tail).map_err(|_| FsError::TooLarge)?;
                    chunk[tail..].fill(0);
                    tree = update_chunk(store, tree.as_ref(), chunk_index, &chunk, &mut blobs)?;
                }
            }
            manifest.files.insert(
                path.to_string(),
                FileRecord {
                    blob: None,
                    tree,
                    bytes: len,
                    mode: record.mode,
                },
            );
            Ok(ManifestChange::Changed { value: (), blobs })
        })?;
        Ok(())
    }

    /// Replace one node's POSIX permission and special bits in a selected
    /// generation without changing its content or kind.
    pub fn set_mode(&mut self, path: &str, mode: u32) -> Result<(), FsError> {
        validate_relative_path(path)?;
        let mode = mode & 0o7777;
        self.mutate(|_, manifest| {
            if let Some(record) = manifest.files.get_mut(path) {
                if record.mode == mode {
                    return Ok(ManifestChange::Unchanged(()));
                }
                record.mode = mode;
            } else if let Some(record) = manifest.directories.get_mut(path) {
                if record.mode == mode {
                    return Ok(ManifestChange::Unchanged(()));
                }
                record.mode = mode;
            } else {
                return Err(FsError::NotFound);
            }
            Ok(ManifestChange::Changed {
                value: (),
                blobs: Vec::new(),
            })
        })?;
        Ok(())
    }

    /// Remove one regular file in a new selected generation.
    pub fn remove_file(&mut self, path: &str) -> Result<(), FsError> {
        validate_relative_path(path)?;
        self.mutate(|_, manifest| {
            if manifest.directories.contains_key(path) {
                return Err(FsError::IsDirectory);
            }
            manifest.files.remove(path).ok_or(FsError::NotFound)?;
            Ok(ManifestChange::Changed {
                value: (),
                blobs: Vec::new(),
            })
        })?;
        Ok(())
    }

    /// Remove one empty directory in a new selected generation.
    pub fn remove_dir(&mut self, path: &str) -> Result<(), FsError> {
        validate_relative_path(path)?;
        self.mutate(|_, manifest| {
            if manifest.files.contains_key(path) {
                return Err(FsError::NotDirectory);
            }
            if !manifest.directories.contains_key(path) {
                return Err(FsError::NotFound);
            }
            if has_descendant(manifest, path) {
                return Err(FsError::NotEmpty);
            }
            manifest.directories.remove(path);
            Ok(ManifestChange::Changed {
                value: (),
                blobs: Vec::new(),
            })
        })?;
        Ok(())
    }

    /// Atomically rename one file or directory tree within this filesystem.
    pub fn rename(&mut self, source: &str, destination: &str) -> Result<(), FsError> {
        validate_relative_path(source)?;
        validate_relative_path(destination)?;
        if source == destination {
            return Ok(());
        }
        self.mutate(|_, manifest| {
            require_parent_directory(manifest, destination)?;
            let source_kind = node_kind(manifest, source).ok_or(FsError::NotFound)?;
            if source_kind == FsNodeKind::Directory
                && destination
                    .strip_prefix(source)
                    .is_some_and(|suffix| suffix.starts_with('/'))
            {
                return Err(FsError::InvalidPath);
            }
            match (source_kind, node_kind(manifest, destination)) {
                (_, None) => {}
                (FsNodeKind::File, Some(FsNodeKind::Directory)) => {
                    return Err(FsError::IsDirectory);
                }
                (FsNodeKind::Directory, Some(FsNodeKind::File)) => {
                    return Err(FsError::NotDirectory);
                }
                (FsNodeKind::Directory, Some(FsNodeKind::Directory))
                    if has_descendant(manifest, destination) =>
                {
                    return Err(FsError::NotEmpty);
                }
                _ => remove_node(manifest, destination),
            }
            rename_node(manifest, source, destination, source_kind);
            Ok(ManifestChange::Changed {
                value: (),
                blobs: Vec::new(),
            })
        })?;
        Ok(())
    }

    /// Inspect the selected generation without mutating the store.
    pub fn status(&self) -> Result<FsStatus, FsError> {
        let snapshot = self.load_snapshot()?;
        Ok(FsStatus {
            format: snapshot.manifest.format,
            generation: snapshot.manifest.generation,
            files: snapshot.manifest.files.len(),
            manifest: snapshot.manifest_digest,
        })
    }

    fn mutate<T>(
        &mut self,
        mut operation: impl FnMut(&S, &mut Manifest) -> Result<ManifestChange<T>, FsError>,
    ) -> Result<MutationResult<T>, FsError> {
        for _ in 0..CAS_RETRY_LIMIT {
            let snapshot = self.load_snapshot()?;
            let mut manifest = snapshot.manifest;
            let (value, blobs) = match operation(&self.store, &mut manifest)? {
                ManifestChange::Unchanged(value) => {
                    return Ok(MutationResult {
                        value,
                        generation: manifest.generation,
                        manifest: snapshot.manifest_digest,
                    });
                }
                ManifestChange::Changed { value, blobs } => (value, blobs),
            };

            let generation = manifest
                .generation
                .checked_add(1)
                .ok_or(FsError::TooLarge)?;
            manifest.format = FORMAT_VERSION;
            manifest.generation = generation;
            manifest.parent_manifest = snapshot.manifest_digest;
            for (digest, bytes) in blobs {
                self.put_verified_blob(&digest, &bytes)?;
            }
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
                return Ok(MutationResult {
                    value,
                    generation,
                    manifest: Some(manifest_digest),
                });
            }
        }
        Err(FsError::Contended)
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
        if raw_head.len() > MAX_HEAD_BYTES {
            return Err(FsError::Corrupt("selected head is oversized".to_string()));
        }
        let head: HeadRecord = decode(&raw_head)?;
        if !matches!(
            head.format,
            LEGACY_FORMAT_VERSION | WHOLE_BLOB_FORMAT_VERSION | FORMAT_VERSION
        ) {
            return Err(FsError::Corrupt(format!(
                "unsupported head format {}",
                head.format
            )));
        }
        let manifest_bytes = self
            .store
            .get_blob(&head.manifest)?
            .ok_or_else(|| FsError::Corrupt("selected manifest blob is missing".to_string()))?;
        if manifest_bytes.len() > MAX_MANIFEST_BYTES {
            return Err(FsError::Corrupt(
                "selected manifest is oversized".to_string(),
            ));
        }
        verify_blob(&head.manifest, &manifest_bytes)?;
        let mut manifest: Manifest = decode(&manifest_bytes)?;
        if manifest.format != head.format || manifest.generation != head.generation {
            return Err(FsError::Corrupt(
                "head and selected manifest metadata disagree".to_string(),
            ));
        }
        if manifest.format == LEGACY_FORMAT_VERSION {
            materialize_legacy_directories(&mut manifest)?;
        }
        validate_manifest(&manifest)?;
        Ok(LoadedSnapshot {
            raw_head: Some(raw_head),
            manifest_digest: Some(head.manifest),
            manifest,
        })
    }
}

fn validate_manifest(manifest: &Manifest) -> Result<(), FsError> {
    if manifest.generation == 0 || (manifest.generation == 1) != manifest.parent_manifest.is_none()
    {
        return Err(FsError::Corrupt(
            "manifest generation and parent disagree".to_string(),
        ));
    }
    for (path, record) in &manifest.files {
        validate_stored_path(path)?;
        if record.bytes > MAX_LOGICAL_FILE_BYTES || record.mode & !0o7777 != 0 {
            return Err(FsError::Corrupt(format!(
                "file metadata is outside bounds for {path:?}"
            )));
        }
        match manifest.format {
            LEGACY_FORMAT_VERSION | WHOLE_BLOB_FORMAT_VERSION => {
                if record.blob.is_none()
                    || record.tree.is_some()
                    || record.bytes > FILE_CHUNK_BYTES as u64
                {
                    return Err(FsError::Corrupt(format!(
                        "legacy file content is invalid for {path:?}"
                    )));
                }
            }
            FORMAT_VERSION => {
                if record.blob.is_some() && record.tree.is_some() {
                    return Err(FsError::Corrupt(format!(
                        "file selects two content representations for {path:?}"
                    )));
                }
                if record.blob.is_some() && record.bytes > FILE_CHUNK_BYTES as u64 {
                    return Err(FsError::Corrupt(format!(
                        "unmigrated file content is oversized for {path:?}"
                    )));
                }
            }
            _ => {
                return Err(FsError::Corrupt(format!(
                    "unsupported manifest format {}",
                    manifest.format
                )));
            }
        }
        if manifest.directories.contains_key(path)
            || node_kind(manifest, parent_path(path)) != Some(FsNodeKind::Directory)
        {
            return Err(FsError::Corrupt(format!(
                "file parent or node kind is invalid for {path:?}"
            )));
        }
    }
    for (path, record) in &manifest.directories {
        validate_stored_path(path)?;
        if record.mode & !0o7777 != 0
            || manifest.files.contains_key(path)
            || node_kind(manifest, parent_path(path)) != Some(FsNodeKind::Directory)
        {
            return Err(FsError::Corrupt(format!(
                "directory metadata is invalid for {path:?}"
            )));
        }
    }
    Ok(())
}

fn validate_stored_path(path: &str) -> Result<(), FsError> {
    validate_relative_path(path)
        .map_err(|_| FsError::Corrupt(format!("manifest contains invalid path {path:?}")))
}

fn read_record_bytes<S: RealmStore>(
    store: &S,
    path: &str,
    record: &FileRecord,
) -> Result<Vec<u8>, FsError> {
    let length = usize::try_from(record.bytes).map_err(|_| FsError::TooLarge)?;
    read_record_at(store, path, record, 0, length)
}

fn read_record_at<S: RealmStore>(
    store: &S,
    path: &str,
    record: &FileRecord,
    offset: u64,
    count: usize,
) -> Result<Vec<u8>, FsError> {
    if offset >= record.bytes || count == 0 {
        return Ok(Vec::new());
    }
    let count = u64::try_from(count).map_err(|_| FsError::TooLarge)?;
    let end = offset.saturating_add(count).min(record.bytes);
    if let Some(blob) = &record.blob {
        let bytes = store
            .get_blob(blob)?
            .ok_or_else(|| FsError::Corrupt(format!("missing file blob {}", blob.as_str())))?;
        verify_blob(blob, &bytes)?;
        if u64::try_from(bytes.len()).map_err(|_| FsError::TooLarge)? != record.bytes {
            return Err(FsError::Corrupt(format!(
                "file length does not match manifest for {path}"
            )));
        }
        let start = usize::try_from(offset).map_err(|_| FsError::TooLarge)?;
        let end = usize::try_from(end).map_err(|_| FsError::TooLarge)?;
        return Ok(bytes[start..end].to_vec());
    }

    let output_len = usize::try_from(end - offset).map_err(|_| FsError::TooLarge)?;
    let mut output = Vec::with_capacity(output_len);
    let first_chunk = offset / FILE_CHUNK_BYTES as u64;
    let last_chunk = (end - 1) / FILE_CHUNK_BYTES as u64;
    for chunk_index in first_chunk..=last_chunk {
        let chunk = read_chunk(store, record.tree.as_ref(), chunk_index)?;
        let chunk_start = chunk_index * FILE_CHUNK_BYTES as u64;
        let read_start = offset.max(chunk_start) - chunk_start;
        let read_end = end.min(chunk_start + FILE_CHUNK_BYTES as u64) - chunk_start;
        let read_start = usize::try_from(read_start).map_err(|_| FsError::TooLarge)?;
        let read_end = usize::try_from(read_end).map_err(|_| FsError::TooLarge)?;
        output.extend_from_slice(&chunk[read_start..read_end]);
    }
    Ok(output)
}

fn build_tree<S: RealmStore>(store: &S, bytes: &[u8]) -> Result<BuiltTree, FsError> {
    let mut tree = None;
    let mut blobs = Vec::new();
    for (chunk_index, source) in bytes.chunks(FILE_CHUNK_BYTES).enumerate() {
        let mut chunk = vec![0; FILE_CHUNK_BYTES];
        chunk[..source.len()].copy_from_slice(source);
        tree = update_chunk(
            store,
            tree.as_ref(),
            u64::try_from(chunk_index).map_err(|_| FsError::TooLarge)?,
            &chunk,
            &mut blobs,
        )?;
    }
    Ok((tree, blobs))
}

fn tree_for_mutation<S: RealmStore>(
    store: &S,
    path: &str,
    record: &FileRecord,
) -> Result<BuiltTree, FsError> {
    if record.blob.is_none() {
        return Ok((record.tree.clone(), Vec::new()));
    }
    let bytes = read_record_bytes(store, path, record)?;
    build_tree(store, &bytes)
}

fn read_chunk<S: RealmStore>(
    store: &S,
    root: Option<&BlobDigest>,
    chunk_index: u64,
) -> Result<Vec<u8>, FsError> {
    read_chunk_with_pending(store, root, chunk_index, &[])
}

fn read_chunk_with_pending<S: RealmStore>(
    store: &S,
    root: Option<&BlobDigest>,
    chunk_index: u64,
    pending: &[StagedBlob],
) -> Result<Vec<u8>, FsError> {
    if chunk_index >= CHUNK_TREE_FANOUT.pow(u32::from(CHUNK_TREE_LEVELS)) {
        return Err(FsError::TooLarge);
    }
    let mut current = root.cloned();
    for level in (0..CHUNK_TREE_LEVELS).rev() {
        let Some(digest) = current else {
            return Ok(vec![0; FILE_CHUNK_BYTES]);
        };
        let node = load_chunk_node(store, &digest, level, pending)?;
        current = node.children.get(&chunk_digit(chunk_index, level)).cloned();
    }
    let Some(digest) = current else {
        return Ok(vec![0; FILE_CHUNK_BYTES]);
    };
    let bytes = load_verified_blob(store, &digest, "file chunk", pending)?;
    if bytes.len() != FILE_CHUNK_BYTES {
        return Err(FsError::Corrupt(format!(
            "file chunk {} has non-canonical length",
            digest.as_str()
        )));
    }
    Ok(bytes)
}

fn update_chunk<S: RealmStore>(
    store: &S,
    root: Option<&BlobDigest>,
    chunk_index: u64,
    chunk: &[u8],
    blobs: &mut StagedBlobs,
) -> Result<Option<BlobDigest>, FsError> {
    if chunk.len() != FILE_CHUNK_BYTES
        || chunk_index >= CHUNK_TREE_FANOUT.pow(u32::from(CHUNK_TREE_LEVELS))
    {
        return Err(FsError::TooLarge);
    }

    let mut nodes = Vec::with_capacity(usize::from(CHUNK_TREE_LEVELS));
    let mut current = root.cloned();
    for level in (0..CHUNK_TREE_LEVELS).rev() {
        let node = match &current {
            Some(digest) => load_chunk_node(store, digest, level, blobs)?,
            None => ChunkNode {
                level,
                children: BTreeMap::new(),
            },
        };
        current = node.children.get(&chunk_digit(chunk_index, level)).cloned();
        nodes.push(node);
    }

    let mut child = if chunk.iter().all(|byte| *byte == 0) {
        None
    } else {
        Some(stage_blob(chunk.to_vec(), blobs))
    };
    for mut node in nodes.into_iter().rev() {
        let digit = chunk_digit(chunk_index, node.level);
        if let Some(digest) = child {
            node.children.insert(digit, digest);
        } else {
            node.children.remove(&digit);
        }
        child = if node.children.is_empty() {
            None
        } else {
            Some(stage_blob(encode(&node)?, blobs))
        };
    }
    Ok(child)
}

fn prune_tree<S: RealmStore>(
    store: &S,
    root: Option<&BlobDigest>,
    retained_chunks: u64,
    blobs: &mut StagedBlobs,
) -> Result<Option<BlobDigest>, FsError> {
    let Some(root) = root else {
        return Ok(None);
    };
    prune_node(
        store,
        root,
        CHUNK_TREE_LEVELS - 1,
        0,
        retained_chunks,
        blobs,
    )
}

fn prune_node<S: RealmStore>(
    store: &S,
    digest: &BlobDigest,
    level: u8,
    first_chunk: u64,
    retained_chunks: u64,
    blobs: &mut StagedBlobs,
) -> Result<Option<BlobDigest>, FsError> {
    let mut node = load_chunk_node(store, digest, level, blobs)?;
    let span = CHUNK_TREE_FANOUT.pow(u32::from(level));
    let children: Vec<_> = node.children.keys().copied().collect();
    for digit in children {
        let child_start = first_chunk + u64::from(digit) * span;
        if child_start >= retained_chunks {
            node.children.remove(&digit);
        } else if level > 0 && child_start + span > retained_chunks {
            let child =
                node.children.get(&digit).cloned().ok_or_else(|| {
                    FsError::Corrupt("chunk node changed during pruning".to_string())
                })?;
            match prune_node(
                store,
                &child,
                level - 1,
                child_start,
                retained_chunks,
                blobs,
            )? {
                Some(replacement) => {
                    node.children.insert(digit, replacement);
                }
                None => {
                    node.children.remove(&digit);
                }
            }
        }
    }
    if node.children.is_empty() {
        Ok(None)
    } else {
        Ok(Some(stage_blob(encode(&node)?, blobs)))
    }
}

fn load_chunk_node<S: RealmStore>(
    store: &S,
    digest: &BlobDigest,
    expected_level: u8,
    pending: &[StagedBlob],
) -> Result<ChunkNode, FsError> {
    let bytes = load_verified_blob(store, digest, "chunk node", pending)?;
    if bytes.len() > MAX_MANIFEST_BYTES {
        return Err(FsError::Corrupt("chunk node is oversized".to_string()));
    }
    let node: ChunkNode = decode(&bytes)?;
    if node.level != expected_level || node.children.is_empty() {
        return Err(FsError::Corrupt(format!(
            "chunk node {} is non-canonical",
            digest.as_str()
        )));
    }
    Ok(node)
}

fn load_verified_blob<S: RealmStore>(
    store: &S,
    digest: &BlobDigest,
    kind: &str,
    pending: &[StagedBlob],
) -> Result<Vec<u8>, FsError> {
    let bytes = if let Some((_, bytes)) = pending
        .iter()
        .rev()
        .find(|(candidate, _)| candidate == digest)
    {
        bytes.clone()
    } else {
        store
            .get_blob(digest)?
            .ok_or_else(|| FsError::Corrupt(format!("missing {kind} {}", digest.as_str())))?
    };
    verify_blob(digest, &bytes)?;
    Ok(bytes)
}

fn stage_blob(bytes: Vec<u8>, blobs: &mut StagedBlobs) -> BlobDigest {
    let digest = BlobDigest::for_bytes(&bytes);
    blobs.push((digest.clone(), bytes));
    digest
}

fn chunk_digit(chunk_index: u64, level: u8) -> u8 {
    ((chunk_index >> (u32::from(level) * 8)) & 0xff) as u8
}

fn metadata_in(manifest: &Manifest, path: &str) -> Result<FsMetadata, FsError> {
    if path.is_empty() {
        return Ok(FsMetadata {
            kind: FsNodeKind::Directory,
            bytes: 0,
            mode: ROOT_DIRECTORY_MODE,
            generation: manifest.generation,
        });
    }
    if let Some(record) = manifest.files.get(path) {
        return Ok(FsMetadata {
            kind: FsNodeKind::File,
            bytes: record.bytes,
            mode: record.mode,
            generation: manifest.generation,
        });
    }
    if let Some(record) = manifest.directories.get(path) {
        return Ok(FsMetadata {
            kind: FsNodeKind::Directory,
            bytes: 0,
            mode: record.mode,
            generation: manifest.generation,
        });
    }
    Err(FsError::NotFound)
}

fn node_kind(manifest: &Manifest, path: &str) -> Option<FsNodeKind> {
    if path.is_empty() || manifest.directories.contains_key(path) {
        Some(FsNodeKind::Directory)
    } else if manifest.files.contains_key(path) {
        Some(FsNodeKind::File)
    } else {
        None
    }
}

fn node_exists(manifest: &Manifest, path: &str) -> bool {
    node_kind(manifest, path).is_some()
}

fn parent_path(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

fn require_parent_directory(manifest: &Manifest, path: &str) -> Result<(), FsError> {
    match node_kind(manifest, parent_path(path)) {
        Some(FsNodeKind::Directory) => Ok(()),
        Some(FsNodeKind::File) | None => Err(FsError::NotDirectory),
    }
}

fn ensure_parent_directories(manifest: &mut Manifest, path: &str) -> Result<(), FsError> {
    let Some((parent, _)) = path.rsplit_once('/') else {
        return Ok(());
    };
    let mut current = String::new();
    for component in parent.split('/') {
        if !current.is_empty() {
            current.push('/');
        }
        current.push_str(component);
        if manifest.files.contains_key(&current) {
            return Err(FsError::NotDirectory);
        }
        manifest
            .directories
            .entry(current.clone())
            .or_insert(DirectoryRecord {
                mode: DEFAULT_DIRECTORY_MODE,
            });
    }
    Ok(())
}

fn materialize_legacy_directories(manifest: &mut Manifest) -> Result<(), FsError> {
    let paths: Vec<_> = manifest.files.keys().cloned().collect();
    for path in paths {
        validate_relative_path(&path).map_err(|_| {
            FsError::Corrupt(format!("legacy manifest contains invalid path {path:?}"))
        })?;
        ensure_parent_directories(manifest, &path)?;
    }
    Ok(())
}

fn immediate_child<'a>(parent: &str, candidate: &'a str) -> Option<&'a str> {
    let relative = if parent.is_empty() {
        candidate
    } else {
        candidate.strip_prefix(parent)?.strip_prefix('/')?
    };
    if relative.is_empty() {
        return None;
    }
    Some(relative.split('/').next().unwrap_or(relative))
}

fn has_descendant(manifest: &Manifest, path: &str) -> bool {
    let prefix = format!("{path}/");
    manifest
        .files
        .keys()
        .chain(manifest.directories.keys())
        .any(|candidate| candidate.starts_with(&prefix))
}

fn remove_node(manifest: &mut Manifest, path: &str) {
    manifest.files.remove(path);
    manifest.directories.remove(path);
}

fn rename_node(manifest: &mut Manifest, source: &str, destination: &str, source_kind: FsNodeKind) {
    match source_kind {
        FsNodeKind::File => {
            if let Some(record) = manifest.files.remove(source) {
                manifest.files.insert(destination.to_string(), record);
            }
        }
        FsNodeKind::Directory => {
            let prefix = format!("{source}/");
            let directories: Vec<_> = manifest
                .directories
                .iter()
                .filter(|(path, _)| path.as_str() == source || path.starts_with(&prefix))
                .map(|(path, record)| (path.clone(), record.clone()))
                .collect();
            let files: Vec<_> = manifest
                .files
                .iter()
                .filter(|(path, _)| path.starts_with(&prefix))
                .map(|(path, record)| (path.clone(), record.clone()))
                .collect();
            for (path, _) in &directories {
                manifest.directories.remove(path);
            }
            for (path, _) in &files {
                manifest.files.remove(path);
            }
            for (path, record) in directories {
                let suffix = path.strip_prefix(source).unwrap_or_default();
                manifest
                    .directories
                    .insert(format!("{destination}{suffix}"), record);
            }
            for (path, record) in files {
                let suffix = path.strip_prefix(source).unwrap_or_default();
                manifest
                    .files
                    .insert(format!("{destination}{suffix}"), record);
            }
        }
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

        fn stored_blob_bytes(&self) -> usize {
            self.inner.borrow().blobs.values().map(Vec::len).sum()
        }

        fn stage_competing_file(&self, path: &str, bytes: &[u8]) -> BlobDigest {
            let file_blob = BlobDigest::for_bytes(bytes);
            let mut files = BTreeMap::new();
            files.insert(
                path.to_string(),
                FileRecord {
                    blob: Some(file_blob.clone()),
                    tree: None,
                    bytes: u64::try_from(bytes.len()).expect("test file length fits"),
                    mode: DEFAULT_FILE_MODE,
                },
            );
            let manifest = Manifest {
                format: FORMAT_VERSION,
                generation: 1,
                parent_manifest: None,
                files,
                directories: BTreeMap::new(),
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
    fn multi_megabyte_files_are_chunked_and_support_bounded_reads() {
        let store = MemoryStore::default();
        let mut filesystem = RealmFs::new(store);
        let bytes: Vec<_> = (0..(4 * 1024 * 1024 + 137))
            .map(|index| (index % 251) as u8)
            .collect();

        filesystem
            .write_file("target/debug/app", &bytes)
            .expect("large file commits");

        assert_eq!(
            filesystem.read_at("target/debug/app", 65_500, 80_000),
            Ok(bytes[65_500..145_500].to_vec())
        );
        assert_eq!(
            filesystem.read_at(
                "target/debug/app",
                u64::try_from(bytes.len() - 29).expect("length fits"),
                200,
            ),
            Ok(bytes[bytes.len() - 29..].to_vec())
        );
        assert_eq!(filesystem.read_file("target/debug/app"), Ok(bytes));
    }

    #[test]
    fn one_byte_edit_materializes_one_chunk_path_not_the_whole_file() {
        let store = MemoryStore::default();
        let mut filesystem = RealmFs::new(store.clone());
        let bytes: Vec<_> = (0..(4 * 1024 * 1024))
            .map(|index| (index % 251) as u8)
            .collect();
        filesystem
            .write_file("compiler-output", &bytes)
            .expect("large file commits");
        let before = store.stored_blob_bytes();

        filesystem
            .write_at("compiler-output", 2 * 1024 * 1024 + 17, &[0xff])
            .expect("single-byte patch commits");

        let added = store.stored_blob_bytes() - before;
        assert!(
            added < FILE_CHUNK_BYTES + 32 * 1024,
            "single-byte edit stored {added} new bytes"
        );
        assert_eq!(
            filesystem.read_at("compiler-output", 2 * 1024 * 1024 + 16, 3),
            Ok(vec![63, 0xff, 65])
        );
    }

    #[test]
    fn sparse_growth_does_not_materialize_zero_ranges() {
        let store = MemoryStore::default();
        let mut filesystem = RealmFs::new(store.clone());
        filesystem
            .create_file("sparse", 0o600, true, false)
            .expect("file creates");
        let before = store.stored_blob_bytes();

        filesystem
            .set_len("sparse", 512 * 1024 * 1024)
            .expect("sparse extension commits");
        filesystem
            .write_at("sparse", 512 * 1024 * 1024 - 3, b"end")
            .expect("tail write commits");

        assert_eq!(filesystem.read_at("sparse", 0, 32), Ok(vec![0; 32]));
        assert_eq!(
            filesystem.read_at("sparse", 512 * 1024 * 1024 - 5, 5),
            Ok(b"\0\0end".to_vec())
        );
        assert!(
            store.stored_blob_bytes() - before < FILE_CHUNK_BYTES + 64 * 1024,
            "sparse extension unexpectedly materialized its holes"
        );
    }

    #[test]
    fn truncate_then_extend_never_reveals_discarded_bytes() {
        let store = MemoryStore::default();
        let mut filesystem = RealmFs::new(store);
        let original = vec![0xa5; FILE_CHUNK_BYTES * 3];
        filesystem
            .write_file("state", &original)
            .expect("initial content commits");

        filesystem
            .set_len("state", FILE_CHUNK_BYTES as u64 + 13)
            .expect("truncate commits");
        filesystem
            .set_len("state", (FILE_CHUNK_BYTES * 3) as u64)
            .expect("extension commits");

        assert_eq!(
            filesystem.read_at("state", FILE_CHUNK_BYTES as u64, 32),
            Ok([vec![0xa5; 13], vec![0; 19]].concat())
        );
        assert_eq!(
            filesystem.read_at("state", (FILE_CHUNK_BYTES * 2) as u64, 32),
            Ok(vec![0; 32])
        );
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
        store.replace_blob(
            receipt.content_root.expect("non-empty file has a tree"),
            b"tampered".to_vec(),
        );

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
            filesystem.set_len("large", MAX_LOGICAL_FILE_BYTES + 1),
            Err(FsError::TooLarge)
        );
        assert!(store.read_head().expect("head reads").is_none());
    }

    #[test]
    fn directory_and_positional_mutations_are_generation_atomic() {
        let store = MemoryStore::default();
        let mut filesystem = RealmFs::new(store);

        filesystem
            .create_dir("projects", 0o750)
            .expect("directory commits");
        filesystem
            .create_file("projects/main.rs", 0o640, true, false)
            .expect("file commits");
        assert_eq!(
            filesystem.write_at("projects/main.rs", 0, b"fn main"),
            Ok(7)
        );
        assert_eq!(filesystem.write_at("projects/main.rs", 10, b"{}"), Ok(2));
        assert_eq!(
            filesystem.read_file("projects/main.rs"),
            Ok(b"fn main\0\0\0{}".to_vec())
        );
        filesystem
            .set_len("projects/main.rs", 9)
            .expect("truncate commits");

        let before_failed_remove = filesystem.status().expect("status").generation;
        assert_eq!(filesystem.remove_dir("projects"), Err(FsError::NotEmpty));
        assert_eq!(
            filesystem.status().expect("status").generation,
            before_failed_remove
        );

        filesystem
            .rename("projects", "archive")
            .expect("tree rename commits");
        assert_eq!(
            filesystem.read_file("archive/main.rs"),
            Ok(b"fn main\0\0".to_vec())
        );
        assert_eq!(
            filesystem.metadata("archive").expect("directory metadata"),
            FsMetadata {
                kind: FsNodeKind::Directory,
                bytes: 0,
                mode: 0o750,
                generation: filesystem.status().expect("status").generation,
            }
        );
        assert_eq!(
            filesystem
                .read_dir("")
                .expect("root directory")
                .into_iter()
                .map(|entry| entry.name)
                .collect::<Vec<_>>(),
            vec!["archive"]
        );

        filesystem
            .remove_file("archive/main.rs")
            .expect("file removal commits");
        filesystem
            .remove_dir("archive")
            .expect("empty directory removal commits");
        assert_eq!(filesystem.metadata("archive"), Err(FsError::NotFound));
    }

    #[test]
    fn mode_changes_are_generation_atomic_and_preserve_content_and_kind() {
        let store = MemoryStore::default();
        let mut filesystem = RealmFs::new(store);
        filesystem
            .create_dir("bin", 0o750)
            .expect("directory commits");
        filesystem
            .create_file("bin/tool", 0o640, true, false)
            .expect("file commits");
        filesystem
            .write_at("bin/tool", 0, b"executable")
            .expect("content commits");
        let before = filesystem.status().expect("status").generation;

        filesystem.set_mode("bin/tool", 0o755).expect("chmod file");
        assert!(filesystem.status().expect("status").generation > before);
        assert_eq!(
            filesystem.metadata("bin/tool").expect("file metadata").mode,
            0o755
        );
        assert_eq!(filesystem.read_file("bin/tool"), Ok(b"executable".to_vec()));

        filesystem.set_mode("bin", 0o700).expect("chmod directory");
        let directory = filesystem.metadata("bin").expect("directory metadata");
        assert_eq!(directory.kind, FsNodeKind::Directory);
        assert_eq!(directory.mode, 0o700);
        assert_eq!(
            filesystem.set_mode("missing", 0o755),
            Err(FsError::NotFound)
        );
    }

    #[test]
    fn legacy_file_manifests_gain_directories_on_the_next_mutation() {
        let store = MemoryStore::default();
        let file_bytes = b"legacy".to_vec();
        let file_blob = BlobDigest::for_bytes(&file_bytes);
        let manifest_bytes = serde_json::to_vec(&serde_json::json!({
            "format": LEGACY_FORMAT_VERSION,
            "generation": 1,
            "parent_manifest": null,
            "files": {
                "state/session.json": {
                    "blob": file_blob.as_str(),
                    "bytes": file_bytes.len()
                }
            }
        }))
        .expect("legacy manifest encodes");
        let manifest_digest = BlobDigest::for_bytes(&manifest_bytes);
        {
            let mut state = store.inner.borrow_mut();
            state.blobs.insert(file_blob, file_bytes);
            state.blobs.insert(manifest_digest.clone(), manifest_bytes);
            state.head = Some(
                encode(&HeadRecord {
                    format: LEGACY_FORMAT_VERSION,
                    generation: 1,
                    manifest: manifest_digest,
                })
                .expect("legacy head encodes"),
            );
        }
        let mut filesystem = RealmFs::new(store);

        assert_eq!(
            filesystem
                .metadata("state")
                .expect("implicit directory")
                .kind,
            FsNodeKind::Directory
        );
        assert_eq!(filesystem.status().expect("legacy status").format, 1);
        filesystem
            .write_at("state/session.json", 6, b"-migrated")
            .expect("mutation upgrades the manifest");
        assert_eq!(
            filesystem.status().expect("current status").format,
            FORMAT_VERSION
        );
        assert_eq!(
            filesystem.read_file("state/session.json"),
            Ok(b"legacy-migrated".to_vec())
        );
        assert_eq!(
            filesystem
                .read_dir("")
                .expect("root directory")
                .first()
                .map(|entry| entry.name.as_str()),
            Some("state")
        );
    }

    #[test]
    fn format_two_whole_files_read_then_upgrade_lazily_on_mutation() {
        let store = MemoryStore::default();
        let file_bytes = b"deployed-format-two".to_vec();
        let file_blob = BlobDigest::for_bytes(&file_bytes);
        let manifest = Manifest {
            format: WHOLE_BLOB_FORMAT_VERSION,
            generation: 1,
            parent_manifest: None,
            files: BTreeMap::from([(
                "state".to_string(),
                FileRecord {
                    blob: Some(file_blob.clone()),
                    tree: None,
                    bytes: u64::try_from(file_bytes.len()).expect("test length fits"),
                    mode: DEFAULT_FILE_MODE,
                },
            )]),
            directories: BTreeMap::new(),
        };
        let manifest_bytes = encode(&manifest).expect("format-two manifest encodes");
        let manifest_digest = BlobDigest::for_bytes(&manifest_bytes);
        {
            let mut state = store.inner.borrow_mut();
            state.blobs.insert(file_blob, file_bytes);
            state.blobs.insert(manifest_digest.clone(), manifest_bytes);
            state.head = Some(
                encode(&HeadRecord {
                    format: WHOLE_BLOB_FORMAT_VERSION,
                    generation: 1,
                    manifest: manifest_digest,
                })
                .expect("format-two head encodes"),
            );
        }
        let mut filesystem = RealmFs::new(store.clone());

        assert_eq!(
            filesystem.read_file("state"),
            Ok(b"deployed-format-two".to_vec())
        );
        assert_eq!(
            filesystem.status().expect("old format reads").format,
            WHOLE_BLOB_FORMAT_VERSION
        );
        filesystem
            .write_at("state", 9, b"three")
            .expect("first content mutation upgrades");

        let snapshot = filesystem.load_snapshot().expect("new snapshot loads");
        let record = snapshot.manifest.files.get("state").expect("file remains");
        assert_eq!(snapshot.manifest.format, FORMAT_VERSION);
        assert!(record.blob.is_none());
        assert!(record.tree.is_some());
        assert_eq!(
            filesystem.read_file("state"),
            Ok(b"deployed-threet-two".to_vec())
        );
    }

    #[test]
    fn newer_or_malformed_metadata_is_never_interpreted() {
        let store = MemoryStore::default();
        store.inner.borrow_mut().head = Some(
            encode(&HeadRecord {
                format: FORMAT_VERSION + 1,
                generation: 1,
                manifest: BlobDigest::parse("a".repeat(64)).expect("digest"),
            })
            .expect("future head encodes"),
        );
        let filesystem = RealmFs::new(store);

        assert!(matches!(filesystem.status(), Err(FsError::Corrupt(_))));
        assert!(BlobDigest::parse("A".repeat(64)).is_err());
        assert!(matches!(
            decode::<HeadRecord>(
                br#"{"format":2,"generation":1,"manifest":"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa","extra":true}"#
            ),
            Err(FsError::Serialization(_))
        ));
        assert!(matches!(
            decode::<Manifest>(
                br#"{"format":2,"generation":1,"parent_manifest":null,"files":{},"directories":{},"extra":true}"#
            ),
            Err(FsError::Serialization(_))
        ));
    }

    #[test]
    fn structurally_invalid_selected_manifests_fail_closed() {
        for manifest in [
            Manifest {
                format: FORMAT_VERSION,
                generation: 1,
                parent_manifest: None,
                files: BTreeMap::from([(
                    "missing/parent".to_string(),
                    FileRecord {
                        blob: Some(BlobDigest::for_bytes(b"x")),
                        tree: None,
                        bytes: 1,
                        mode: DEFAULT_FILE_MODE,
                    },
                )]),
                directories: BTreeMap::new(),
            },
            Manifest {
                format: FORMAT_VERSION,
                generation: 2,
                parent_manifest: None,
                files: BTreeMap::new(),
                directories: BTreeMap::new(),
            },
        ] {
            let store = MemoryStore::default();
            let bytes = encode(&manifest).expect("invalid manifest still encodes");
            let digest = BlobDigest::for_bytes(&bytes);
            {
                let mut state = store.inner.borrow_mut();
                state.blobs.insert(digest.clone(), bytes);
                state.head = Some(
                    encode(&HeadRecord {
                        format: manifest.format,
                        generation: manifest.generation,
                        manifest: digest,
                    })
                    .expect("head encodes"),
                );
            }

            assert!(matches!(
                RealmFs::new(store).status(),
                Err(FsError::Corrupt(_))
            ));
        }
    }
}
