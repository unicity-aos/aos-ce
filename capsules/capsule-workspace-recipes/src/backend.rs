use crate::documents::parse_canonical as parse_canonical_document;
use crate::documents::{
    ActivityPointer, ConflictRecord, CurrentPointer, JournalRecord, RevisionEnvelope,
};
use crate::service::CommitStage;
use capsule_surface_model::canonical::{canonical_bytes, digest_parts};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum BackendError {
    CasMismatch,
    HashCollision,
    CorruptState,
    Crash(CommitStage),
}

#[derive(Clone, Debug, Default)]
pub struct FixtureBackend {
    objects: BTreeMap<String, Vec<u8>>,
    revisions: BTreeMap<String, Vec<u8>>,
    pointers: BTreeMap<String, CurrentPointer>,
    journals: BTreeMap<String, JournalRecord>,
    activity_objects: BTreeMap<String, Vec<u8>>,
    activity_pointers: BTreeMap<String, ActivityPointer>,
    conflicts: BTreeMap<String, ConflictRecord>,
    quarantine: BTreeMap<String, Vec<u8>>,
    fail_after: Option<CommitStage>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RecipeLocation {
    pub scope: String,
    pub recipe_id: String,
}

impl RecipeLocation {
    fn revision_key(&self, revision: u64) -> String {
        format!("revision/{}/{}/{}", self.scope, self.recipe_id, revision)
    }

    fn pointer_key(&self) -> String {
        format!("pointer/{}/{}/current", self.scope, self.recipe_id)
    }

    fn journal_key(&self, patch_id: &str) -> String {
        format!("journal/{}/{}/{}", self.scope, self.recipe_id, patch_id)
    }

    fn conflict_key(&self, incoming_patch_id: &str) -> String {
        format!(
            "conflict/{}/{}/{}",
            self.scope, self.recipe_id, incoming_patch_id
        )
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct CommittedPointer {
    pub location: RecipeLocation,
    pub pointer: CurrentPointer,
}

pub(crate) fn scope_key(owner_json: &[u8]) -> String {
    digest_parts(&owner_json).expect("canonical owner digest")
}

fn referenced_revision_keys(key: &str, referenced: &BTreeSet<(String, String, u64)>) -> bool {
    let Some(remainder) = key.strip_prefix("revision/") else {
        return false;
    };
    let Some((scope, tail)) = remainder.split_once('/') else {
        return false;
    };
    let Some((recipe_id, revision)) = tail.rsplit_once('/') else {
        return false;
    };
    let Ok(revision) = revision.parse::<u64>() else {
        return false;
    };
    referenced.contains(&(scope.to_owned(), recipe_id.to_owned(), revision))
}

impl FixtureBackend {
    pub(crate) fn clear_fault(&mut self) {
        self.fail_after = None;
    }

    pub fn set_fail_after(&mut self, stage: CommitStage) {
        self.fail_after = Some(stage);
    }

    fn crash_if_requested(&mut self, completed: CommitStage) -> Result<(), BackendError> {
        if self.fail_after == Some(completed) {
            self.fail_after = None;
            return Err(BackendError::Crash(completed));
        }
        Ok(())
    }

    pub(crate) fn put_object_once(
        &mut self,
        _location: &RecipeLocation,
        recipe_digest: &str,
        recipe_bytes: &[u8],
    ) -> Result<(), BackendError> {
        let key = format!("object/{recipe_digest}");
        if let Some(existing) = self.objects.get(&key) {
            if existing != recipe_bytes {
                return Err(BackendError::HashCollision);
            }
        } else {
            self.objects.insert(key, recipe_bytes.to_vec());
        }
        self.crash_if_requested(CommitStage::Object)
    }

    pub(crate) fn put_revision_once(
        &mut self,
        location: &RecipeLocation,
        revision: u64,
        envelope: &RevisionEnvelope,
    ) -> Result<(), BackendError> {
        let key = location.revision_key(revision);
        let bytes = canonical_bytes(envelope).map_err(|_| BackendError::CorruptState)?;
        if let Some(existing) = self.revisions.get(&key) {
            if existing != &bytes {
                return Err(BackendError::HashCollision);
            }
        } else {
            self.revisions.insert(key, bytes);
        }
        self.crash_if_requested(CommitStage::Revision)
    }

    pub(crate) fn cas_current(
        &mut self,
        location: &RecipeLocation,
        expected: Option<&CurrentPointer>,
        next: CurrentPointer,
    ) -> Result<(), BackendError> {
        let key = location.pointer_key();
        if self.pointers.get(&key) != expected {
            return Err(BackendError::CasMismatch);
        }
        self.pointers.insert(key, next);
        self.crash_if_requested(CommitStage::Pointer)
    }

    pub(crate) fn put_journal_once(
        &mut self,
        location: &RecipeLocation,
        record: JournalRecord,
    ) -> Result<(), BackendError> {
        let key = location.journal_key(&record.patch_id);
        if let Some(existing) = self.journals.get(&key) {
            if existing != &record {
                return Err(BackendError::HashCollision);
            }
            return Ok(());
        }
        self.journals.insert(key, record);
        self.crash_if_requested(CommitStage::Journal)
    }

    pub(crate) fn retain_conflict(&mut self, location: &RecipeLocation, record: ConflictRecord) {
        let key = location.conflict_key(&record.incoming_patch_id);
        self.conflicts.insert(key, record);
    }

    pub(crate) fn conflict(
        &self,
        location: &RecipeLocation,
        incoming_patch_id: &str,
    ) -> Option<&ConflictRecord> {
        self.conflicts
            .get(&location.conflict_key(incoming_patch_id))
    }

    pub(crate) fn current(
        &self,
        location: &RecipeLocation,
    ) -> Result<Option<CurrentPointer>, BackendError> {
        self.pointers
            .get(&location.pointer_key())
            .cloned()
            .map_or_else(
                || Ok(None),
                |pointer| {
                    let bytes = self
                        .revisions
                        .get(&location.revision_key(pointer.revision))
                        .ok_or(BackendError::CorruptState)?;
                    let envelope: RevisionEnvelope =
                        parse_canonical_document(bytes).map_err(|_| BackendError::CorruptState)?;
                    if envelope.recipe.digest != pointer.recipe_digest {
                        return Err(BackendError::CorruptState);
                    }
                    let object = self
                        .objects
                        .get(&format!("object/{}", envelope.recipe.digest))
                        .ok_or(BackendError::CorruptState)?;
                    let canonical = canonical_bytes(&envelope.recipe)
                        .map_err(|_| BackendError::CorruptState)?;
                    if object != &canonical {
                        return Err(BackendError::CorruptState);
                    }
                    Ok(Some(pointer))
                },
            )
    }

    pub(crate) fn quarantine_orphans(
        &mut self,
        referenced_objects: &BTreeSet<String>,
        referenced_revisions: &BTreeSet<(String, String, u64)>,
    ) {
        let mut keys: Vec<String> = self
            .objects
            .keys()
            .filter(|key| !referenced_objects.contains(*key))
            .cloned()
            .collect();
        keys.extend(
            self.revisions
                .keys()
                .filter(|key| !referenced_revision_keys(key, referenced_revisions))
                .cloned(),
        );
        for key in keys {
            if let Some(value) = self
                .objects
                .remove(&key)
                .or_else(|| self.revisions.remove(&key))
            {
                self.quarantine.insert(key, value);
            }
        }
    }

    pub fn quarantine_len(&self) -> usize {
        self.quarantine.len()
    }

    pub(crate) fn committed_pointers(&self) -> Vec<CommittedPointer> {
        self.pointers
            .iter()
            .map(|(key, pointer)| {
                let remainder = key
                    .strip_prefix("pointer/")
                    .and_then(|value| value.strip_suffix("/current"))
                    .ok_or(BackendError::CorruptState)
                    .expect("backend pointer key");
                let (scope, recipe_id) = remainder
                    .split_once('/')
                    .ok_or(BackendError::CorruptState)
                    .expect("backend pointer location");
                CommittedPointer {
                    location: RecipeLocation {
                        scope: scope.to_owned(),
                        recipe_id: recipe_id.to_owned(),
                    },
                    pointer: pointer.clone(),
                }
            })
            .collect()
    }

    pub(crate) fn revision_envelope(
        &self,
        scope: &str,
        recipe_id: &str,
        revision: u64,
    ) -> Result<RevisionEnvelope, BackendError> {
        let location = RecipeLocation {
            scope: scope.to_owned(),
            recipe_id: recipe_id.to_owned(),
        };
        let bytes = self
            .revisions
            .get(&location.revision_key(revision))
            .ok_or(BackendError::CorruptState)?;
        parse_canonical_document(bytes).map_err(|_| BackendError::CorruptState)
    }

    pub(crate) fn journal_for_recipe(
        &self,
        scope: &str,
        recipe_id: &str,
        patch_id: &str,
    ) -> Option<&JournalRecord> {
        self.journals
            .get(&format!("journal/{scope}/{recipe_id}/{patch_id}"))
    }

    pub(crate) fn insert_recovered_journal(
        &mut self,
        scope: &str,
        recipe_id: &str,
        record: JournalRecord,
    ) {
        let location = RecipeLocation {
            scope: scope.to_owned(),
            recipe_id: recipe_id.to_owned(),
        };
        let key = location.journal_key(&record.patch_id);
        self.journals.entry(key).or_insert(record);
    }

    pub(crate) fn save_activity(
        &mut self,
        scope: &str,
        activity_id: &str,
        activity_digest: &str,
        activity_bytes: &[u8],
        expected: Option<&ActivityPointer>,
    ) -> Result<(), BackendError> {
        let object_key = format!("activity/{activity_digest}");
        if let Some(existing) = self.activity_objects.get(&object_key) {
            if existing != activity_bytes {
                return Err(BackendError::HashCollision);
            }
        } else {
            self.activity_objects
                .insert(object_key, activity_bytes.to_vec());
        }
        let pointer_key = format!("activity-pointer/{scope}/{activity_id}");
        if self.activity_pointers.get(&pointer_key) != expected {
            return Err(BackendError::CasMismatch);
        }
        self.activity_pointers.insert(
            pointer_key,
            ActivityPointer {
                activity_digest: activity_digest.to_owned(),
            },
        );
        Ok(())
    }

    pub(crate) fn activity_pointer(
        &self,
        scope: &str,
        activity_id: &str,
    ) -> Option<ActivityPointer> {
        self.activity_pointers
            .get(&format!("activity-pointer/{scope}/{activity_id}"))
            .cloned()
    }

    pub(crate) fn activity_bytes(
        &self,
        pointer: &ActivityPointer,
    ) -> Result<Vec<u8>, BackendError> {
        self.activity_objects
            .get(&format!("activity/{}", pointer.activity_digest))
            .cloned()
            .ok_or(BackendError::CorruptState)
    }

    pub(crate) fn object_matches(&self, recipe_digest: &str, canonical_bytes: &[u8]) -> bool {
        self.objects
            .get(&format!("object/{recipe_digest}"))
            .is_some_and(|stored| stored == canonical_bytes)
    }
}
