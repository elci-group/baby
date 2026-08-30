//! Boaring — deterministic persistent computation-reuse substrate.
//!
//! This crate provides content-addressed caching for arbitrary computations.
//! A [`ComputationId`] uniquely identifies a computation from all inputs that
//! can affect its result.  The [`Cache`] stores immutable manifests and
//! content-addressed artifact objects, validates integrity via SHA-256, and
//! supports atomic publication.  The [`Resolver`] adds single-flight
//! deduplication so concurrent identical computations execute only once.

pub mod cache;
pub mod computation_id;
pub mod manifest;
pub mod resolver;
pub mod sha256;
pub mod store;
pub mod telemetry;

pub use cache::{Cache, ResolveResult};
pub use computation_id::ComputationId;
pub use manifest::Manifest;
pub use resolver::Resolver;
pub use sha256::{Sha256, encode_hex};
pub use store::ArtifactStore;
pub use telemetry::{Telemetry, TelemetrySnapshot};
