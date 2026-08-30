//! Basic telemetry counters for the cache.

/// Mutable telemetry state kept inside a [`Cache`](crate::cache::Cache).
#[derive(Debug, Default)]
pub struct Telemetry {
    requests: u64,
    hits: u64,
    misses: u64,
    validation_failures: u64,
    bytes_stored: u64,
}

impl Telemetry {
    pub fn record_request(&mut self) {
        self.requests += 1;
    }

    pub fn record_hit(&mut self) {
        self.hits += 1;
    }

    pub fn record_miss(&mut self, bytes: u64) {
        self.misses += 1;
        self.bytes_stored += bytes;
    }

    pub fn record_validation_failure(&mut self) {
        self.validation_failures += 1;
    }

    pub fn release_bytes(&mut self, bytes: u64) {
        self.bytes_stored = self.bytes_stored.saturating_sub(bytes);
    }

    pub fn snapshot(&self) -> TelemetrySnapshot {
        TelemetrySnapshot {
            requests: self.requests,
            hits: self.hits,
            misses: self.misses,
            validation_failures: self.validation_failures,
            bytes_stored: self.bytes_stored,
        }
    }
}

/// Immutable snapshot of cache telemetry.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TelemetrySnapshot {
    pub requests: u64,
    pub hits: u64,
    pub misses: u64,
    pub validation_failures: u64,
    pub bytes_stored: u64,
}
