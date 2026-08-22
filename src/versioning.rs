// Copyright (c) 2026 sal
// SPDX-License-Identifier: MIT

use crate::error::{BabyError, Result};
use std::cmp::Ordering;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Channel {
    Stable,
    Nightly,
    Bleeding,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
    pub prerelease: Option<String>,
}

impl Version {
    pub fn parse(version_str: &str) -> Result<Self> {
        let version_str = version_str.trim_start_matches('v');
        let (base, prerelease) = if let Some(idx) = version_str.find('-') {
            (
                &version_str[..idx],
                Some(version_str[idx + 1..].to_string()),
            )
        } else {
            (version_str, None)
        };

        let parts: Vec<&str> = base.split('.').collect();
        if parts.len() < 3 {
            return Err(BabyError::new(
                crate::error::ErrorKind::VersionCheck,
                format!("invalid version format: {}", version_str),
            ));
        }

        let major = parts[0].parse::<u32>().map_err(|_| {
            BabyError::new(
                crate::error::ErrorKind::VersionCheck,
                format!("invalid major version: {}", parts[0]),
            )
        })?;
        let minor = parts[1].parse::<u32>().map_err(|_| {
            BabyError::new(
                crate::error::ErrorKind::VersionCheck,
                format!("invalid minor version: {}", parts[1]),
            )
        })?;
        let patch = parts[2].parse::<u32>().map_err(|_| {
            BabyError::new(
                crate::error::ErrorKind::VersionCheck,
                format!("invalid patch version: {}", parts[2]),
            )
        })?;

        Ok(Version {
            major,
            minor,
            patch,
            prerelease,
        })
    }

    pub fn current() -> Self {
        const VERSION: &str = env!("CARGO_PKG_VERSION");
        Version::parse(VERSION).expect("cargo version is always valid")
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)?;
        if let Some(ref prerelease) = self.prerelease {
            write!(f, "-{}", prerelease)?;
        }
        Ok(())
    }
}

impl PartialOrd for Version {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Version {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.major.cmp(&other.major) {
            Ordering::Equal => match self.minor.cmp(&other.minor) {
                Ordering::Equal => match self.patch.cmp(&other.patch) {
                    Ordering::Equal => {
                        match (&self.prerelease, &other.prerelease) {
                            (None, None) => Ordering::Equal,
                            (None, Some(_)) => Ordering::Greater,
                            (Some(_), None) => Ordering::Less,
                            (Some(a), Some(b)) => a.cmp(b),
                        }
                    }
                    other => other,
                },
                other => other,
            },
            other => other,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_simple_version() {
        let v = Version::parse("1.2.3").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
        assert_eq!(v.prerelease, None);
    }

    #[test]
    fn parse_version_with_v_prefix() {
        let v = Version::parse("v1.2.3").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 2);
        assert_eq!(v.patch, 3);
    }

    #[test]
    fn parse_prerelease_version() {
        let v = Version::parse("1.0.0-alpha").unwrap();
        assert_eq!(v.major, 1);
        assert_eq!(v.minor, 0);
        assert_eq!(v.patch, 0);
        assert_eq!(v.prerelease, Some("alpha".to_string()));
    }

    #[test]
    fn version_ordering() {
        let v1 = Version::parse("1.0.0").unwrap();
        let v2 = Version::parse("1.0.1").unwrap();
        let v3 = Version::parse("1.1.0").unwrap();
        let v4 = Version::parse("2.0.0").unwrap();

        assert!(v1 < v2);
        assert!(v2 < v3);
        assert!(v3 < v4);
    }

    #[test]
    fn prerelease_is_less_than_release() {
        let v1 = Version::parse("1.0.0-alpha").unwrap();
        let v2 = Version::parse("1.0.0").unwrap();
        assert!(v1 < v2);
    }

    #[test]
    fn version_to_string() {
        let v = Version::parse("1.2.3").unwrap();
        assert_eq!(v.to_string(), "1.2.3");

        let v = Version::parse("1.0.0-beta").unwrap();
        assert_eq!(v.to_string(), "1.0.0-beta");
    }
}
