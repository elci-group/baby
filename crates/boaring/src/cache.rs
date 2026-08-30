//! Cache service: lookup, insert, integrity validation, atomic publication,
//! and basic pruning.

use std::collections::HashMap;
use std::fs;
use std::io::{Read, Write};
use std::path::PathBuf;
use std::sync::{Mutex, RwLock};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::computation_id::ComputationId;
use crate::manifest::Manifest;
use crate::sha256::{Sha256, encode_hex};
use crate::store::ArtifactStore;
use crate::telemetry::{Telemetry, TelemetrySnapshot};

/// Result of resolving a computation through the cache.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveResult {
    pub hit: bool,
    pub path: PathBuf,
    pub manifest: Manifest,
}

#[derive(Clone)]
struct IndexEntry {
    content_digest: String,
    size: u64,
    last_access: SystemTime,
}

/// Content-addressed cache with integrity checking and telemetry.
pub struct Cache {
    store: ArtifactStore,
    telemetry: Mutex<Telemetry>,
    index: RwLock<HashMap<String, IndexEntry>>,
}

impl Cache {
    /// Open or create a cache at `store`.
    pub fn new(store: ArtifactStore) -> Result<Self, String> {
        store.ensure_dirs()?;
        let mut cache = Self {
            store,
            telemetry: Mutex::new(Telemetry::default()),
            index: RwLock::new(HashMap::new()),
        };
        cache.load_index()?;
        Ok(cache)
    }

    /// Look up a previously published result.
    ///
    /// On success, updates hit telemetry and the entry's last-access time.
    pub fn lookup(&self, id: &ComputationId) -> Option<ResolveResult> {
        self.telemetry.lock().ok()?.record_request();

        let entry = {
            let index = self.index.read().ok()?;
            index.get(id.as_ref()).cloned()?
        };

        let manifest_path = self.store.manifest_path(id.as_ref());
        let object_path = self.store.object_path(&entry.content_digest);

        if !manifest_path.exists() || !object_path.exists() {
            return None;
        }

        let manifest_text = fs::read_to_string(&manifest_path).ok()?;
        let manifest = Manifest::parse(&manifest_text).ok()?;

        if !self.validate_object(&object_path, &manifest.content_digest) {
            let _ = self
                .telemetry
                .lock()
                .map(|mut t| t.record_validation_failure());
            return None;
        }

        {
            let mut index = self.index.write().ok()?;
            if let Some(e) = index.get_mut(id.as_ref()) {
                e.last_access = SystemTime::now();
            }
        }

        self.telemetry.lock().ok()?.record_hit();
        Some(ResolveResult {
            hit: true,
            path: object_path,
            manifest,
        })
    }

    /// Atomically publish a new result into the cache.
    ///
    /// The publication path is: temporary quarantine files → sealed object →
    /// verified manifest → published index entry.  If the object already exists
    /// it is reused; the manifest is always written/updated.
    #[allow(clippy::too_many_arguments)]
    pub fn insert(
        &self,
        id: &ComputationId,
        operation: &str,
        inputs: &[(&str, &str)],
        implementation: &str,
        target: &str,
        schema_version: u64,
        provenance: &str,
        content: &[u8],
    ) -> Result<ResolveResult, String> {
        self.telemetry
            .lock()
            .map_err(|e| format!("telemetry lock poisoned: {e}"))?
            .record_request();

        let content_digest = encode_hex(&Sha256::new().chain(content));
        let manifest = Manifest::new(
            id.clone(),
            content_digest.clone(),
            operation.to_string(),
            inputs
                .iter()
                .map(|(k, v)| (k.to_string(), v.to_string()))
                .collect(),
            implementation.to_string(),
            target.to_string(),
            schema_version,
            provenance.to_string(),
        );

        let object_path = self.store.object_path(&content_digest);
        let manifest_path = self.store.manifest_path(id.as_ref());

        // Write object to quarantine, then atomically publish.
        if !object_path.exists() {
            let tmp_object = self.unique_quarantine_path("object");
            fs::write(&tmp_object, content)
                .map_err(|e| format!("failed to write quarantine object: {e}"))?;
            fs::rename(&tmp_object, &object_path)
                .map_err(|e| format!("failed to publish object: {e}"))?;
        }

        // Write manifest to quarantine, then atomically publish.
        let tmp_manifest = self.unique_quarantine_path("manifest");
        fs::write(&tmp_manifest, manifest.render())
            .map_err(|e| format!("failed to write quarantine manifest: {e}"))?;
        fs::rename(&tmp_manifest, &manifest_path)
            .map_err(|e| format!("failed to publish manifest: {e}"))?;

        // Validate the published object before indexing.
        if !self.validate_object(&object_path, &content_digest) {
            // Roll back the manifest so the corrupt entry is not advertised.
            let _ = fs::remove_file(&manifest_path);
            return Err("published object failed integrity validation".to_string());
        }

        let size = content.len() as u64;
        {
            let mut index = self
                .index
                .write()
                .map_err(|e| format!("index write lock poisoned: {e}"))?;
            index.insert(
                id.as_ref().to_string(),
                IndexEntry {
                    content_digest,
                    size,
                    last_access: SystemTime::now(),
                },
            );
        }
        self.save_index()?;

        self.telemetry
            .lock()
            .map_err(|e| format!("telemetry lock poisoned: {e}"))?
            .record_miss(size);
        Ok(ResolveResult {
            hit: false,
            path: object_path,
            manifest,
        })
    }

    /// Validate the object referenced by `id` against its manifest digest.
    pub fn validate(&self, id: &ComputationId) -> bool {
        let Some(entry) = self
            .index
            .read()
            .ok()
            .and_then(|i| i.get(id.as_ref()).cloned())
        else {
            return false;
        };
        let manifest_path = self.store.manifest_path(id.as_ref());
        let object_path = self.store.object_path(&entry.content_digest);
        let Ok(manifest_text) = fs::read_to_string(&manifest_path) else {
            return false;
        };
        let Ok(manifest) = Manifest::parse(&manifest_text) else {
            return false;
        };
        let valid = self.validate_object(&object_path, &manifest.content_digest);
        if !valid {
            let _ = self
                .telemetry
                .lock()
                .map(|mut t| t.record_validation_failure());
        }
        valid
    }

    /// Remove entries older than `max_age` and/or until total stored size is
    /// below `max_size`.  Returns the number of artifacts removed.
    pub fn prune(&self, max_age: Duration, max_size: u64) -> Result<usize, String> {
        let now = SystemTime::now();
        let to_remove = {
            let index = self
                .index
                .read()
                .map_err(|e| format!("index read lock poisoned: {e}"))?;
            let mut candidates: Vec<_> = index
                .iter()
                .map(|(id, e)| (id.clone(), e.content_digest.clone(), e.size, e.last_access))
                .collect();

            let mut remove = Vec::new();
            for (id, digest, size, last_access) in &candidates {
                if now.duration_since(*last_access).unwrap_or(Duration::ZERO) > max_age {
                    remove.push((id.clone(), digest.clone(), *size));
                }
            }

            // If still over size budget, evict oldest entries first.
            let mut total: u64 = candidates.iter().map(|(_, _, s, _)| s).sum();
            candidates.sort_by_key(|(_, _, _, t)| *t);
            for (id, digest, size, _) in candidates {
                if total <= max_size {
                    break;
                }
                if !remove.iter().any(|(rid, _, _)| rid == &id) {
                    remove.push((id, digest, size));
                    total -= size;
                }
            }
            remove
        };

        let mut removed = 0usize;
        let mut freed = 0u64;
        {
            let mut index = self
                .index
                .write()
                .map_err(|e| format!("index write lock poisoned: {e}"))?;
            for (id, _digest, size) in &to_remove {
                index.remove(id);
                let _ = fs::remove_file(self.store.manifest_path(id));
                // Objects are content-addressed and may be shared; leave them in
                // place for a separate object GC pass.
                removed += 1;
                freed += size;
            }
        }
        if removed > 0 {
            self.save_index()?;
            self.telemetry
                .lock()
                .map_err(|e| format!("telemetry lock poisoned: {e}"))?
                .release_bytes(freed);
        }
        Ok(removed)
    }

    /// Return a snapshot of current telemetry counters.
    pub fn telemetry(&self) -> TelemetrySnapshot {
        self.telemetry
            .lock()
            .map(|t| t.snapshot())
            .unwrap_or_default()
    }

    fn validate_object(&self, path: &PathBuf, expected_digest: &str) -> bool {
        let Ok(mut file) = fs::File::open(path) else {
            return false;
        };
        let mut hasher = Sha256::new();
        let mut buf = [0u8; 8192];
        loop {
            match file.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => hasher.update(&buf[..n]),
                Err(_) => return false,
            }
        }
        encode_hex(&hasher.finalize()) == expected_digest
    }

    fn load_index(&mut self) -> Result<(), String> {
        let path = self.store.index_path("main");
        if !path.exists() {
            return Ok(());
        }
        let text = fs::read_to_string(&path).map_err(|e| format!("failed to read index: {e}"))?;
        let mut index = HashMap::new();
        for (lineno, line) in text.lines().enumerate() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() != 5 {
                return Err(format!("index line {}: expected 5 fields", lineno + 1));
            }
            let id = parts[0].to_string();
            let content_digest = parts[1].to_string();
            let size: u64 = parts[2].parse().map_err(|e| format!("index size: {e}"))?;
            let secs: u64 = parts[3].parse().map_err(|e| format!("index secs: {e}"))?;
            let nanos: u32 = parts[4].parse().map_err(|e| format!("index nanos: {e}"))?;
            index.insert(
                id,
                IndexEntry {
                    content_digest,
                    size,
                    last_access: UNIX_EPOCH + Duration::new(secs, nanos),
                },
            );
        }
        self.index = RwLock::new(index);
        Ok(())
    }

    fn save_index(&self) -> Result<(), String> {
        let path = self.store.index_path("main");
        let tmp = self.unique_quarantine_path("index");
        {
            let index = self
                .index
                .read()
                .map_err(|e| format!("index read lock poisoned: {e}"))?;
            let mut file =
                fs::File::create(&tmp).map_err(|e| format!("failed to create index temp: {e}"))?;
            for (id, entry) in index.iter() {
                let elapsed = entry
                    .last_access
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or(Duration::ZERO);
                writeln!(
                    file,
                    "{} {} {} {} {}",
                    id,
                    entry.content_digest,
                    entry.size,
                    elapsed.as_secs(),
                    elapsed.subsec_nanos()
                )
                .map_err(|e| format!("failed to write index: {e}"))?;
            }
        }
        fs::rename(&tmp, &path).map_err(|e| format!("failed to publish index: {e}"))?;
        Ok(())
    }

    fn unique_quarantine_path(&self, prefix: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_nanos();
        let pid = std::process::id();
        self.store
            .quarantine_path(&format!("{prefix}-{pid}-{nanos}"))
    }
}

/// Convenience extension for streaming SHA-256 over a byte slice.
trait Sha256Ext {
    fn chain(self, data: &[u8]) -> [u8; 32];
}

impl Sha256Ext for Sha256 {
    fn chain(mut self, data: &[u8]) -> [u8; 32] {
        self.update(data);
        self.finalize()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_cache() -> (tempdir::TempDir, Cache) {
        let dir = tempdir::TempDir::new().unwrap();
        let cache = Cache::new(ArtifactStore::new(dir.path())).unwrap();
        (dir, cache)
    }

    #[test]
    fn insert_then_lookup_returns_hit() {
        let (_dir, cache) = temp_cache();
        let id = ComputationId::new("op", &[("k", "v")], "i", "1", &[], &[], "t", 1);
        cache
            .insert(&id, "op", &[("k", "v")], "i", "t", 1, "test", b"hello")
            .unwrap();
        let res = cache.lookup(&id).unwrap();
        assert!(res.hit);
        assert_eq!(
            res.manifest.content_digest,
            encode_hex(&Sha256::new().chain(b"hello"))
        );
    }

    #[test]
    fn lookup_miss_returns_none() {
        let (_dir, cache) = temp_cache();
        let id = ComputationId::new("op", &[], "i", "1", &[], &[], "t", 1);
        assert!(cache.lookup(&id).is_none());
    }

    #[test]
    fn validate_detects_corruption() {
        let (dir, cache) = temp_cache();
        let id = ComputationId::new("op", &[], "i", "1", &[], &[], "t", 1);
        cache
            .insert(&id, "op", &[], "i", "t", 1, "test", b"good")
            .unwrap();
        assert!(cache.validate(&id));

        // Corrupt the object file directly.
        let digest = cache.lookup(&id).unwrap().manifest.content_digest;
        fs::write(dir.path().join("objects").join(&digest), b"bad").unwrap();
        assert!(!cache.validate(&id));
    }

    #[test]
    fn prune_removes_old_entries() {
        let (_dir, cache) = temp_cache();
        let id = ComputationId::new("op", &[], "i", "1", &[], &[], "t", 1);
        cache
            .insert(&id, "op", &[], "i", "t", 1, "test", b"x")
            .unwrap();
        let removed = cache.prune(Duration::from_secs(0), u64::MAX).unwrap();
        assert_eq!(removed, 1);
        assert!(cache.lookup(&id).is_none());
    }

    // Minimal tempdir replacement for tests.
    mod tempdir {
        use std::path::PathBuf;
        use std::sync::atomic::{AtomicU64, Ordering};

        pub struct TempDir(PathBuf);

        impl TempDir {
            pub fn new() -> std::io::Result<Self> {
                static COUNTER: AtomicU64 = AtomicU64::new(0);
                let mut p = std::env::temp_dir();
                p.push(format!(
                    "boaring-test-{}-{}",
                    std::process::id(),
                    COUNTER.fetch_add(1, Ordering::SeqCst)
                ));
                std::fs::create_dir_all(&p)?;
                Ok(Self(p))
            }

            pub fn path(&self) -> &std::path::Path {
                &self.0
            }
        }

        impl Drop for TempDir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }
}
