//! Local artifact index for the Baby/Boar/Boarish ecosystem.
//!
//! The index answers the hot-path question:
//!
//! > "Does a valid artifact exist for this identity, and where can I get it?"
//!
//! It is intentionally separate from the slower ontology store (Padagonia) and
//! from the artifact fabric itself (Boaring/Boar). The index is a
//! materialised operational view that is kept up to date asynchronously.
//!
//! Two backends are provided:
//!
//! - [`BoarIndex::open_in_memory`] — for tests and ephemeral state.
//! - [`BoarIndex::open_sqlite`] — persistent local index.

use std::path::Path;

use boar_core::ArtifactId;
pub use boar_core::{Availability, FeatureSet, StorageLocation, VerificationState};

use crate::memory::InMemoryBackend;
pub use crate::record::{IndexDelta, IndexRecord};
use crate::sqlite::SqliteBackend;

mod memory;
mod record;
mod sqlite;
pub mod sync;

/// Which storage backend to use for the index.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BackendKind {
    InMemory,
    Sqlite,
}

/// A local artifact index.
#[derive(Debug)]
pub struct BoarIndex {
    inner: Inner,
}

#[derive(Debug)]
enum Inner {
    Memory(InMemoryBackend),
    Sqlite(SqliteBackend),
}

impl BoarIndex {
    /// Open an in-memory index. Data is lost when the index is dropped.
    pub fn open_in_memory() -> Self {
        Self {
            inner: Inner::Memory(InMemoryBackend::new()),
        }
    }

    /// Open or create a SQLite-backed index at `path`.
    pub fn open_sqlite(path: impl AsRef<Path>) -> Result<Self, String> {
        Ok(Self {
            inner: Inner::Sqlite(SqliteBackend::open(path)?),
        })
    }

    /// Open an in-memory SQLite backend. Useful for tests.
    pub fn open_sqlite_in_memory() -> Result<Self, String> {
        Ok(Self {
            inner: Inner::Sqlite(SqliteBackend::open_in_memory()?),
        })
    }

    /// Return the backend kind.
    pub fn backend_kind(&self) -> BackendKind {
        match &self.inner {
            Inner::Memory(_) => BackendKind::InMemory,
            Inner::Sqlite(_) => BackendKind::Sqlite,
        }
    }

    /// Look up a record by artifact id.
    pub fn get(&self, id: &ArtifactId) -> Result<Option<IndexRecord>, String> {
        match &self.inner {
            Inner::Memory(b) => Ok(b.get(id)),
            Inner::Sqlite(b) => b.get(id),
        }
    }

    /// Insert or update a record.
    pub fn put(&mut self, record: IndexRecord) -> Result<(), String> {
        match &mut self.inner {
            Inner::Memory(b) => {
                b.put(record);
                Ok(())
            }
            Inner::Sqlite(b) => b.put(record),
        }
    }

    /// Insert or update many records atomically (SQLite) or in bulk (memory).
    pub fn put_batch(&mut self, records: &[IndexRecord]) -> Result<(), String> {
        match &mut self.inner {
            Inner::Memory(b) => {
                b.put_batch(records);
                Ok(())
            }
            Inner::Sqlite(b) => b.put_batch(records),
        }
    }

    /// Remove a record. Returns `true` if it existed.
    pub fn delete(&mut self, id: &ArtifactId) -> Result<bool, String> {
        match &mut self.inner {
            Inner::Memory(b) => Ok(b.delete(id)),
            Inner::Sqlite(b) => b.delete(id),
        }
    }

    /// List all records.
    pub fn list(&self) -> Result<Vec<IndexRecord>, String> {
        match &self.inner {
            Inner::Memory(b) => Ok(b.list()),
            Inner::Sqlite(b) => b.list(),
        }
    }

    /// Apply a delta to the index. This is the primary incremental update path.
    pub fn apply_delta(&mut self, delta: IndexDelta) -> Result<(), String> {
        match &mut self.inner {
            Inner::Memory(b) => {
                b.apply_delta(delta);
                Ok(())
            }
            Inner::Sqlite(b) => b.apply_delta(delta),
        }
    }

    /// Return the current monotonic generation of the index.
    pub fn generation(&self) -> Result<u64, String> {
        match &self.inner {
            Inner::Memory(b) => Ok(b.generation()),
            Inner::Sqlite(b) => b.generation(),
        }
    }

    /// Advance the generation counter and return the new value.
    pub fn bump_generation(&mut self) -> Result<u64, String> {
        match &mut self.inner {
            Inner::Memory(b) => {
                b.bump_generation();
                Ok(b.generation())
            }
            Inner::Sqlite(b) => b.bump_generation(),
        }
    }

    /// Set the generation counter explicitly.
    pub fn set_generation(&mut self, generation: u64) -> Result<(), String> {
        match &mut self.inner {
            Inner::Memory(b) => {
                b.set_generation(generation);
                Ok(())
            }
            Inner::Sqlite(b) => b.set_generation(generation),
        }
    }

    /// Remove records with a generation older than `min_generation`.
    ///
    /// This is the local index equivalent of garbage collection: after a sync
    /// bumps the generation, callers can prune entries that were not refreshed.
    pub fn prune_stale_generations(&mut self, min_generation: u64) -> Result<usize, String> {
        match &mut self.inner {
            Inner::Memory(_) => Ok(0),
            Inner::Sqlite(b) => b.prune_stale_generations(min_generation),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn in_memory_round_trip() {
        let mut index = BoarIndex::open_in_memory();
        let id = ArtifactId("abc".into());
        index.put(IndexRecord::new(id.clone())).unwrap();
        assert!(index.get(&id).unwrap().is_some());
    }

    #[test]
    fn sqlite_round_trip() {
        let dir = tempfile::tempdir().unwrap();
        let mut index = BoarIndex::open_sqlite(dir.path().join("index.db")).unwrap();
        let id = ArtifactId("abc".into());
        index.put(IndexRecord::new(id.clone())).unwrap();
        assert!(index.get(&id).unwrap().is_some());
    }

    #[test]
    fn generation_advances() {
        let mut index = BoarIndex::open_in_memory();
        assert_eq!(index.generation().unwrap(), 0);
        let generation = index.bump_generation().unwrap();
        assert_eq!(generation, 1);
        assert_eq!(index.generation().unwrap(), 1);
    }
}
