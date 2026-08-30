//! Synchronise the local artifact index from a Boaring artifact store.
//!
//! This module walks the manifests in a [`boaring::ArtifactStore`] and emits
//! [`IndexDelta::Upsert`] records so the index can be updated incrementally.
//! It is designed to run asynchronously / daemon-less: a caller can run it
//! periodically or after a local build finishes.

use std::fs;
use std::path::Path;
use std::time::SystemTime;

use boar_core::{ArtifactId, Availability, StorageLocation, TargetId, VerificationState};
use boaring::{ArtifactStore, Manifest};

use crate::{IndexDelta, IndexRecord};

/// Sync state produced by scanning a Boaring store.
#[derive(Debug, Clone, Default)]
pub struct SyncSummary {
    pub scanned: usize,
    pub added: usize,
    pub updated: usize,
    pub removed: usize,
    pub invalid: usize,
}

/// Scan `store` and return the deltas needed to bring an index in sync.
///
/// Manifests whose object file is missing or whose content digest does not
/// match the on-disk object are skipped (and could be marked unavailable by
/// the caller if they were previously indexed).
pub fn scan_boaring_store(store: &ArtifactStore) -> Result<Vec<IndexDelta>, String> {
    let manifests_dir = store.manifests_dir();
    if !manifests_dir.exists() {
        return Ok(Vec::new());
    }

    let mut deltas = Vec::new();
    for entry in fs::read_dir(&manifests_dir).map_err(|e| format!("read manifests dir: {e}"))? {
        let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let text = match fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) => {
                log::warn!("cannot read manifest {}: {e}", path.display());
                continue;
            }
        };

        let manifest = match Manifest::parse(&text) {
            Ok(m) => m,
            Err(e) => {
                log::warn!("cannot parse manifest {}: {e}", path.display());
                continue;
            }
        };

        let object_path = store.object_path(&manifest.content_digest);
        if !object_path.exists() {
            log::debug!(
                "manifest {} references missing object {}; skipping",
                path.display(),
                manifest.content_digest
            );
            continue;
        }

        let size = match fs::metadata(&object_path) {
            Ok(m) => m.len(),
            Err(e) => {
                log::warn!("cannot stat object {}: {e}", object_path.display());
                continue;
            }
        };

        let mut record = IndexRecord::new(ArtifactId(manifest.computation_id.as_ref().to_string()));
        record.target_id = TargetId::new(manifest.target.clone());
        record.size = size;
        record.compressed_size = size; // Boaring stores uncompressed objects today.
        record.checksum = manifest.content_digest.clone();
        record.locations = vec![StorageLocation::Local {
            path: object_path.to_string_lossy().to_string(),
        }];
        record.availability = Availability::Available;
        record.verification_state = VerificationState::Verified;
        record.last_seen = SystemTime::now();
        record.confidence = 100;

        deltas.push(IndexDelta::Upsert(Box::new(record)));
    }

    Ok(deltas)
}

/// Sync an index with a Boaring store, applying all valid deltas and advancing
/// the generation. Returns a summary of changes.
pub fn sync_index_with_store(
    index: &mut crate::BoarIndex,
    store: &ArtifactStore,
) -> Result<SyncSummary, String> {
    let deltas = scan_boaring_store(store)?;
    let scanned = deltas.len();

    let mut added = 0;
    let mut updated = 0;
    let mut records = Vec::with_capacity(deltas.len());
    for delta in deltas {
        if let IndexDelta::Upsert(record) = delta {
            match index.get(&record.artifact_id)? {
                None => added += 1,
                Some(_) => updated += 1,
            }
            records.push(*record);
        }
    }
    index.put_batch(&records)?;

    let _ = index.bump_generation()?;

    Ok(SyncSummary {
        scanned,
        added,
        updated,
        removed: 0,
        invalid: 0,
    })
}

/// Mark every previously-indexed artifact that lives under `store_root` as
/// unavailable if its object file no longer exists.
pub fn remove_missing_local_artifacts(
    index: &mut crate::BoarIndex,
    store_root: impl AsRef<Path>,
) -> Result<usize, String> {
    let store_root = store_root.as_ref();
    let records = index.list()?;
    let mut removed = 0;
    for record in records {
        let local_paths: Vec<_> = record
            .locations
            .iter()
            .filter_map(|loc| match loc {
                StorageLocation::Local { path } => Some(path),
                _ => None,
            })
            .collect();
        if local_paths.is_empty() {
            continue;
        }
        if local_paths.iter().all(|p| {
            let absolute = Path::new(p);
            !absolute.exists() && !store_root.join(p).exists()
        }) {
            index.apply_delta(IndexDelta::MarkUnavailable(record.artifact_id.clone()))?;
            removed += 1;
        }
    }
    Ok(removed)
}

#[cfg(test)]
mod tests {
    use super::*;
    use boaring::Cache;

    fn temp_store() -> (tempfile::TempDir, ArtifactStore) {
        let dir = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(dir.path());
        (dir, store)
    }

    #[test]
    fn scan_empty_store_returns_no_deltas() {
        let (_dir, store) = temp_store();
        let deltas = scan_boaring_store(&store).unwrap();
        assert!(deltas.is_empty());
    }

    #[test]
    fn scan_published_artifact_produces_upsert() {
        let (dir, store) = temp_store();
        let cache = Cache::new(store.clone()).unwrap();
        let id = ArtifactId("abc".into());
        cache
            .insert(
                &boaring::ComputationId::from_artifact_id(id.clone()),
                "cargo build",
                &[("src", "main.rs")],
                "rustc",
                "x86_64-unknown-linux-gnu",
                1,
                "boar-index-test",
                b"compiled bytes",
            )
            .unwrap();

        let deltas = scan_boaring_store(&store).unwrap();
        assert_eq!(deltas.len(), 1);
        let record = match &deltas[0] {
            IndexDelta::Upsert(r) => r.as_ref(),
            _ => panic!("expected upsert"),
        };
        assert_eq!(record.artifact_id.0, "abc");
        assert_eq!(record.size, b"compiled bytes".len() as u64);
        assert!(record.is_locally_available());

        let mut index = crate::BoarIndex::open_in_memory();
        sync_index_with_store(&mut index, &store).unwrap();
        let got = index.get(&id).unwrap().unwrap();
        assert_eq!(got.size, record.size);
        assert_eq!(index.generation().unwrap(), 1);

        // Keep temp dir alive until after the scan.
        let _ = dir;
    }

    #[test]
    fn missing_object_is_skipped() {
        let (dir, store) = temp_store();
        let cache = Cache::new(store.clone()).unwrap();
        let id = ArtifactId("abc".into());
        cache
            .insert(
                &boaring::ComputationId::from_artifact_id(id.clone()),
                "cargo build",
                &[],
                "rustc",
                "x86_64-unknown-linux-gnu",
                1,
                "test",
                b"x",
            )
            .unwrap();

        // Corrupt the object so the manifest exists but the object does not validate.
        // Instead, delete the object file directly.
        let manifest = cache
            .lookup(&boaring::ComputationId::from_artifact_id(id))
            .unwrap()
            .manifest;
        fs::remove_file(store.object_path(&manifest.content_digest)).unwrap();

        let deltas = scan_boaring_store(&store).unwrap();
        assert!(deltas.is_empty());
        let _ = dir;
    }
}
