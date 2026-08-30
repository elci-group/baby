//! SQLite-backed index backend.
//!
//! Stores [`IndexRecord`]s in a local SQLite database. The schema is designed
//! so that `artifact_id` lookups are primary-key lookups. JSON is used only
//! for the variable-length list fields (`features`, `locations`) to keep the
//! table simple and portable.

use std::path::Path;

use boar_core::ArtifactId;
use rusqlite::{Connection, OptionalExtension};

use crate::record::{IndexDelta, IndexRecord, StoredRecord};

const SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS index_records (
    artifact_id TEXT PRIMARY KEY NOT NULL,
    recipe_id TEXT NOT NULL,
    toolchain_id TEXT NOT NULL,
    target_id TEXT NOT NULL,
    profile_id TEXT NOT NULL,
    features TEXT NOT NULL,
    size INTEGER NOT NULL,
    compressed_size INTEGER NOT NULL,
    checksum TEXT NOT NULL,
    locations TEXT NOT NULL,
    availability INTEGER NOT NULL,
    verification_state INTEGER NOT NULL,
    last_seen_secs INTEGER NOT NULL,
    last_seen_nanos INTEGER NOT NULL,
    observed_retrieval_latency_ns INTEGER NOT NULL,
    observed_throughput_bps INTEGER NOT NULL,
    historical_compile_time_ns INTEGER NOT NULL,
    historical_retrieval_time_ns INTEGER NOT NULL,
    confidence INTEGER NOT NULL,
    generation INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_records_recipe ON index_records(recipe_id);
CREATE INDEX IF NOT EXISTS idx_records_toolchain ON index_records(toolchain_id);
CREATE INDEX IF NOT EXISTS idx_records_target ON index_records(target_id);
CREATE INDEX IF NOT EXISTS idx_records_generation ON index_records(generation);

CREATE TABLE IF NOT EXISTS index_meta (
    key TEXT PRIMARY KEY NOT NULL,
    value INTEGER NOT NULL
);
"#;

const UPSERT_SQL: &str = r#"
INSERT INTO index_records
 (artifact_id, recipe_id, toolchain_id, target_id, profile_id, features,
  size, compressed_size, checksum, locations, availability,
  verification_state, last_seen_secs, last_seen_nanos,
  observed_retrieval_latency_ns, observed_throughput_bps,
  historical_compile_time_ns, historical_retrieval_time_ns,
  confidence, generation)
 VALUES
 (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20)
 ON CONFLICT(artifact_id) DO UPDATE SET
  recipe_id = excluded.recipe_id,
  toolchain_id = excluded.toolchain_id,
  target_id = excluded.target_id,
  profile_id = excluded.profile_id,
  features = excluded.features,
  size = excluded.size,
  compressed_size = excluded.compressed_size,
  checksum = excluded.checksum,
  locations = excluded.locations,
  availability = excluded.availability,
  verification_state = excluded.verification_state,
  last_seen_secs = excluded.last_seen_secs,
  last_seen_nanos = excluded.last_seen_nanos,
  observed_retrieval_latency_ns = excluded.observed_retrieval_latency_ns,
  observed_throughput_bps = excluded.observed_throughput_bps,
  historical_compile_time_ns = excluded.historical_compile_time_ns,
  historical_retrieval_time_ns = excluded.historical_retrieval_time_ns,
  confidence = excluded.confidence,
  generation = excluded.generation
"#;

/// SQLite-backed index backend.
#[derive(Debug)]
pub struct SqliteBackend {
    conn: Connection,
}

impl SqliteBackend {
    /// Open or create a SQLite index at `path`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, String> {
        let conn = Connection::open(path).map_err(|e| format!("open sqlite: {e}"))?;
        conn.execute_batch(SCHEMA)
            .map_err(|e| format!("create schema: {e}"))?;
        Ok(Self { conn })
    }

    /// Open an in-memory SQLite backend. Useful for tests.
    pub fn open_in_memory() -> Result<Self, String> {
        let conn = Connection::open_in_memory().map_err(|e| format!("open memory sqlite: {e}"))?;
        conn.execute_batch(SCHEMA)
            .map_err(|e| format!("create schema: {e}"))?;
        Ok(Self { conn })
    }

    pub fn get(&self, id: &ArtifactId) -> Result<Option<IndexRecord>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT artifact_id, recipe_id, toolchain_id, target_id, profile_id, features,
                        size, compressed_size, checksum, locations, availability,
                        verification_state, last_seen_secs, last_seen_nanos,
                        observed_retrieval_latency_ns, observed_throughput_bps,
                        historical_compile_time_ns, historical_retrieval_time_ns,
                        confidence, generation
                 FROM index_records WHERE artifact_id = ?1",
            )
            .map_err(|e| format!("prepare get: {e}"))?;

        let stored = stmt
            .query_row([&id.0], |row| Ok(map_row(row)))
            .optional()
            .map_err(|e| format!("query get: {e}"))?;

        stored.map(|s| s.to_record()).transpose()
    }

    pub fn put(&mut self, record: IndexRecord) -> Result<(), String> {
        let generation = self.generation()?;
        let stored = StoredRecord::from_record(&record, generation);
        self.conn
            .execute(
                UPSERT_SQL,
                rusqlite::params![
                    stored.artifact_id,
                    stored.recipe_id,
                    stored.toolchain_id,
                    stored.target_id,
                    stored.profile_id,
                    stored.features,
                    stored.size,
                    stored.compressed_size,
                    stored.checksum,
                    stored.locations,
                    stored.availability,
                    stored.verification_state,
                    stored.last_seen_secs,
                    stored.last_seen_nanos,
                    stored.observed_retrieval_latency_ns,
                    stored.observed_throughput_bps,
                    stored.historical_compile_time_ns,
                    stored.historical_retrieval_time_ns,
                    stored.confidence,
                    stored.generation,
                ],
            )
            .map_err(|e| format!("upsert: {e}"))?;
        Ok(())
    }

    /// Insert or update many records inside a single transaction.
    pub fn put_batch(&mut self, records: &[IndexRecord]) -> Result<(), String> {
        if records.is_empty() {
            return Ok(());
        }
        let generation = self.generation()?;
        let stored_records: Vec<_> = records
            .iter()
            .map(|r| StoredRecord::from_record(r, generation))
            .collect();
        let tx = self
            .conn
            .transaction()
            .map_err(|e| format!("batch transaction: {e}"))?;
        {
            let mut stmt = tx
                .prepare(UPSERT_SQL)
                .map_err(|e| format!("batch prepare: {e}"))?;
            for stored in &stored_records {
                stmt.execute(rusqlite::params![
                    stored.artifact_id,
                    stored.recipe_id,
                    stored.toolchain_id,
                    stored.target_id,
                    stored.profile_id,
                    stored.features,
                    stored.size,
                    stored.compressed_size,
                    stored.checksum,
                    stored.locations,
                    stored.availability,
                    stored.verification_state,
                    stored.last_seen_secs,
                    stored.last_seen_nanos,
                    stored.observed_retrieval_latency_ns,
                    stored.observed_throughput_bps,
                    stored.historical_compile_time_ns,
                    stored.historical_retrieval_time_ns,
                    stored.confidence,
                    stored.generation,
                ])
                .map_err(|e| format!("batch upsert: {e}"))?;
            }
        }
        tx.commit().map_err(|e| format!("batch commit: {e}"))?;
        Ok(())
    }

    pub fn delete(&mut self, id: &ArtifactId) -> Result<bool, String> {
        let rows = self
            .conn
            .execute("DELETE FROM index_records WHERE artifact_id = ?1", [&id.0])
            .map_err(|e| format!("delete: {e}"))?;
        Ok(rows > 0)
    }

    pub fn list(&self) -> Result<Vec<IndexRecord>, String> {
        let mut stmt = self
            .conn
            .prepare(
                "SELECT artifact_id, recipe_id, toolchain_id, target_id, profile_id, features,
                        size, compressed_size, checksum, locations, availability,
                        verification_state, last_seen_secs, last_seen_nanos,
                        observed_retrieval_latency_ns, observed_throughput_bps,
                        historical_compile_time_ns, historical_retrieval_time_ns,
                        confidence, generation
                 FROM index_records",
            )
            .map_err(|e| format!("prepare list: {e}"))?;

        let records = stmt
            .query_map([], |row| Ok(map_row(row).to_record()))
            .map_err(|e| format!("query list: {e}"))?
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| format!("collect list: {e}"))?
            .into_iter()
            .collect::<Result<Vec<_>, _>>()?;
        Ok(records)
    }

    pub fn apply_delta(&mut self, delta: IndexDelta) -> Result<(), String> {
        match delta {
            IndexDelta::Upsert(record) => self.put(*record),
            IndexDelta::Delete(id) => {
                self.delete(&id)?;
                Ok(())
            }
            IndexDelta::MarkUnavailable(id) => {
                self.conn
                    .execute(
                        "UPDATE index_records SET availability = ?1 WHERE artifact_id = ?2",
                        rusqlite::params![
                            availability_to_sql(boar_core::Availability::Unavailable),
                            id.0
                        ],
                    )
                    .map_err(|e| format!("mark unavailable: {e}"))?;
                Ok(())
            }
            IndexDelta::MarkVerified(id) => {
                self.conn
                    .execute(
                        "UPDATE index_records SET verification_state = ?1 WHERE artifact_id = ?2",
                        rusqlite::params![
                            verification_to_sql(boar_core::VerificationState::Verified),
                            id.0
                        ],
                    )
                    .map_err(|e| format!("mark verified: {e}"))?;
                Ok(())
            }
        }
    }

    pub fn generation(&self) -> Result<u64, String> {
        let mut stmt = self
            .conn
            .prepare("SELECT value FROM index_meta WHERE key = 'generation'")
            .map_err(|e| format!("prepare generation: {e}"))?;
        let value: Result<i64, _> = stmt.query_row([], |row| row.get(0));
        match value {
            Ok(v) => Ok(v as u64),
            Err(rusqlite::Error::QueryReturnedNoRows) => Ok(0),
            Err(e) => Err(format!("query generation: {e}")),
        }
    }

    pub fn bump_generation(&mut self) -> Result<u64, String> {
        let current = self.generation()?;
        let next = current.saturating_add(1);
        self.set_generation(next)?;
        Ok(next)
    }

    pub fn set_generation(&mut self, generation: u64) -> Result<(), String> {
        self.conn
            .execute(
                "INSERT INTO index_meta (key, value) VALUES ('generation', ?1)
                 ON CONFLICT(key) DO UPDATE SET value = excluded.value",
                [generation as i64],
            )
            .map_err(|e| format!("set generation: {e}"))?;
        Ok(())
    }

    /// Remove records with a generation older than `min_generation`.
    pub fn prune_stale_generations(&mut self, min_generation: u64) -> Result<usize, String> {
        let rows = self
            .conn
            .execute(
                "DELETE FROM index_records WHERE generation < ?1",
                [min_generation as i64],
            )
            .map_err(|e| format!("prune generations: {e}"))?;
        Ok(rows)
    }
}

fn map_row(row: &rusqlite::Row<'_>) -> StoredRecord {
    StoredRecord {
        artifact_id: row.get(0).unwrap_or_default(),
        recipe_id: row.get(1).unwrap_or_default(),
        toolchain_id: row.get(2).unwrap_or_default(),
        target_id: row.get(3).unwrap_or_default(),
        profile_id: row.get(4).unwrap_or_default(),
        features: row.get(5).unwrap_or_default(),
        size: row.get(6).unwrap_or_default(),
        compressed_size: row.get(7).unwrap_or_default(),
        checksum: row.get(8).unwrap_or_default(),
        locations: row.get(9).unwrap_or_default(),
        availability: row.get(10).unwrap_or_default(),
        verification_state: row.get(11).unwrap_or_default(),
        last_seen_secs: row.get(12).unwrap_or_default(),
        last_seen_nanos: row.get(13).unwrap_or_default(),
        observed_retrieval_latency_ns: row.get(14).unwrap_or_default(),
        observed_throughput_bps: row.get(15).unwrap_or_default(),
        historical_compile_time_ns: row.get(16).unwrap_or_default(),
        historical_retrieval_time_ns: row.get(17).unwrap_or_default(),
        confidence: row.get(18).unwrap_or_default(),
        generation: row.get::<_, i64>(19).unwrap_or_default() as u64,
    }
}

fn availability_to_sql(a: boar_core::Availability) -> i64 {
    match a {
        boar_core::Availability::Unknown => 0,
        boar_core::Availability::Available => 1,
        boar_core::Availability::Unavailable => 2,
        boar_core::Availability::Expired => 3,
    }
}

fn verification_to_sql(v: boar_core::VerificationState) -> i64 {
    match v {
        boar_core::VerificationState::Unverified => 0,
        boar_core::VerificationState::Verified => 1,
        boar_core::VerificationState::Failed => 2,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::SystemTime;

    #[test]
    fn round_trip() {
        let mut backend = SqliteBackend::open_in_memory().unwrap();
        let id = ArtifactId("abc".into());
        let mut record = IndexRecord::new(id.clone());
        record.size = 1234;
        record.last_seen = SystemTime::now();
        backend.put(record.clone()).unwrap();
        let got = backend.get(&id).unwrap().unwrap();
        assert_eq!(got.size, record.size);
        assert_eq!(got.last_seen, record.last_seen);
    }

    #[test]
    fn delete_removes_record() {
        let mut backend = SqliteBackend::open_in_memory().unwrap();
        let id = ArtifactId("abc".into());
        backend.put(IndexRecord::new(id.clone())).unwrap();
        assert!(backend.delete(&id).unwrap());
        assert!(backend.get(&id).unwrap().is_none());
    }

    #[test]
    fn generation_is_persisted() {
        let mut backend = SqliteBackend::open_in_memory().unwrap();
        assert_eq!(backend.generation().unwrap(), 0);
        let generation = backend.bump_generation().unwrap();
        assert_eq!(generation, 1);
        assert_eq!(backend.generation().unwrap(), 1);
    }

    #[test]
    fn delta_mark_unavailable() {
        let mut backend = SqliteBackend::open_in_memory().unwrap();
        let id = ArtifactId("abc".into());
        backend.put(IndexRecord::new(id.clone())).unwrap();
        backend
            .apply_delta(IndexDelta::MarkUnavailable(id.clone()))
            .unwrap();
        let record = backend.get(&id).unwrap().unwrap();
        assert_eq!(record.availability, boar_core::Availability::Unavailable);
    }

    #[test]
    fn prune_stale_generations_removes_old() {
        let mut backend = SqliteBackend::open_in_memory().unwrap();
        backend.set_generation(1).unwrap();
        let id1 = ArtifactId("old".into());
        backend.put(IndexRecord::new(id1.clone())).unwrap();
        backend.set_generation(2).unwrap();
        let id2 = ArtifactId("new".into());
        backend.put(IndexRecord::new(id2.clone())).unwrap();

        let removed = backend.prune_stale_generations(2).unwrap();
        assert_eq!(removed, 1);
        assert!(backend.get(&id1).unwrap().is_none());
        assert!(backend.get(&id2).unwrap().is_some());
    }
}
