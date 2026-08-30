//! In-memory index backend.
//!
//! This backend is intended for tests, short-lived daemon state, and as a
//! baseline for disk-backed implementations. It provides O(log n) lookup by
//! artifact id via a [`BTreeMap`].

use std::collections::BTreeMap;

use boar_core::ArtifactId;

use crate::record::{IndexDelta, IndexRecord};

/// In-memory index backend.
#[derive(Debug, Default, Clone)]
pub struct InMemoryBackend {
    records: BTreeMap<String, IndexRecord>,
    generation: u64,
}

impl InMemoryBackend {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn get(&self, id: &ArtifactId) -> Option<IndexRecord> {
        self.records.get(&id.0).cloned()
    }

    pub fn put(&mut self, record: IndexRecord) {
        self.records.insert(record.artifact_id.0.clone(), record);
    }

    pub fn put_batch(&mut self, records: &[IndexRecord]) {
        for record in records {
            self.put(record.clone());
        }
    }

    pub fn delete(&mut self, id: &ArtifactId) -> bool {
        self.records.remove(&id.0).is_some()
    }

    pub fn list(&self) -> Vec<IndexRecord> {
        self.records.values().cloned().collect()
    }

    pub fn apply_delta(&mut self, delta: IndexDelta) {
        match delta {
            IndexDelta::Upsert(record) => self.put(*record),
            IndexDelta::Delete(id) => {
                self.delete(&id);
            }
            IndexDelta::MarkUnavailable(id) => {
                if let Some(record) = self.records.get_mut(&id.0) {
                    record.availability = boar_core::Availability::Unavailable;
                }
            }
            IndexDelta::MarkVerified(id) => {
                if let Some(record) = self.records.get_mut(&id.0) {
                    record.verification_state = boar_core::VerificationState::Verified;
                }
            }
        }
    }

    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn bump_generation(&mut self) {
        self.generation += 1;
    }

    pub fn set_generation(&mut self, generation: u64) {
        self.generation = generation;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let mut backend = InMemoryBackend::new();
        let id = ArtifactId("abc".into());
        let record = IndexRecord::new(id.clone());
        backend.put(record.clone());
        assert_eq!(backend.get(&id), Some(record));
    }

    #[test]
    fn delete_removes_record() {
        let mut backend = InMemoryBackend::new();
        let id = ArtifactId("abc".into());
        backend.put(IndexRecord::new(id.clone()));
        assert!(backend.delete(&id));
        assert!(backend.get(&id).is_none());
    }

    #[test]
    fn delta_mark_unavailable() {
        let mut backend = InMemoryBackend::new();
        let id = ArtifactId("abc".into());
        backend.put(IndexRecord::new(id.clone()));
        backend.apply_delta(IndexDelta::MarkUnavailable(id.clone()));
        let record = backend.get(&id).unwrap();
        assert_eq!(record.availability, boar_core::Availability::Unavailable);
    }
}
