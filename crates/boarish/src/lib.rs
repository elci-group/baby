//! Boarish: a Rust/Cargo compilation cache built on top of Boaring.
//!
//! The crate exposes three modules:
//! - [`identity`]: canonical, deterministic compilation identities.
//! - [`cargo`]: helpers for invoking Cargo and fingerprinting inputs.
//! - [`cache`]: high-level cache operations and telemetry.

pub mod cache;
pub mod cargo;
pub mod identity;

pub use cache::{BoarishCache, BuildOutcome, CacheStatus, ExplainReason, Telemetry};
pub use cargo::{CargoInvocation, cargo_version, rustc_fingerprint, source_fingerprint};
pub use identity::{CompilationIdentity, IdentityInputs};

/// Schema version of Boarish's own identity. Bumping this invalidates
/// all previously cached artifacts so that identity semantics can evolve.
pub const BOARISH_SCHEMA_VERSION: &str = "0.1.0";
