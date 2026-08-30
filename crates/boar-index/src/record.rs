//! Record types stored in the local artifact index.

use std::time::SystemTime;

use boar_core::{
    ArtifactId, Availability, FeatureSet, ProfileId, RecipeId, StorageLocation, TargetId,
    ToolchainId, VerificationState,
};
use serde::{Deserialize, Serialize};

/// A single entry in the local artifact index.
///
/// This record captures everything Baby/Boarish needs to decide whether an
/// artifact can be reused without synchronous network discovery.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IndexRecord {
    pub artifact_id: ArtifactId,
    pub recipe_id: RecipeId,
    pub toolchain_id: ToolchainId,
    pub target_id: TargetId,
    pub profile_id: ProfileId,
    pub features: FeatureSet,
    pub size: u64,
    pub compressed_size: u64,
    pub checksum: String,
    pub locations: Vec<StorageLocation>,
    pub availability: Availability,
    pub verification_state: VerificationState,
    pub last_seen: SystemTime,
    pub observed_retrieval_latency_ns: u64,
    pub observed_throughput_bps: u64,
    pub historical_compile_time_ns: u64,
    pub historical_retrieval_time_ns: u64,
    pub confidence: u64, // 0-100
}

impl IndexRecord {
    /// Create a minimal record for the given artifact identity.
    pub fn new(artifact_id: ArtifactId) -> Self {
        Self {
            artifact_id,
            recipe_id: RecipeId::new(""),
            toolchain_id: ToolchainId::new(""),
            target_id: TargetId::new(""),
            profile_id: ProfileId::new("dev"),
            features: FeatureSet::new(),
            size: 0,
            compressed_size: 0,
            checksum: String::new(),
            locations: Vec::new(),
            availability: Availability::Unknown,
            verification_state: VerificationState::Unverified,
            last_seen: SystemTime::UNIX_EPOCH,
            observed_retrieval_latency_ns: 0,
            observed_throughput_bps: 0,
            historical_compile_time_ns: 0,
            historical_retrieval_time_ns: 0,
            confidence: 0,
        }
    }

    /// Return the estimated retrieval cost in nanoseconds, or a default based
    /// on size when no observations exist.
    pub fn estimated_retrieval_ns(&self) -> u64 {
        if self.historical_retrieval_time_ns > 0 {
            return self.historical_retrieval_time_ns;
        }
        if self.observed_throughput_bps > 0 && self.compressed_size > 0 {
            // size / throughput, converted to nanoseconds.
            let bits = self.compressed_size.saturating_mul(8);
            return bits.saturating_mul(1_000_000_000) / self.observed_throughput_bps.max(1);
        }
        // Fallback: assume 1 ms/MiB.
        self.compressed_size.saturating_mul(1000) / (1024 * 1024)
    }

    /// Return the estimated compilation cost in nanoseconds, or zero when
    /// unknown.
    pub fn estimated_compile_ns(&self) -> u64 {
        self.historical_compile_time_ns
    }

    /// Whether the record has at least one usable local location.
    pub fn is_locally_available(&self) -> bool {
        self.availability == Availability::Available
            && self
                .locations
                .iter()
                .any(|l| matches!(l, StorageLocation::Local { .. }))
    }
}

/// A change to apply to the index atomically.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IndexDelta {
    Upsert(Box<IndexRecord>),
    Delete(ArtifactId),
    MarkUnavailable(ArtifactId),
    MarkVerified(ArtifactId),
}

/// A serializable snapshot of a record for storage backends that need it.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub(crate) struct StoredRecord {
    pub artifact_id: String,
    pub recipe_id: String,
    pub toolchain_id: String,
    pub target_id: String,
    pub profile_id: String,
    pub features: String,
    pub size: u64,
    pub compressed_size: u64,
    pub checksum: String,
    pub locations: String,
    pub availability: u8,
    pub verification_state: u8,
    pub last_seen_secs: i64,
    pub last_seen_nanos: u32,
    pub observed_retrieval_latency_ns: u64,
    pub observed_throughput_bps: u64,
    pub historical_compile_time_ns: u64,
    pub historical_retrieval_time_ns: u64,
    pub confidence: u64,
    pub generation: u64,
}

impl StoredRecord {
    pub fn from_record(record: &IndexRecord, generation: u64) -> Self {
        let since_epoch = record
            .last_seen
            .duration_since(SystemTime::UNIX_EPOCH)
            .unwrap_or_default();
        Self {
            artifact_id: record.artifact_id.0.clone(),
            recipe_id: record.recipe_id.0.clone(),
            toolchain_id: record.toolchain_id.0.clone(),
            target_id: record.target_id.0.clone(),
            profile_id: record.profile_id.0.clone(),
            features: serde_json::to_string(&record.features).unwrap_or_default(),
            size: record.size,
            compressed_size: record.compressed_size,
            checksum: record.checksum.clone(),
            locations: serde_json::to_string(&record.locations).unwrap_or_default(),
            availability: availability_as_u8(record.availability),
            verification_state: verification_as_u8(record.verification_state),
            last_seen_secs: since_epoch.as_secs() as i64,
            last_seen_nanos: since_epoch.subsec_nanos(),
            observed_retrieval_latency_ns: record.observed_retrieval_latency_ns,
            observed_throughput_bps: record.observed_throughput_bps,
            historical_compile_time_ns: record.historical_compile_time_ns,
            historical_retrieval_time_ns: record.historical_retrieval_time_ns,
            confidence: record.confidence,
            generation,
        }
    }

    pub fn to_record(&self) -> Result<IndexRecord, String> {
        let last_seen = SystemTime::UNIX_EPOCH
            + std::time::Duration::new(self.last_seen_secs as u64, self.last_seen_nanos);
        Ok(IndexRecord {
            artifact_id: ArtifactId(self.artifact_id.clone()),
            recipe_id: RecipeId::new(self.recipe_id.clone()),
            toolchain_id: ToolchainId::new(self.toolchain_id.clone()),
            target_id: TargetId::new(self.target_id.clone()),
            profile_id: ProfileId::new(self.profile_id.clone()),
            features: serde_json::from_str(&self.features)
                .map_err(|e| format!("features json: {e}"))?,
            size: self.size,
            compressed_size: self.compressed_size,
            checksum: self.checksum.clone(),
            locations: serde_json::from_str(&self.locations)
                .map_err(|e| format!("locations json: {e}"))?,
            availability: availability_from_u8(self.availability)?,
            verification_state: verification_from_u8(self.verification_state)?,
            last_seen,
            observed_retrieval_latency_ns: self.observed_retrieval_latency_ns,
            observed_throughput_bps: self.observed_throughput_bps,
            historical_compile_time_ns: self.historical_compile_time_ns,
            historical_retrieval_time_ns: self.historical_retrieval_time_ns,
            confidence: self.confidence,
        })
    }
}

fn availability_as_u8(a: Availability) -> u8 {
    match a {
        Availability::Unknown => 0,
        Availability::Available => 1,
        Availability::Unavailable => 2,
        Availability::Expired => 3,
    }
}

fn availability_from_u8(v: u8) -> Result<Availability, String> {
    match v {
        0 => Ok(Availability::Unknown),
        1 => Ok(Availability::Available),
        2 => Ok(Availability::Unavailable),
        3 => Ok(Availability::Expired),
        _ => Err(format!("unknown availability {v}")),
    }
}

fn verification_as_u8(v: VerificationState) -> u8 {
    match v {
        VerificationState::Unverified => 0,
        VerificationState::Verified => 1,
        VerificationState::Failed => 2,
    }
}

fn verification_from_u8(v: u8) -> Result<VerificationState, String> {
    match v {
        0 => Ok(VerificationState::Unverified),
        1 => Ok(VerificationState::Verified),
        2 => Ok(VerificationState::Failed),
        _ => Err(format!("unknown verification state {v}")),
    }
}
