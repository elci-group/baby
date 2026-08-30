//! Content-addressed artifact storage layout.
//!
//! The store organizes data under a base path:
//!
//! ```text
//! <base>/
//!   objects/      -- content-addressed artifact bytes
//!   manifests/    -- computation-id-addressed manifests
//!   indexes/      -- reverse/usage indexes
//!   locks/        -- producer lease files
//!   quarantine/   -- artifacts that failed integrity validation
//!   staging/      -- temporary files being written
//!   sealed/       -- files awaiting final verification
//! ```

use std::fs;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct ArtifactStore {
    base: PathBuf,
}

impl ArtifactStore {
    /// Open a store rooted at `base`.
    pub fn new(base: impl AsRef<Path>) -> Self {
        Self {
            base: base.as_ref().to_path_buf(),
        }
    }

    /// Return the default store path (`~/.boaring/`).
    pub fn default_path() -> Result<PathBuf, String> {
        dirs::home_dir()
            .map(|h| h.join(".boaring"))
            .ok_or_else(|| "unable to determine home directory".to_string())
    }

    /// Ensure that all store directories exist.
    pub fn ensure_dirs(&self) -> Result<(), String> {
        for dir in [
            self.objects_dir(),
            self.manifests_dir(),
            self.indexes_dir(),
            self.locks_dir(),
            self.quarantine_dir(),
            self.staging_dir(),
            self.sealed_dir(),
        ] {
            fs::create_dir_all(&dir)
                .map_err(|e| format!("failed to create {}: {e}", dir.display()))?;
        }
        Ok(())
    }

    pub fn base(&self) -> &Path {
        &self.base
    }

    pub fn objects_dir(&self) -> PathBuf {
        self.base.join("objects")
    }

    pub fn manifests_dir(&self) -> PathBuf {
        self.base.join("manifests")
    }

    pub fn indexes_dir(&self) -> PathBuf {
        self.base.join("indexes")
    }

    pub fn locks_dir(&self) -> PathBuf {
        self.base.join("locks")
    }

    pub fn quarantine_dir(&self) -> PathBuf {
        self.base.join("quarantine")
    }

    pub fn staging_dir(&self) -> PathBuf {
        self.base.join("staging")
    }

    pub fn sealed_dir(&self) -> PathBuf {
        self.base.join("sealed")
    }

    pub fn object_path(&self, digest: &str) -> PathBuf {
        self.objects_dir().join(digest)
    }

    pub fn manifest_path(&self, computation_id: &str) -> PathBuf {
        self.manifests_dir().join(computation_id)
    }

    pub fn index_path(&self, name: &str) -> PathBuf {
        self.indexes_dir().join(name)
    }

    pub fn lock_path(&self, computation_id: &str) -> PathBuf {
        self.locks_dir().join(computation_id)
    }

    pub fn quarantine_path(&self, name: &str) -> PathBuf {
        self.quarantine_dir().join(name)
    }

    pub fn staging_object_path(&self, name: &str) -> PathBuf {
        self.staging_dir().join(format!("obj.{name}"))
    }

    pub fn staging_manifest_path(&self, computation_id: &str) -> PathBuf {
        self.staging_dir().join(format!("mf.{computation_id}"))
    }

    pub fn sealed_object_path(&self, digest: &str) -> PathBuf {
        self.sealed_dir().join(format!("obj.{digest}"))
    }

    pub fn sealed_manifest_path(&self, computation_id: &str) -> PathBuf {
        self.sealed_dir().join(format!("mf.{computation_id}"))
    }

    pub fn quarantine_object_path(&self, digest: &str) -> PathBuf {
        self.quarantine_dir().join(format!("obj.{digest}"))
    }

    pub fn quarantine_manifest_path(&self, computation_id: &str) -> PathBuf {
        self.quarantine_dir().join(format!("mf.{computation_id}"))
    }
}

/// Minimal home-directory helper (stdlib-only fallback).
mod dirs {
    use std::path::PathBuf;

    pub fn home_dir() -> Option<PathBuf> {
        std::env::var_os("HOME").map(PathBuf::from)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_paths_are_consistent() {
        let store = ArtifactStore::new("/tmp/boaring-test");
        assert_eq!(
            store.objects_dir(),
            PathBuf::from("/tmp/boaring-test/objects")
        );
        assert_eq!(
            store.manifest_path("cid123"),
            PathBuf::from("/tmp/boaring-test/manifests/cid123")
        );
        assert_eq!(
            store.object_path("digest456"),
            PathBuf::from("/tmp/boaring-test/objects/digest456")
        );
    }
}
