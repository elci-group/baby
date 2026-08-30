//! Single-flight resolver.
//!
//! Concurrent requests for the same [`ComputationId`] converge on a single
//! producer.  Failed producers release their lease so the next caller retries.

use std::collections::HashMap;
use std::sync::{Arc, Condvar, Mutex};

use crate::cache::{Cache, ResolveResult};
use crate::computation_id::ComputationId;

pub struct Resolver<F> {
    cache: Arc<Cache>,
    in_flight: Mutex<HashMap<String, Arc<InFlight>>>,
    compute: F,
}

struct InFlight {
    mutex: Mutex<InFlightState>,
    condvar: Condvar,
}

#[derive(Clone)]
enum InFlightState {
    Computing,
    Done(Box<Result<ResolveResult, String>>),
}

impl<F> Resolver<F>
where
    F: Fn(&ComputationId) -> Result<Vec<u8>, String> + Send + Sync,
{
    /// Create a resolver backed by `cache` that computes missing results with
    /// `compute`.
    pub fn new(cache: Arc<Cache>, compute: F) -> Self {
        Self {
            cache,
            in_flight: Mutex::new(HashMap::new()),
            compute,
        }
    }

    /// Resolve `id`, using the cache if possible.
    ///
    /// On a cache miss, exactly one concurrent caller will execute `compute`;
    /// the rest wait for the result.  If `compute` fails, all waiters see the
    /// error and the lease is released for the next attempt.
    #[allow(clippy::too_many_arguments)]
    pub fn resolve(
        &self,
        id: &ComputationId,
        operation: &str,
        inputs: &[(&str, &str)],
        implementation: &str,
        target: &str,
        schema_version: u64,
        provenance: &str,
    ) -> Result<ResolveResult, String> {
        // Fast path: already cached.
        if let Some(res) = self.cache.lookup(id) {
            return Ok(res);
        }

        let key = id.as_ref().to_string();

        // Try to install ourselves as the producer.  If another thread is
        // already producing, wait on its in-flight state.
        {
            let mut map = self
                .in_flight
                .lock()
                .map_err(|e| format!("in_flight lock poisoned: {e}"))?;
            if let Some(f) = map.get(&key) {
                let flight = Arc::clone(f);
                drop(map);
                return Self::wait_on(&flight);
            }
            let f = Arc::new(InFlight {
                mutex: Mutex::new(InFlightState::Computing),
                condvar: Condvar::new(),
            });
            map.insert(key.clone(), Arc::clone(&f));
            drop(map);

            // We are the producer.  Re-check the cache in case another producer
            // published while we were waiting on the map lock.
            if let Some(res) = self.cache.lookup(id) {
                self.finish(&f, &key, Ok(res.clone()));
                return Ok(res);
            }

            let result = (self.compute)(id).and_then(|content| {
                self.cache.insert(
                    id,
                    operation,
                    inputs,
                    implementation,
                    target,
                    schema_version,
                    provenance,
                    &content,
                )
            });

            self.finish(&f, &key, result.clone());
            result
        }
    }

    fn finish(&self, flight: &Arc<InFlight>, key: &str, result: Result<ResolveResult, String>) {
        {
            let mut state = flight.mutex.lock().unwrap();
            *state = InFlightState::Done(Box::new(result));
        }
        flight.condvar.notify_all();
        let mut map = self.in_flight.lock().unwrap();
        map.remove(key);
    }

    fn wait_on(flight: &Arc<InFlight>) -> Result<ResolveResult, String> {
        let mut state = flight
            .mutex
            .lock()
            .map_err(|e| format!("flight lock poisoned: {e}"))?;
        loop {
            match &*state {
                InFlightState::Computing => {
                    state = flight
                        .condvar
                        .wait(state)
                        .map_err(|e| format!("condvar wait poisoned: {e}"))?;
                }
                InFlightState::Done(res) => return *res.clone(),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::ArtifactStore;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::thread;

    fn temp_cache() -> (tempdir::TempDir, Arc<Cache>) {
        let dir = tempdir::TempDir::new().unwrap();
        let cache = Arc::new(Cache::new(ArtifactStore::new(dir.path())).unwrap());
        (dir, cache)
    }

    #[test]
    fn single_flight_converges_on_one_producer() {
        let (_dir, cache) = temp_cache();
        let calls = Arc::new(AtomicUsize::new(0));

        let resolver = Arc::new(Resolver::new(cache, {
            let calls = Arc::clone(&calls);
            move |_id| {
                calls.fetch_add(1, Ordering::SeqCst);
                thread::sleep(std::time::Duration::from_millis(50));
                Ok(vec![1, 2, 3])
            }
        }));

        let id = ComputationId::new("op", &[], "i", "1", &[], &[], "t", 1);
        let mut handles = Vec::new();
        for _ in 0..10 {
            let r = Arc::clone(&resolver);
            let id = id.clone();
            handles.push(thread::spawn(move || {
                r.resolve(&id, "op", &[], "i", "t", 1, "test").unwrap()
            }));
        }

        let results: Vec<_> = handles.into_iter().map(|h| h.join().unwrap()).collect();
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let mut h = crate::sha256::Sha256::new();
        h.update(&[1, 2, 3]);
        let expected_digest = crate::sha256::encode_hex(&h.finalize());
        for r in &results {
            assert!(!r.hit);
            assert_eq!(r.manifest.content_digest, expected_digest);
        }

        // Subsequent resolve should be a cache hit.
        let hit = resolver
            .resolve(&id, "op", &[], "i", "t", 1, "test")
            .unwrap();
        assert!(hit.hit);
    }

    #[test]
    fn failed_producer_releases_lease() {
        let (_dir, cache) = temp_cache();
        let calls = Arc::new(AtomicUsize::new(0));

        let resolver = Resolver::new(cache, {
            let calls = Arc::clone(&calls);
            move |_id| {
                calls.fetch_add(1, Ordering::SeqCst);
                Err("boom".to_string())
            }
        });

        let id = ComputationId::new("op", &[], "i", "1", &[], &[], "t", 1);
        assert!(
            resolver
                .resolve(&id, "op", &[], "i", "t", 1, "test")
                .is_err()
        );
        assert!(
            resolver
                .resolve(&id, "op", &[], "i", "t", 1, "test")
                .is_err()
        );
        assert_eq!(calls.load(Ordering::SeqCst), 2);
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
                    "boaring-resolver-test-{}-{}",
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
