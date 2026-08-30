//! Shared identity, protocol, and capability types for the Baby/Boar/Boarish
//! ecosystem.
//!
//! This crate is intentionally dependency-light (optional `serde`) so that
//! protocol crates, storage backends, and scheduler crates can all share the
//! same type definitions without dragging in heavy build dependencies.

#![cfg_attr(not(feature = "serde"), no_std)]

use core::fmt;
use core::hash::Hash;

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

/// A deterministic content/environment identity for a compiled artifact.
///
/// Equivalent inputs must produce equivalent `ArtifactId`s; non-equivalent
/// inputs must not collide. The canonical identity is a SHA-256 digest rendered
/// as lowercase hexadecimal.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ArtifactId(pub String);

impl ArtifactId {
    /// Build a deterministic artifact identity from all build-relevant inputs.
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
        let mut hasher = Sha256::new();

        hasher.update(operation.as_bytes());
        hasher.update(b"\0");
        hasher.update(implementation.as_bytes());
        hasher.update(b"\0");
        hasher.update(version.as_bytes());
        hasher.update(b"\0");
        hasher.update(target.as_bytes());
        hasher.update(b"\0");
        hasher.update(&schema_version.to_be_bytes());
        hasher.update(b"\0");

        Self::hash_map(&mut hasher, inputs);
        hasher.update(b"\0");
        Self::hash_map(&mut hasher, config);
        hasher.update(b"\0");
        Self::hash_map(&mut hasher, environment);

        Self(encode_hex(&hasher.finalize()))
    }

    fn hash_map(hasher: &mut Sha256, entries: &[(&str, &str)]) {
        let mut sorted: alloc::vec::Vec<(&&str, &&str)> =
            entries.iter().map(|(k, v)| (k, v)).collect();
        sorted.sort_unstable();
        for (k, v) in sorted {
            hasher.update(k.as_bytes());
            hasher.update(b"=");
            hasher.update(v.as_bytes());
            hasher.update(b"\0");
        }
    }
}

impl fmt::Display for ArtifactId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl AsRef<str> for ArtifactId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl ArtifactId {
    /// Consume the wrapper and return the underlying hex digest.
    pub fn into_inner(self) -> String {
        self.0
    }
}

impl alloc::borrow::Borrow<str> for ArtifactId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

/// A deterministic identity for a build recipe.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct RecipeId(pub String);

impl RecipeId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for RecipeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Identity of a toolchain (rustc + cargo + relevant wrappers).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ToolchainId(pub String);

impl ToolchainId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for ToolchainId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Target triple plus CPU configuration.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct TargetId(pub String);

impl TargetId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for TargetId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Cargo profile identity.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ProfileId(pub String);

impl ProfileId {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for ProfileId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

/// Sorted, deterministic feature set.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct FeatureSet {
    features: alloc::collections::BTreeSet<alloc::string::String>,
}

impl FeatureSet {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn with_features(
        features: impl IntoIterator<Item = impl Into<alloc::string::String>>,
    ) -> Self {
        let mut set = alloc::collections::BTreeSet::new();
        for f in features {
            set.insert(f.into());
        }
        Self { features: set }
    }

    pub fn insert(&mut self, feature: impl Into<alloc::string::String>) -> bool {
        self.features.insert(feature.into())
    }

    pub fn contains(&self, feature: &str) -> bool {
        self.features.contains(feature)
    }

    pub fn iter(&self) -> impl Iterator<Item = &alloc::string::String> {
        self.features.iter()
    }

    pub fn is_empty(&self) -> bool {
        self.features.is_empty()
    }

    pub fn len(&self) -> usize {
        self.features.len()
    }
}

impl FromIterator<alloc::string::String> for FeatureSet {
    fn from_iter<T: IntoIterator<Item = alloc::string::String>>(iter: T) -> Self {
        Self::with_features(iter)
    }
}

/// Location where an artifact can be retrieved from.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum StorageLocation {
    /// Local filesystem path.
    Local { path: alloc::string::String },
    /// Boar node endpoint (transport-agnostic URI).
    Node {
        node_id: alloc::string::String,
        uri: alloc::string::String,
    },
}

/// Availability state of an indexed artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum Availability {
    #[default]
    Unknown,
    Available,
    Unavailable,
    Expired,
}

/// Verification state of an indexed artifact.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum VerificationState {
    #[default]
    Unverified,
    Verified,
    Failed,
}

/// Capability metadata advertised by a Boar node.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct NodeCapability {
    pub node_id: alloc::string::String,
    pub storage_class: StorageClass,
    pub available_capacity_bytes: u64,
    pub observed_throughput_bps: u64,
    pub observed_latency_ns: u64,
    pub compression_algorithms: alloc::vec::Vec<alloc::string::String>,
    pub targets: alloc::vec::Vec<alloc::string::String>,
    pub artifact_count: u64,
    pub healthy: bool,
    pub load: u8,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub enum StorageClass {
    #[default]
    Memory,
    Ssd,
    Hdd,
    Lan,
    Remote,
}

/// Canonical build recipe before execution.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct BuildRecipe {
    pub crate_name: alloc::string::String,
    pub crate_version: alloc::string::String,
    pub source_digest: Digest,
    pub dependency_digests: alloc::vec::Vec<Digest>,
    pub compiler: CompilerIdentity,
    pub target: TargetId,
    pub profile: ProfileId,
    pub features: FeatureSet,
    pub rustflags: alloc::vec::Vec<alloc::string::String>,
    pub linker: alloc::string::String,
    pub environment: alloc::collections::BTreeMap<alloc::string::String, alloc::string::String>,
}

/// Compiler identity.
#[derive(Debug, Clone, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct CompilerIdentity {
    pub rustc_version: alloc::string::String,
    pub cargo_version: alloc::string::String,
    pub wrapper: Option<alloc::string::String>,
}

/// A cryptographic digest, conventionally SHA-256 lowercase hex.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct Digest(pub String);

impl Digest {
    pub fn new(value: impl Into<String>) -> Self {
        Self(value.into())
    }
}

impl fmt::Display for Digest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

// ---------------------------------------------------------------------------
// Minimal stdlib-only SHA-256 implementation (mirrors boaring's sha256.rs).
// Kept here so boar-core can produce ArtifactIds without depending on boaring.
// ---------------------------------------------------------------------------

struct Sha256 {
    state: [u32; 8],
    buffer: alloc::vec::Vec<u8>,
    total_len: u64,
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

impl Sha256 {
    fn new() -> Self {
        Self {
            state: [
                0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
                0x5be0cd19,
            ],
            buffer: alloc::vec::Vec::new(),
            total_len: 0,
        }
    }

    fn update(&mut self, data: &[u8]) {
        self.total_len += data.len() as u64;
        self.buffer.extend_from_slice(data);

        while self.buffer.len() >= 64 {
            let chunk = &self.buffer[..64];
            Self::compress(&mut self.state, chunk);
            self.buffer.drain(..64);
        }
    }

    fn finalize(mut self) -> [u8; 32] {
        let bit_len = self.total_len.wrapping_mul(8);

        self.buffer.push(0x80);
        while self.buffer.len() % 64 != 56 {
            self.buffer.push(0);
        }
        self.buffer.extend_from_slice(&bit_len.to_be_bytes());

        debug_assert!(self.buffer.len() % 64 == 0);
        let chunks = self.buffer.chunks_exact(64);
        for chunk in chunks {
            Self::compress(&mut self.state, chunk);
        }

        let mut out = [0u8; 32];
        for (i, word) in self.state.iter().enumerate() {
            out[i * 4..(i + 1) * 4].copy_from_slice(&word.to_be_bytes());
        }
        out
    }

    fn compress(state: &mut [u32; 8], chunk: &[u8]) {
        let mut w = [0u32; 64];
        for (i, word) in w.iter_mut().enumerate().take(16) {
            *word = u32::from_be_bytes([
                chunk[i * 4],
                chunk[i * 4 + 1],
                chunk[i * 4 + 2],
                chunk[i * 4 + 3],
            ]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }

        let mut a = state[0];
        let mut b = state[1];
        let mut c = state[2];
        let mut d = state[3];
        let mut e = state[4];
        let mut f = state[5];
        let mut g = state[6];
        let mut h = state[7];

        for i in 0..64 {
            let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
            let ch = (e & f) ^ ((!e) & g);
            let temp1 = h
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
            let maj = (a & b) ^ (a & c) ^ (b & c);
            let temp2 = s0.wrapping_add(maj);

            h = g;
            g = f;
            f = e;
            e = d.wrapping_add(temp1);
            d = c;
            c = b;
            b = a;
            a = temp1.wrapping_add(temp2);
        }

        state[0] = state[0].wrapping_add(a);
        state[1] = state[1].wrapping_add(b);
        state[2] = state[2].wrapping_add(c);
        state[3] = state[3].wrapping_add(d);
        state[4] = state[4].wrapping_add(e);
        state[5] = state[5].wrapping_add(f);
        state[6] = state[6].wrapping_add(g);
        state[7] = state[7].wrapping_add(h);
    }
}

fn encode_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = alloc::string::String::with_capacity(bytes.len() * 2);
    for &b in bytes {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0xf) as usize] as char);
    }
    out
}

extern crate alloc;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_id_is_deterministic() {
        let a = ArtifactId::new(
            "compile",
            &[("src", "a.rs"), ("flags", "-O")],
            "rustc",
            "1.0",
            &[],
            &[],
            "x86_64",
            1,
        );
        let b = ArtifactId::new(
            "compile",
            &[("flags", "-O"), ("src", "a.rs")],
            "rustc",
            "1.0",
            &[],
            &[],
            "x86_64",
            1,
        );
        assert_eq!(a.0, b.0);
    }

    #[test]
    fn different_inputs_yield_different_ids() {
        let a = ArtifactId::new("op", &[("k", "v")], "i", "1", &[], &[], "t", 1);
        let b = ArtifactId::new("op", &[("k", "w")], "i", "1", &[], &[], "t", 1);
        assert_ne!(a.0, b.0);
    }

    #[test]
    fn empty_sha256() {
        assert_eq!(
            encode_hex(&Sha256::new().finalize()),
            "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }
}
