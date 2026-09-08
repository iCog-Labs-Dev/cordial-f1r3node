//! Trace emitter for Cordial Miners consensus decisions.
//!
//! When compiled without the `trace` feature this module compiles away to nothing.
//! When enabled, each event is serialized as a newline-delimited JSON record and
//! appended to `CORDIAL_TRACE_FILE`, or written to stderr if that variable is unset.
//!
//! Schema changes here must be mirrored in `lean/LeanVerification/Trace.lean`.

use serde::{Deserialize, Serialize};

// Internally tagged enums are deserialized through Serde's intermediate
// `Content` value, whose default u128 entry point rejects JSON numbers even
// when they fit in u64.  Decode through `deserialize_any` so certificate
// weights remain numeric JSON and the Rust schema still round-trips.
fn deserialize_u128_number<'de, D>(deserializer: D) -> Result<u128, D::Error>
where
    D: serde::Deserializer<'de>,
{
    struct U128Visitor;

    impl<'de> serde::de::Visitor<'de> for U128Visitor {
        type Value = u128;

        fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            formatter.write_str("a non-negative integer")
        }

        fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
            Ok(u128::from(value))
        }

        fn visit_u128<E>(self, value: u128) -> Result<Self::Value, E> {
            Ok(value)
        }

        fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E>
        where
            E: serde::de::Error,
        {
            u128::try_from(value).map_err(E::custom)
        }
    }

    deserializer.deserialize_any(U128Visitor)
}

// Event catalogue

/// A safety-relevant consensus event.
///
/// Serialized with `#[serde(tag = "event")]` so every JSON object carries an
/// `"event"` discriminant field. Required `Option` fields serialize as JSON
/// `null` when absent, preserving the distinction between null and omission.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "event", rename_all = "snake_case")]
pub enum TraceEvent {
    // Block lifecycle
    CreateBlock(BlockLifecycleEvent),
    ValidateBlock(ValidateBlockEvent),
    InsertBlock(BlockLifecycleEvent),
    /// Emitted when a predecessor is missing; block is not inserted.
    BufferBlock(BlockLifecycleEvent),
    ResolveMissingParent(ResolveMissingParentEvent),

    // Equivocation
    DetectEquivocation(DetectEquivocationEvent),

    // Approval and threshold certificates
    AcceptApproval(AcceptApprovalEvent),
    BuildThresholdCertificate(ThresholdCertificateEvent),

    // Finality
    ComputeFinality(ComputeFinalityEvent),

    // Ordering
    RunTauOrder(TauOrderEvent),
    EmitOutput(EmitOutputEvent),

    // Dissemination
    SendPackage(PackageEvent),
    DeliverPackage(PackageEvent),

    // Scheduler
    SchedulerTick(SchedulerTickEvent),
    RunWaveTask(WaveTaskEvent),
}

// Per-event payload structs

/// Shared fields for block-lifecycle events (Create / Insert / Buffer).
///
/// `node_id` is the node/context performing the lifecycle action. `creator` is
/// always the author of the block and may differ from the actor. The generic
/// `Blocklace` commit path has no observer parameter and therefore uses the
/// creator as its commit actor; node-owned paths provide the local node.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BlockLifecycleEvent {
    pub node_id: String,
    pub wave: Option<u64>,
    /// `None` when missing predecessors make the DAG depth unknowable.
    pub round: Option<u64>,
    pub block_hash: String,
    pub parent_hashes: Vec<String>,
    /// The complete subset of `parent_hashes` not yet available locally.
    pub missing_parent_hashes: Vec<String>,
    /// Block author (= NodeId bytes, hex-encoded).
    pub creator: String,
    /// FNV-1a-64 over the canonical sorted weight table; `None` when the
    /// operation has no weight context.
    pub weight_table_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ValidateBlockEvent {
    pub node_id: String,
    pub wave: Option<u64>,
    /// `None` when validation is deferred for missing predecessors.
    pub round: Option<u64>,
    pub block_hash: String,
    pub parent_hashes: Vec<String>,
    pub creator: String,
    pub weight_table_hash: Option<String>,
    /// `"valid"` | `"invalid"`
    pub outcome: String,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResolveMissingParentEvent {
    pub node_id: String,
    pub block_hash: String,
    pub resolved_parent_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DetectEquivocationEvent {
    /// Observer node.
    pub node_id: String,
    /// The validator that produced two incomparable blocks at the same round.
    pub equivocator: String,
    pub round: u64,
    pub conflicting_block_hashes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptApprovalEvent {
    /// Node evaluating/accepting this approval. It is normally the approver
    /// because the current approval API has no separate observer parameter.
    pub node_id: String,
    pub wave: Option<u64>,
    pub round: u64,
    pub approver: String,
    pub approver_hash: String,
    pub target_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThresholdCertificateEvent {
    pub node_id: String,
    pub wave: Option<u64>,
    /// `"ratification"` or `"super_ratification"`.
    pub kind: String,
    pub leader_hash: String,
    pub ratifier_hash: Option<String>,
    pub certificate_id: String,
    pub approver_hashes: Vec<String>,
    pub approvers: Vec<String>,
    pub approver_count: usize,
    #[serde(deserialize_with = "deserialize_u128_number")]
    pub approver_weight: u128,
    #[serde(deserialize_with = "deserialize_u128_number")]
    pub total_weight: u128,
    pub weight_table_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComputeFinalityEvent {
    pub node_id: String,
    pub wave: u64,
    pub wavelength: u64,
    pub block_hash: String,
    /// `"finalized"` | `"not_finalized"`
    pub decision: String,
    pub certificate_id: Option<String>,
    pub output_prefix_hash: Option<String>,
    /// `Some` for stake-weighted finality; `None` for paper-native unweighted
    /// finality, which has no validator weight-table context.
    pub weight_table_hash: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TauOrderEvent {
    pub node_id: String,
    pub wave: u64,
    pub wavelength: u64,
    pub latest_leader_hash: String,
    pub ordered_block_hashes: Vec<String>,
    pub output_len: usize,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmitOutputEvent {
    pub node_id: String,
    pub wave: u64,
    pub block_hash: String,
    pub output_index: usize,
    /// Running FNV-1a-64 hash of the output prefix after this block.
    pub output_prefix_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PackageEvent {
    pub node_id: String,
    pub peer_id: String,
    pub block_hashes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SchedulerTickEvent {
    pub node_id: String,
    /// Deterministic logical scheduler step (not wall-clock time).
    pub tick: u64,
    pub wave: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaveTaskEvent {
    pub node_id: String,
    pub wave: u64,
    /// `"propose"` | `"vote"` | `"finalize"`
    pub task: String,
}

// Emission helpers: the public API used by consensus modules

/// Serialize and write one complete event, reporting serialization/open/write
/// failures. This API exists only in trace-enabled builds.
#[cfg(feature = "trace")]
pub fn try_emit(event: &TraceEvent) -> std::io::Result<()> {
    use std::fs::OpenOptions;
    use std::io::Write;

    let line = serde_json::to_string(event).map_err(std::io::Error::other)?;
    if let Some(path) = std::env::var_os("CORDIAL_TRACE_FILE") {
        let mut file = OpenOptions::new().create(true).append(true).open(path)?;
        writeln!(file, "{line}")?;
        file.flush()
    } else {
        let mut stderr = std::io::stderr().lock();
        writeln!(stderr, "[TRACE] {line}")?;
        stderr.flush()
    }
}

/// Best-effort production tracing; conformance runs set
/// `CORDIAL_TRACE_STRICT=1` to fail immediately on a broken sink. An incomplete
/// canonical execution must never be mistaken for a successful capture.
#[cfg(feature = "trace")]
pub fn emit(event: TraceEvent) {
    let result = if std::env::var_os("CORDIAL_TRACE_FILE").is_none() {
        // Preserve libtest's stderr capture in ordinary trace-enabled tests.
        // Writing directly to stderr here would bypass capture and flood the
        // test runner with every consensus event, even for passing tests.
        serde_json::to_string(&event)
            .map(|line| eprintln!("[TRACE] {line}"))
            .map_err(std::io::Error::other)
    } else {
        try_emit(&event)
    };
    match result {
        Err(error)
            if std::env::var_os("CORDIAL_TRACE_STRICT").is_some_and(|value| value == "1") =>
        {
            panic!("TRACE EMISSION ERROR: {error}");
        }
        _ => {}
    }
}

/// No-op stub compiled when the `trace` feature is disabled.
#[cfg(not(feature = "trace"))]
#[inline(always)]
pub fn emit(_event: TraceEvent) {}

// Helpers

/// Hex-encode a byte slice into a lowercase hex string.
pub fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Canonically encode and sort block hashes.  Rust `HashSet` iteration order
/// is randomized, so every set-valued trace field must pass through this
/// helper before serialization.
pub fn sorted_block_hashes<'a>(
    identities: impl IntoIterator<Item = &'a crate::types::BlockIdentity>,
) -> Vec<String> {
    let mut hashes: Vec<String> = identities
        .into_iter()
        .map(|identity| hex(&identity.content_hash))
        .collect();
    hashes.sort();
    hashes
}

/// FNV-1a-64 fingerprint of a bond table using a documented canonical byte
/// encoding. The deliberately small algorithm is duplicated in Lean so replay
/// can recompute the fingerprint without trusting a native hashing FFI.
pub fn weight_table_hash(bonds: &std::collections::HashMap<crate::types::NodeId, u64>) -> String {
    let mut entries: Vec<_> = bonds.iter().collect();
    entries.sort_by_key(|(node, _)| node.0.as_slice());

    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for (node, weight) in entries {
        let row = format!("{}:{}\n", hex(&node.0), weight);
        for byte in row.bytes() {
            h ^= u64::from(byte);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("{h:016x}")
}

/// Stable evidence id for a threshold certificate. Evidence details remain in
/// the event; this id links `compute_finality` to the super-ratification event.
pub fn certificate_id(kind: &str, leader_hash: &str, ratifier_hash: Option<&str>) -> String {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for byte in kind
        .bytes()
        .chain([0xff])
        .chain(leader_hash.bytes())
        .chain([0xff])
        .chain(ratifier_hash.unwrap_or("").bytes())
    {
        h ^= u64::from(byte);
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

/// Canonical running hash for an ordered output prefix. Hashes are the
/// lowercase hexadecimal block hashes appearing in the JSON trace and are
/// separated by `0xff`, which cannot occur in their ASCII encoding.
pub fn output_prefix_hash(block_hashes: &[String]) -> String {
    let mut h = 0xcbf2_9ce4_8422_2325u64;
    for block_hash in block_hashes {
        for byte in block_hash.bytes().chain([0xff]) {
            h ^= u64::from(byte);
            h = h.wrapping_mul(0x0000_0100_0000_01b3);
        }
    }
    format!("{h:016x}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every variant must round-trip through JSON with the `"event"` tag intact.
    #[test]
    fn all_variants_serialize_with_event_tag() {
        let events: Vec<TraceEvent> = vec![
            TraceEvent::CreateBlock(BlockLifecycleEvent {
                node_id: "v1".into(),
                wave: Some(0),
                round: Some(0),
                block_hash: "aabb".into(),
                parent_hashes: vec![],
                missing_parent_hashes: vec![],
                creator: "deadbeef".into(),
                weight_table_hash: None,
            }),
            TraceEvent::ValidateBlock(ValidateBlockEvent {
                node_id: "v1".into(),
                wave: Some(0),
                round: Some(0),
                block_hash: "aabb".into(),
                parent_hashes: vec![],
                creator: "deadbeef".into(),
                weight_table_hash: None,
                outcome: "valid".into(),
                errors: vec![],
            }),
            TraceEvent::InsertBlock(BlockLifecycleEvent {
                node_id: "v1".into(),
                wave: Some(0),
                round: Some(0),
                block_hash: "aabb".into(),
                parent_hashes: vec![],
                missing_parent_hashes: vec![],
                creator: "deadbeef".into(),
                weight_table_hash: None,
            }),
            TraceEvent::BufferBlock(BlockLifecycleEvent {
                node_id: "v1".into(),
                wave: None,
                round: None,
                block_hash: "ccdd".into(),
                parent_hashes: vec!["aabb".into()],
                missing_parent_hashes: vec!["aabb".into()],
                creator: "deadbeef".into(),
                weight_table_hash: None,
            }),
            TraceEvent::ResolveMissingParent(ResolveMissingParentEvent {
                node_id: "v1".into(),
                block_hash: "ccdd".into(),
                resolved_parent_hash: "aabb".into(),
            }),
            TraceEvent::DetectEquivocation(DetectEquivocationEvent {
                node_id: "v2".into(),
                equivocator: "deadbeef".into(),
                round: 0,
                conflicting_block_hashes: vec!["aabb".into(), "1122".into()],
            }),
            TraceEvent::AcceptApproval(AcceptApprovalEvent {
                node_id: "v2".into(),
                wave: Some(1),
                round: 1,
                approver: "v2".into(),
                approver_hash: "ccdd".into(),
                target_hash: "aabb".into(),
            }),
            TraceEvent::BuildThresholdCertificate(ThresholdCertificateEvent {
                node_id: "v1".into(),
                wave: Some(1),
                kind: "super_ratification".into(),
                leader_hash: "aabb".into(),
                ratifier_hash: None,
                certificate_id: "cert01".into(),
                approver_hashes: vec!["ccdd".into()],
                approvers: vec!["v2".into()],
                approver_count: 1,
                approver_weight: 300,
                total_weight: 400,
                weight_table_hash: "weights01".into(),
            }),
            TraceEvent::ComputeFinality(ComputeFinalityEvent {
                node_id: "v1".into(),
                wave: 1,
                wavelength: 3,
                block_hash: "aabb".into(),
                decision: "finalized".into(),
                certificate_id: Some("cert01".into()),
                output_prefix_hash: Some("ffee".into()),
                weight_table_hash: Some("weights01".into()),
            }),
            TraceEvent::RunTauOrder(TauOrderEvent {
                node_id: "v1".into(),
                wave: 1,
                wavelength: 3,
                latest_leader_hash: "aabb".into(),
                ordered_block_hashes: vec!["aabb".into()],
                output_len: 1,
            }),
            TraceEvent::EmitOutput(EmitOutputEvent {
                node_id: "v1".into(),
                wave: 1,
                block_hash: "aabb".into(),
                output_index: 0,
                output_prefix_hash: "ffee".into(),
            }),
            TraceEvent::SendPackage(PackageEvent {
                node_id: "v1".into(),
                peer_id: "v2".into(),
                block_hashes: vec!["aabb".into()],
            }),
            TraceEvent::DeliverPackage(PackageEvent {
                node_id: "v2".into(),
                peer_id: "v1".into(),
                block_hashes: vec!["aabb".into()],
            }),
            TraceEvent::SchedulerTick(SchedulerTickEvent {
                node_id: "v1".into(),
                tick: 1,
                wave: Some(1),
            }),
            TraceEvent::RunWaveTask(WaveTaskEvent {
                node_id: "v1".into(),
                wave: 1,
                task: "propose".into(),
            }),
        ];

        for event in events {
            let json = serde_json::to_string(&event).expect("serialize failed");
            assert!(
                json.contains(r#""event""#),
                "Missing 'event' tag in: {}",
                json
            );
            let _: TraceEvent = serde_json::from_str(&json).expect("deserialize failed");
        }
    }

    #[test]
    fn hex_produces_lowercase_even_length_string() {
        let h = hex(&[0xde, 0xad, 0xbe, 0xef]);
        assert_eq!(h, "deadbeef");
        assert_eq!(h.len(), 8);
    }

    #[test]
    fn weight_table_hash_is_deterministic() {
        use crate::types::NodeId;
        use std::collections::HashMap;

        let mut bonds: HashMap<NodeId, u64> = HashMap::new();
        bonds.insert(NodeId(vec![1]), 100);
        bonds.insert(NodeId(vec![2]), 200);
        bonds.insert(NodeId(vec![3]), 300);

        let h1 = weight_table_hash(&bonds);
        let h2 = weight_table_hash(&bonds);
        assert_eq!(h1, h2);
        assert_eq!(h1.len(), 16);
    }
}
