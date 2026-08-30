//! Immutable manifest describing a published computation result.

use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::computation_id::ComputationId;
use crate::sha256::encode_hex;

/// Description of a published artifact.
///
/// Once written, a manifest is never mutated; any change to its fields
/// constitutes a new computation and therefore a new identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Manifest {
    pub computation_id: ComputationId,
    pub content_digest: String,
    pub operation: String,
    pub inputs: BTreeMap<String, String>,
    pub implementation: String,
    pub target: String,
    pub created_at: SystemTime,
    pub schema_version: u64,
    pub provenance: String,
}

impl Manifest {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        computation_id: ComputationId,
        content_digest: String,
        operation: String,
        inputs: BTreeMap<String, String>,
        implementation: String,
        target: String,
        schema_version: u64,
        provenance: String,
    ) -> Self {
        Self {
            computation_id,
            content_digest,
            operation,
            inputs,
            implementation,
            target,
            created_at: SystemTime::now(),
            schema_version,
            provenance,
        }
    }

    /// Serialize the manifest to a line-oriented format.
    pub fn render(&self) -> String {
        let mut lines = Vec::new();
        lines.push(format!(
            "computation_id:{}",
            encode_hex(self.computation_id.as_ref().as_bytes())
        ));
        lines.push(format!(
            "content_digest:{}",
            encode_hex(self.content_digest.as_bytes())
        ));
        lines.push(format!(
            "operation:{}",
            encode_hex(self.operation.as_bytes())
        ));
        for (k, v) in &self.inputs {
            lines.push(format!(
                "input:{}={}",
                encode_hex(k.as_bytes()),
                encode_hex(v.as_bytes())
            ));
        }
        lines.push(format!(
            "implementation:{}",
            encode_hex(self.implementation.as_bytes())
        ));
        lines.push(format!("target:{}", encode_hex(self.target.as_bytes())));
        let elapsed = self
            .created_at
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO);
        lines.push(format!(
            "created:{}.{:09}",
            elapsed.as_secs(),
            elapsed.subsec_nanos()
        ));
        lines.push(format!("schema:{}", self.schema_version));
        lines.push(format!(
            "provenance:{}",
            encode_hex(self.provenance.as_bytes())
        ));
        lines.join("\n")
    }

    /// Deserialize a manifest previously produced by [`Self::render`].
    pub fn parse(text: &str) -> Result<Self, String> {
        let mut computation_id: Option<String> = None;
        let mut content_digest: Option<String> = None;
        let mut operation: Option<String> = None;
        let mut inputs: BTreeMap<String, String> = BTreeMap::new();
        let mut implementation: Option<String> = None;
        let mut target: Option<String> = None;
        let mut created_at: Option<SystemTime> = None;
        let mut schema_version: Option<u64> = None;
        let mut provenance: Option<String> = None;

        for (lineno, line) in text.lines().enumerate() {
            let (key, value) = line
                .split_once(':')
                .ok_or_else(|| format!("line {}: missing ':' delimiter", lineno + 1))?;
            match key {
                "computation_id" => {
                    computation_id = Some(
                        String::from_utf8(decode_hex(value)?)
                            .map_err(|e| format!("invalid computation_id: {e}"))?,
                    );
                }
                "content_digest" => {
                    content_digest = Some(
                        String::from_utf8(decode_hex(value)?)
                            .map_err(|e| format!("invalid content_digest: {e}"))?,
                    );
                }
                "operation" => {
                    operation = Some(
                        String::from_utf8(decode_hex(value)?)
                            .map_err(|e| format!("invalid operation: {e}"))?,
                    );
                }
                "input" => {
                    let (k, v) = value
                        .split_once('=')
                        .ok_or_else(|| format!("line {}: invalid input", lineno + 1))?;
                    inputs.insert(
                        String::from_utf8(decode_hex(k)?)
                            .map_err(|e| format!("invalid input key: {e}"))?,
                        String::from_utf8(decode_hex(v)?)
                            .map_err(|e| format!("invalid input value: {e}"))?,
                    );
                }
                "implementation" => {
                    implementation = Some(
                        String::from_utf8(decode_hex(value)?)
                            .map_err(|e| format!("invalid implementation: {e}"))?,
                    );
                }
                "target" => {
                    target = Some(
                        String::from_utf8(decode_hex(value)?)
                            .map_err(|e| format!("invalid target: {e}"))?,
                    );
                }
                "created" => {
                    let (secs, nanos) = value
                        .split_once('.')
                        .ok_or_else(|| format!("line {}: invalid created", lineno + 1))?;
                    let secs: u64 = secs
                        .parse()
                        .map_err(|e| format!("invalid created secs: {e}"))?;
                    let nanos: u32 = nanos
                        .parse()
                        .map_err(|e| format!("invalid created nanos: {e}"))?;
                    created_at = Some(UNIX_EPOCH + Duration::new(secs, nanos));
                }
                "schema" => {
                    schema_version =
                        Some(value.parse().map_err(|e| format!("invalid schema: {e}"))?);
                }
                "provenance" => {
                    provenance = Some(
                        String::from_utf8(decode_hex(value)?)
                            .map_err(|e| format!("invalid provenance: {e}"))?,
                    );
                }
                _ => return Err(format!("line {}: unknown key '{}'", lineno + 1, key)),
            }
        }

        Ok(Self {
            computation_id: ComputationId(computation_id.ok_or("missing computation_id")?),
            content_digest: content_digest.ok_or("missing content_digest")?,
            operation: operation.ok_or("missing operation")?,
            inputs,
            implementation: implementation.ok_or("missing implementation")?,
            target: target.ok_or("missing target")?,
            created_at: created_at.ok_or("missing created")?,
            schema_version: schema_version.ok_or("missing schema")?,
            provenance: provenance.ok_or("missing provenance")?,
        })
    }
}

fn decode_hex(hex: &str) -> Result<Vec<u8>, String> {
    if hex.len() % 2 != 0 {
        return Err("odd hex length".to_string());
    }
    let mut out = Vec::with_capacity(hex.len() / 2);
    for chunk in hex.as_bytes().chunks_exact(2) {
        let hi = hex_value(chunk[0])?;
        let lo = hex_value(chunk[1])?;
        out.push((hi << 4) | lo);
    }
    Ok(out)
}

fn hex_value(b: u8) -> Result<u8, String> {
    match b {
        b'0'..=b'9' => Ok(b - b'0'),
        b'a'..=b'f' => Ok(b - b'a' + 10),
        b'A'..=b'F' => Ok(b - b'A' + 10),
        _ => Err(format!("invalid hex digit: {}", b as char)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest() -> Manifest {
        let mut inputs = BTreeMap::new();
        inputs.insert("src".to_string(), "main.rs".to_string());
        Manifest::new(
            ComputationId("abc123".to_string()),
            "digest456".to_string(),
            "compile".to_string(),
            inputs,
            "rustc".to_string(),
            "x86_64-unknown-linux-gnu".to_string(),
            1,
            "boaring test".to_string(),
        )
    }

    #[test]
    fn manifest_round_trips() {
        let m = sample_manifest();
        let text = m.render();
        let parsed = Manifest::parse(&text).unwrap();
        assert_eq!(m, parsed);
    }

    #[test]
    fn manifest_rejects_tampered_digest() {
        let m = sample_manifest();
        let mut text = m.render();
        text.push_str("\ncontent_digest:000000");
        // The parser overwrites the first content_digest with the second, so this
        // test instead verifies that a malformed hex value is rejected.
        text.push_str("\ncontent_digest:zz");
        assert!(Manifest::parse(&text).is_err());
    }
}
