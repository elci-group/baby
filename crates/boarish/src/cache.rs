//! High-level cache operations on top of Boaring.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use boaring::{ArtifactStore, Cache, ComputationId, Manifest, ResolveResult};

use crate::CompilationIdentity;

/// Reason a build was a hit or a miss, for explainability.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExplainReason {
    /// The artifact was found in the cache and validated.
    Hit { manifest_digest: String },
    /// No matching artifact was found.
    Miss { reason: String },
    /// A matching artifact existed but failed integrity validation.
    ValidationFailure { reason: String },
    /// The cache errored; falling back to normal computation.
    CacheError { reason: String },
}

impl ExplainReason {
    /// Human-readable one-line explanation.
    pub fn explain(&self, id: &ComputationId) -> String {
        match self {
            ExplainReason::Hit { manifest_digest } => {
                format!(
                    "cache hit for {} (manifest digest {})",
                    id.0, manifest_digest
                )
            }
            ExplainReason::Miss { reason } => {
                format!("cache miss for {}: {reason}", id.0)
            }
            ExplainReason::ValidationFailure { reason } => {
                format!("cache validation failed for {}: {reason}", id.0)
            }
            ExplainReason::CacheError { reason } => {
                format!("cache error for {}: {reason}", id.0)
            }
        }
    }
}

/// Outcome of a `resolve` call.
#[derive(Debug, Clone)]
pub struct BuildOutcome {
    pub hit: bool,
    pub artifact_path: PathBuf,
    pub manifest: Option<Manifest>,
    pub reason: ExplainReason,
}

/// Storage and telemetry summary.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CacheStatus {
    pub artifact_count: usize,
    pub total_bytes: u64,
    pub requests: u64,
    pub hits: u64,
    pub misses: u64,
    pub validation_failures: u64,
}

/// Telemetry counters returned by `stats`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Telemetry {
    pub requests: u64,
    pub hits: u64,
    pub misses: u64,
    pub validation_failures: u64,
    pub bytes_stored: u64,
}

/// Boarish wrapper around a Boaring cache.
#[derive(Clone)]
pub struct BoarishCache {
    cache: Arc<Cache>,
    base: PathBuf,
}

impl BoarishCache {
    /// Open or create a Boarish cache at `base`.
    pub fn new(base: impl Into<PathBuf>) -> Result<Self, String> {
        let base = base.into();
        let store = ArtifactStore::new(&base);
        let cache = Arc::new(Cache::new(store)?);
        Ok(Self { cache, base })
    }

    /// Default cache location: `~/.cache/boarish` (or `~/.boarish/cache` on
    /// systems without a configured cache directory).
    pub fn default_location() -> Result<PathBuf, String> {
        if let Some(cache) = std::env::var_os("XDG_CACHE_HOME") {
            return Ok(PathBuf::from(cache).join("boarish"));
        }
        if let Some(home) = std::env::var_os("HOME") {
            return Ok(PathBuf::from(home).join(".cache").join("boarish"));
        }
        Err("cannot determine cache directory: set HOME or XDG_CACHE_HOME".into())
    }

    /// Return the cache base path.
    pub fn base(&self) -> &Path {
        &self.base
    }

    /// Look up an artifact by identity without producing it.
    pub fn lookup(&self, identity: &CompilationIdentity) -> Option<ResolveResult> {
        self.cache.lookup(&identity.id)
    }

    /// Resolve an artifact: return it from cache if present, otherwise run the
    /// provided producer, publish the result, and return the fresh artifact.
    ///
    /// Any cache error falls back to normal computation, matching Boaring's safe
    /// failure semantics.
    pub fn resolve<F>(&self, identity: &CompilationIdentity, producer: F) -> BuildOutcome
    where
        F: FnOnce() -> Result<PathBuf, String> + Send + Sync,
    {
        // Fast path: cache hit.
        if let Some(result) = self.cache.lookup(&identity.id) {
            return BuildOutcome {
                hit: true,
                artifact_path: result.path,
                manifest: Some(result.manifest.clone()),
                reason: ExplainReason::Hit {
                    manifest_digest: result.manifest.content_digest.clone(),
                },
            };
        }

        // Miss path: produce the artifact, then publish it.
        match producer() {
            Ok(path) => match self.insert(identity, &path) {
                Ok(manifest) => BuildOutcome {
                    hit: false,
                    artifact_path: path,
                    manifest: Some(manifest.clone()),
                    reason: ExplainReason::Miss {
                        reason: "artifact produced and stored".into(),
                    },
                },
                Err(e) => BuildOutcome {
                    hit: false,
                    artifact_path: path,
                    manifest: None,
                    reason: ExplainReason::CacheError { reason: e },
                },
            },
            Err(e) => BuildOutcome {
                hit: false,
                artifact_path: PathBuf::new(),
                manifest: None,
                reason: ExplainReason::CacheError { reason: e },
            },
        }
    }

    /// Insert an artifact directly.
    pub fn insert(
        &self,
        identity: &CompilationIdentity,
        artifact: &Path,
    ) -> Result<Manifest, String> {
        let content = fs::read(artifact).map_err(|e| format!("read artifact: {e}"))?;
        let inputs: Vec<(&str, &str)> = identity
            .inputs
            .source_files
            .iter()
            .map(|(k, v)| (k.as_str(), v.as_str()))
            .collect();
        let result = self.cache.insert(
            &identity.id,
            "cargo build",
            &inputs,
            "boarish",
            &identity.inputs.target_triple,
            1,
            "boarish",
            &content,
        )?;
        Ok(result.manifest)
    }

    /// Remove artifacts older than `max_age` and/or reduce storage below `max_size`.
    pub fn prune(&self, max_age: Duration, max_size: u64) -> Result<usize, String> {
        self.cache.prune(max_age, max_size)
    }

    /// Clear all cached artifacts.
    pub fn clear(&self) -> Result<(), String> {
        fs::remove_dir_all(&self.base).map_err(|e| format!("clear cache: {e}"))?;
        fs::create_dir_all(&self.base).map_err(|e| format!("recreate cache root: {e}"))?;
        Ok(())
    }

    /// Verify every cached artifact and return per-id results.
    pub fn verify(&self) -> Vec<(String, bool)> {
        self.list()
            .into_iter()
            .map(|(id, _)| {
                let valid = self.cache.validate(&ComputationId(id.clone()));
                (id, valid)
            })
            .collect()
    }

    /// List cached manifests keyed by computation id.
    pub fn list(&self) -> Vec<(String, Manifest)> {
        let manifests_dir = self.base.join("manifests");
        let mut out = Vec::new();
        let entries = match fs::read_dir(&manifests_dir) {
            Ok(e) => e,
            Err(_) => return out,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let id = path
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .to_string();
            if let Ok(text) = fs::read_to_string(&path) {
                if let Ok(manifest) = Manifest::parse(&text) {
                    out.push((id, manifest));
                }
            }
        }
        out
    }

    /// Inspect a single manifest by id prefix or full id.
    pub fn inspect(&self, id: &str) -> Option<Manifest> {
        self.list()
            .into_iter()
            .find(|(cid, _)| cid == id)
            .map(|(_, m)| m)
    }

    /// Storage and counter status.
    pub fn status(&self) -> CacheStatus {
        let items = self.list();
        let total_bytes: u64 = items
            .iter()
            .map(|(_, m)| {
                fs::metadata(self.base.join("objects").join(&m.content_digest))
                    .map(|meta| meta.len())
                    .unwrap_or(0)
            })
            .sum();
        let snapshot = self.cache.telemetry();
        CacheStatus {
            artifact_count: items.len(),
            total_bytes,
            requests: snapshot.requests,
            hits: snapshot.hits,
            misses: snapshot.misses,
            validation_failures: snapshot.validation_failures,
        }
    }

    /// Hit/miss telemetry.
    pub fn stats(&self) -> Telemetry {
        let snapshot = self.cache.telemetry();
        Telemetry {
            requests: snapshot.requests,
            hits: snapshot.hits,
            misses: snapshot.misses,
            validation_failures: snapshot.validation_failures,
            bytes_stored: snapshot.bytes_stored,
        }
    }
}

/// Helper: format a duration in a human-friendly way.
pub fn format_duration(d: Duration) -> String {
    let secs = d.as_secs();
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3_600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3_600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

/// Helper: return the age of a file in seconds, or `u64::MAX` on error.
pub fn file_age_secs(path: &Path) -> u64 {
    let meta = match fs::metadata(path) {
        Ok(m) => m,
        Err(_) => return u64::MAX,
    };
    let modified = match meta.modified() {
        Ok(t) => t,
        Err(_) => return u64::MAX,
    };
    SystemTime::now()
        .duration_since(modified)
        .unwrap_or(Duration::MAX)
        .as_secs()
}

/// Format a [`SystemTime`] as an RFC 3339-like string.
pub fn format_time(t: SystemTime) -> String {
    let elapsed = t.duration_since(UNIX_EPOCH).unwrap_or(Duration::ZERO);
    format!("{}.{:09}", elapsed.as_secs(), elapsed.subsec_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::IdentityInputs;

    fn temp_identity(seed: &str) -> CompilationIdentity {
        let mut inputs = IdentityInputs::new();
        inputs.rustc_version = seed.into();
        inputs.target_triple = "x86_64-unknown-linux-gnu".into();
        inputs.profile = "dev".into();
        CompilationIdentity::from_inputs(inputs)
    }

    fn test_cache_dir(name: &str) -> PathBuf {
        std::env::temp_dir().join(format!("boarish-test-{name}-{}", std::process::id()))
    }

    #[test]
    fn new_cache_opens_and_status_is_zero() {
        let tmp = test_cache_dir("status");
        let _ = fs::remove_dir_all(&tmp);
        let cache = BoarishCache::new(&tmp).unwrap();
        let status = cache.status();
        assert_eq!(status.artifact_count, 0);
        assert_eq!(status.hits, 0);
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn insert_then_lookup_round_trip() {
        let tmp = test_cache_dir("roundtrip");
        let _ = fs::remove_dir_all(&tmp);
        let cache = BoarishCache::new(&tmp).unwrap();
        let identity = temp_identity("insert-test");

        let artifact = tmp.join("dummy.rlib");
        fs::write(&artifact, b"compiled bytes").unwrap();

        let manifest = cache.insert(&identity, &artifact).unwrap();
        assert!(!manifest.computation_id.0.is_empty());

        let found = cache.lookup(&identity);
        assert!(found.is_some(), "lookup should find inserted artifact");

        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn cache_miss_falls_back_to_producer() {
        let tmp = test_cache_dir("fallback");
        let _ = fs::remove_dir_all(&tmp);
        let cache = BoarishCache::new(&tmp).unwrap();
        let identity = temp_identity("fallback-test");

        let produced = tmp.join("produced.rlib");
        let outcome = cache.resolve(&identity, || {
            fs::write(&produced, b"fresh bytes").unwrap();
            Ok(produced.clone())
        });

        assert!(!outcome.hit);
        assert_eq!(outcome.artifact_path, produced);
        assert!(outcome.manifest.is_some());

        // Second resolve should hit.
        let hit = cache.resolve(&identity, || Ok(produced.clone()));
        assert!(hit.hit);

        let _ = fs::remove_dir_all(&tmp);
    }
}
