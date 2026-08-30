//! Deterministic compilation identities for Rust/Cargo builds.
//!
//! A [`CompilationIdentity`] captures every input that can change the output of
//! a Rust compilation: source files, compiler, target, profile, features,
//! rustflags, dependencies, build-script outputs, linker, environment, and
//! Boarish's own schema version. The canonical serialization is sorted and
//! delimited so that equivalent inputs always produce the same [`ComputationId`].

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::Path;

use boaring::ComputationId;

use crate::BOARISH_SCHEMA_VERSION;

/// All inputs that participate in a compilation identity.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IdentityInputs {
    /// Relative path → content fingerprint for every source file.
    pub source_files: BTreeMap<String, String>,
    /// `rustc --version --verbose` or similar compiler fingerprint.
    pub rustc_version: String,
    /// Target triple, e.g. `x86_64-unknown-linux-gnu`.
    pub target_triple: String,
    /// Cargo profile, e.g. `dev` or `release`.
    pub profile: String,
    /// Enabled feature names, sorted.
    pub features: Vec<String>,
    /// RUSTFLAGS and related flags, sorted.
    pub rustflags: Vec<String>,
    /// Dependency name → version or identity digest.
    pub dependency_identities: BTreeMap<String, String>,
    /// Build-script output key/value pairs.
    pub build_script_outputs: BTreeMap<String, String>,
    /// Linker command/identity.
    pub linker: String,
    /// Environment variables considered build-relevant.
    pub relevant_env: BTreeMap<String, String>,
    /// Free-form build configuration key/value pairs.
    pub build_config: BTreeMap<String, String>,
    /// Boarish schema version; injected automatically.
    pub boarish_schema_version: String,
}

impl IdentityInputs {
    /// Create empty inputs with the current schema version already filled in.
    pub fn new() -> Self {
        Self {
            boarish_schema_version: BOARISH_SCHEMA_VERSION.to_string(),
            ..Self::default()
        }
    }

    /// Return a deterministic, human-readable-ish canonical form.
    pub fn canonical_form(&self) -> String {
        let mut out = String::new();
        let w = &mut out;

        writeln!(w, "schema={}", self.boarish_schema_version).unwrap();
        writeln!(w, "rustc={}", self.rustc_version).unwrap();
        writeln!(w, "target={}", self.target_triple).unwrap();
        writeln!(w, "profile={}", self.profile).unwrap();
        writeln!(w, "linker={}", self.linker).unwrap();

        for (k, v) in &self.source_files {
            writeln!(w, "src:{k}={v}").unwrap();
        }
        let mut features = self.features.clone();
        features.sort_unstable();
        for f in &features {
            writeln!(w, "feature={f}").unwrap();
        }
        let mut rustflags = self.rustflags.clone();
        rustflags.sort_unstable();
        for r in &rustflags {
            writeln!(w, "rustflag={r}").unwrap();
        }
        for (k, v) in &self.dependency_identities {
            writeln!(w, "dep:{k}={v}").unwrap();
        }
        for (k, v) in &self.build_script_outputs {
            writeln!(w, "build:{k}={v}").unwrap();
        }
        for (k, v) in &self.relevant_env {
            writeln!(w, "env:{k}={v}").unwrap();
        }
        for (k, v) in &self.build_config {
            writeln!(w, "cfg:{k}={v}").unwrap();
        }

        out
    }

    /// Compute the deterministic [`ComputationId`] for these inputs.
    pub fn compute_id(&self) -> ComputationId {
        let canonical = self.canonical_form();
        let digest = fnv1a_hex(canonical.as_bytes());
        ComputationId(digest)
    }
}

/// A prepared compilation identity that can be compared, displayed, and resolved.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompilationIdentity {
    pub inputs: IdentityInputs,
    pub id: ComputationId,
}

impl CompilationIdentity {
    /// Build a [`CompilationIdentity`] from raw inputs and compute its id.
    pub fn from_inputs(inputs: IdentityInputs) -> Self {
        let id = inputs.compute_id();
        Self { inputs, id }
    }

    /// Human-readable explanation of why this identity is what it is.
    pub fn explain(&self) -> String {
        format!(
            "identity {} based on {} source file(s), rustc '{}', target '{}', profile '{}', {} feature(s), {} rustflag(s), {} dep(s), {} build-script output(s)",
            self.id.0,
            self.inputs.source_files.len(),
            self.inputs.rustc_version,
            self.inputs.target_triple,
            self.inputs.profile,
            self.inputs.features.len(),
            self.inputs.rustflags.len(),
            self.inputs.dependency_identities.len(),
            self.inputs.build_script_outputs.len(),
        )
    }
}

/// Fingerprint all `.rs` files under `root` recursively.
pub fn fingerprint_sources(root: &Path) -> Result<BTreeMap<String, String>, String> {
    let mut map = BTreeMap::new();
    if !root.exists() {
        return Ok(map);
    }
    walk_sources(root, root, &mut map)?;
    Ok(map)
}

fn walk_sources(
    base: &Path,
    current: &Path,
    out: &mut BTreeMap<String, String>,
) -> Result<(), String> {
    for entry in std::fs::read_dir(current).map_err(|e| format!("read_dir: {e}"))? {
        let entry = entry.map_err(|e| format!("dir entry: {e}"))?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if path.is_dir() {
            if name == "target" || name.starts_with('.') {
                continue;
            }
            walk_sources(base, &path, out)?;
        } else if path.extension().is_some_and(|e| e == "rs") {
            let rel = path
                .strip_prefix(base)
                .unwrap_or(&path)
                .to_string_lossy()
                .to_string();
            let content = std::fs::read_to_string(&path)
                .map_err(|e| format!("read {}: {e}", path.display()))?;
            out.insert(rel, fnv1a_hex(content.as_bytes()));
        }
    }
    Ok(())
}

/// Compute a hex FNV-1a 64-bit digest of `data`.
///
/// FNV-1a is not cryptographically secure, but it is deterministic, fast, and
/// has good avalanche properties for identity hashing. Integrity verification is
/// delegated to Boaring's cryptographic content hashing.
pub fn fnv1a_hex(data: &[u8]) -> String {
    const FNV_OFFSET: u64 = 0xcbf2_9ce4_8422_2325;
    const FNV_PRIME: u64 = 0x0100_0000_01b3;

    let mut hash = FNV_OFFSET;
    for &byte in data {
        hash ^= u64::from(byte);
        hash = hash.wrapping_mul(FNV_PRIME);
    }
    format!("{hash:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identity_is_deterministic() {
        let a = IdentityInputs {
            rustc_version: "rustc 1.85.0".into(),
            target_triple: "x86_64-unknown-linux-gnu".into(),
            profile: "release".into(),
            features: vec!["foo".into(), "bar".into()],
            ..IdentityInputs::new()
        };
        let b = IdentityInputs {
            rustc_version: "rustc 1.85.0".into(),
            target_triple: "x86_64-unknown-linux-gnu".into(),
            profile: "release".into(),
            features: vec!["bar".into(), "foo".into()],
            ..IdentityInputs::new()
        };
        assert_eq!(a.compute_id().0, b.compute_id().0);
    }

    #[test]
    fn different_inputs_yield_different_ids() {
        let a = IdentityInputs {
            rustc_version: "rustc 1.85.0".into(),
            target_triple: "x86_64-unknown-linux-gnu".into(),
            profile: "release".into(),
            features: vec!["foo".into()],
            ..IdentityInputs::new()
        };
        let b = IdentityInputs {
            rustc_version: "rustc 1.85.0".into(),
            target_triple: "x86_64-unknown-linux-gnu".into(),
            profile: "release".into(),
            features: vec!["bar".into()],
            ..IdentityInputs::new()
        };
        assert_ne!(a.compute_id().0, b.compute_id().0);
    }

    #[test]
    fn canonical_form_is_sorted() {
        let mut a = IdentityInputs::new();
        a.source_files.insert("b.rs".into(), "2".into());
        a.source_files.insert("a.rs".into(), "1".into());

        let mut b = IdentityInputs::new();
        b.source_files.insert("a.rs".into(), "1".into());
        b.source_files.insert("b.rs".into(), "2".into());

        assert_eq!(a.canonical_form(), b.canonical_form());
        assert_eq!(a.compute_id().0, b.compute_id().0);
    }

    #[test]
    fn fnv1a_is_stable() {
        assert_eq!(fnv1a_hex(b"hello"), fnv1a_hex(b"hello"));
        assert_ne!(fnv1a_hex(b"hello"), fnv1a_hex(b"world"));
    }
}
