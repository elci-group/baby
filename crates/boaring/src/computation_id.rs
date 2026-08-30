//! Deterministic computation identity.
//!
//! A [`ComputationId`] is a stable cryptographic hash over every input that can
//! affect the result of a computation: the operation, sorted key/value inputs,
//! implementation identity and version, sorted configuration, sorted
//! environment, target, and a schema version.
//!
//! The actual hashing logic is delegated to [`boar_core::ArtifactId`] so that
//! the wider Baby/Boar/Boarish ecosystem shares a single canonical identity
//! primitive.

use std::fmt;
use std::hash::Hash;

use boar_core::ArtifactId;

/// A deterministic identifier for a computation.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ComputationId(pub String);

impl ComputationId {
    /// Build a deterministic computation identity from all build-relevant inputs.
    ///
    /// Inputs, configuration, and environment are normalized by sorting their
    /// keys so that different orderings produce the same identity.
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        operation: &str,
        inputs: &[(&str, &str)],
        implementation: &str,
        version: &str,
        config: &[(&str, &str)],
        environment: &[(&str, &str)],
        target: &str,
        schema_version: u64,
    ) -> Self {
        let id = ArtifactId::new(
            operation,
            inputs,
            implementation,
            version,
            config,
            environment,
            target,
            schema_version,
        );
        Self(id.into_inner())
    }

    /// Convert from a canonical [`ArtifactId`].
    pub fn from_artifact_id(id: ArtifactId) -> Self {
        Self(id.into_inner())
    }

    /// Convert into the canonical [`ArtifactId`].
    pub fn into_artifact_id(self) -> ArtifactId {
        ArtifactId(self.0)
    }
}

impl fmt::Display for ComputationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<str> for ComputationId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

// Allow ComputationId to be used as a HashMap key via its string content.
impl std::borrow::Borrow<str> for ComputationId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_deterministic() {
        let id1 = ComputationId::new(
            "compile",
            &[("src", "a.rs"), ("flags", "-O")],
            "rustc",
            "1.0",
            &[],
            &[],
            "x86_64",
            1,
        );
        let id2 = ComputationId::new(
            "compile",
            &[("flags", "-O"), ("src", "a.rs")],
            "rustc",
            "1.0",
            &[],
            &[],
            "x86_64",
            1,
        );
        assert_eq!(id1, id2);
    }

    #[test]
    fn different_inputs_yield_different_ids() {
        let id1 = ComputationId::new("op", &[("k", "v")], "i", "1", &[], &[], "t", 1);
        let id2 = ComputationId::new("op", &[("k", "w")], "i", "1", &[], &[], "t", 1);
        assert_ne!(id1, id2);
    }

    #[test]
    fn schema_version_changes_id() {
        let id1 = ComputationId::new("op", &[], "i", "1", &[], &[], "t", 1);
        let id2 = ComputationId::new("op", &[], "i", "1", &[], &[], "t", 2);
        assert_ne!(id1, id2);
    }

    #[test]
    fn artifact_id_round_trip() {
        let id = ComputationId::new("op", &[], "i", "1", &[], &[], "t", 1);
        let artifact = id.clone().into_artifact_id();
        let back = ComputationId::from_artifact_id(artifact);
        assert_eq!(id, back);
    }
}
