//! Deploy lifecycle tracing for Cordial Miners adapter.
//!
//! This module tracks the end-to-end journey of a user deploy from its first
//! observation at the ingress proxy through to final inclusion in a finalized
//! ordered output.
//!
//! ## Lifecycle States
//!
//! ```text
//! Observed  ──►  Accepted  ──►  BlockIncluded  ──►  FinalizedOrdered
//!   │               │                │                     │
//! Seen by        f1r3node        Deploy sig          Block B appears
//! ingress        returned        found in a          in ordered tau
//! proxy          DeployId        mirrored block      output
//! ```
//!
//! ## Design principles
//!
//! - **Non-intrusive**: all tracing is adapter-side only; no f1r3node source
//!   is modified.
//! - **Signature-keyed**: deploys are uniquely identified by their raw
//!   signature bytes, matching how f1r3node identifies them (its `DeployId`
//!   is literally the sig bytes).
//! - **Additive**: transitions are monotonically advancing — once a deploy
//!   reaches `FinalizedOrdered` it stays there regardless of further polls.
//! - **Thread-safe**: [`DeployTracer`] wraps its inner map in an
//!   `Arc<Mutex<...>>` so it can be cheaply cloned and shared across tasks.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};

// ────────────────────────────────────────────────────────────────────────────
// State machine
// ────────────────────────────────────────────────────────────────────────────

/// The four lifecycle stages a traced deploy can occupy.
///
/// States are totally ordered (`Observed < Accepted < BlockIncluded <
/// FinalizedOrdered`) and always advance monotonically; a deploy never
/// regresses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum DeployTraceState {
    /// The deploy signature was observed by the ingress proxy (gRPC or HTTP)
    /// but has not yet been confirmed accepted by f1r3node.
    Observed,

    /// f1r3node returned a valid `DeployId` response for this deploy signature,
    /// indicating the node accepted it into its deploy pool.
    Accepted,

    /// The deploy signature was found inside the body of a mirrored
    /// `BlockMessage` at a specific block height and block hash.
    ///
    /// The block has been ingested into the local blocklace mirror but may
    /// not yet be finalized.
    BlockIncluded,

    /// The block containing this deploy appears in a `OrderedFinalizedOutput`
    /// under a finalized leader anchor, meaning the deploy is fully committed
    /// according to Cordial consensus.
    FinalizedOrdered,
}

impl std::fmt::Display for DeployTraceState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Observed => write!(f, "Observed"),
            Self::Accepted => write!(f, "Accepted"),
            Self::BlockIncluded => write!(f, "BlockIncluded"),
            Self::FinalizedOrdered => write!(f, "FinalizedOrdered"),
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Report type
// ────────────────────────────────────────────────────────────────────────────

/// Ingress source through which a deploy was first observed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum TraceIngressSource {
    Grpc,
    Http,
    Unknown,
}

impl std::fmt::Display for TraceIngressSource {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Grpc => write!(f, "gRPC"),
            Self::Http => write!(f, "HTTP"),
            Self::Unknown => write!(f, "unknown"),
        }
    }
}

/// A point-in-time snapshot of a traced deploy's lifecycle.
///
/// Returned by [`DeployTracer::get_deploy_trace`] and
/// [`DeployTracer::list_active_traces`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DeployTraceReport {
    /// The raw signature bytes that uniquely identify this deploy.
    /// Also the `DeployId` as used by f1r3node.
    pub signature_hex: String,

    /// Current lifecycle state.
    pub state: DeployTraceState,

    /// Wall-clock time (Unix seconds) when the deploy was first observed.
    pub observed_at_secs: u64,

    /// Ingress source (gRPC / HTTP) of the first observation.
    pub ingress_source: TraceIngressSource,

    /// Wall-clock time when the `Accepted` state was entered, if known.
    pub accepted_at_secs: Option<u64>,

    /// Block hash (hex) of the block that included this deploy, if known.
    pub block_hash_hex: Option<String>,

    /// Block height at which the deploy was included, if known.
    pub block_height: Option<i64>,

    /// Wall-clock time when the `BlockIncluded` state was entered, if known.
    pub included_at_secs: Option<u64>,

    /// Anchor block hash (hex) of the finalized ordered output that contained
    /// the block, if the deploy has been finalized.
    pub finalized_anchor_hex: Option<String>,

    /// Wall-clock time when the `FinalizedOrdered` state was entered, if known.
    pub finalized_at_secs: Option<u64>,
}

impl DeployTraceReport {
    /// Elapsed time in seconds from first observation to `FinalizedOrdered`,
    /// or from first observation to "now" if not yet finalized.
    pub fn elapsed_secs(&self) -> u64 {
        let now = now_secs();
        match self.finalized_at_secs {
            Some(fin) => fin.saturating_sub(self.observed_at_secs),
            None => now.saturating_sub(self.observed_at_secs),
        }
    }

    /// Return `true` if this deploy has reached the terminal lifecycle state.
    pub fn is_finalized(&self) -> bool {
        self.state == DeployTraceState::FinalizedOrdered
    }

    /// Pretty-print a concise one-line summary of the current lifecycle status.
    ///
    /// Example output:
    /// ```text
    /// [FinalizedOrdered] sig=0x1234…abcd  block=0xdead… @height=42  anchor=0xcafe…  +5s
    /// ```
    pub fn summary_line(&self) -> String {
        let sig_short = short_hex(&self.signature_hex, 8);
        let elapsed = self.elapsed_secs();

        match self.state {
            DeployTraceState::Observed => {
                format!(
                    "[Observed]          sig=0x{sig_short}…  source={}  +{}s",
                    self.ingress_source, elapsed
                )
            }
            DeployTraceState::Accepted => {
                format!("[Accepted]          sig=0x{sig_short}…  +{}s", elapsed)
            }
            DeployTraceState::BlockIncluded => {
                let bh = self
                    .block_hash_hex
                    .as_deref()
                    .map(|h| short_hex(h, 8))
                    .unwrap_or_default();
                let height = self.block_height.unwrap_or(-1);
                format!(
                    "[BlockIncluded]     sig=0x{sig_short}…  block=0x{bh}… @height={height}  +{}s",
                    elapsed
                )
            }
            DeployTraceState::FinalizedOrdered => {
                let bh = self
                    .block_hash_hex
                    .as_deref()
                    .map(|h| short_hex(h, 8))
                    .unwrap_or_default();
                let anchor = self
                    .finalized_anchor_hex
                    .as_deref()
                    .map(|h| short_hex(h, 8))
                    .unwrap_or_default();
                let height = self.block_height.unwrap_or(-1);
                format!(
                    "[FinalizedOrdered]  sig=0x{sig_short}…  block=0x{bh}… @height={height}  anchor=0x{anchor}…  +{}s",
                    elapsed
                )
            }
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Internal state per traced deploy
// ────────────────────────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct TraceEntry {
    signature_hex: String,
    state: DeployTraceState,
    observed_at_secs: u64,
    ingress_source: TraceIngressSource,
    accepted_at_secs: Option<u64>,
    block_hash_hex: Option<String>,
    block_height: Option<i64>,
    included_at_secs: Option<u64>,
    finalized_anchor_hex: Option<String>,
    finalized_at_secs: Option<u64>,
}

impl TraceEntry {
    fn new(sig_hex: String, source: TraceIngressSource) -> Self {
        Self {
            signature_hex: sig_hex,
            state: DeployTraceState::Observed,
            observed_at_secs: now_secs(),
            ingress_source: source,
            accepted_at_secs: None,
            block_hash_hex: None,
            block_height: None,
            included_at_secs: None,
            finalized_anchor_hex: None,
            finalized_at_secs: None,
        }
    }

    fn to_report(&self) -> DeployTraceReport {
        DeployTraceReport {
            signature_hex: self.signature_hex.clone(),
            state: self.state,
            observed_at_secs: self.observed_at_secs,
            ingress_source: self.ingress_source,
            accepted_at_secs: self.accepted_at_secs,
            block_hash_hex: self.block_hash_hex.clone(),
            block_height: self.block_height,
            included_at_secs: self.included_at_secs,
            finalized_anchor_hex: self.finalized_anchor_hex.clone(),
            finalized_at_secs: self.finalized_at_secs,
        }
    }

    /// Advance to `Accepted`; no-op if already beyond.
    fn advance_accepted(&mut self) {
        if self.state < DeployTraceState::Accepted {
            self.state = DeployTraceState::Accepted;
            self.accepted_at_secs = Some(now_secs());
        }
    }

    /// Advance to `BlockIncluded`; no-op if already beyond.
    fn advance_block_included(&mut self, block_hash_hex: String, block_height: i64) {
        if self.state < DeployTraceState::BlockIncluded {
            // Also mark as accepted if we somehow missed that transition.
            if self.accepted_at_secs.is_none() {
                self.accepted_at_secs = Some(now_secs());
            }
            self.state = DeployTraceState::BlockIncluded;
            self.block_hash_hex = Some(block_hash_hex);
            self.block_height = Some(block_height);
            self.included_at_secs = Some(now_secs());
        }
    }

    /// Advance to `FinalizedOrdered`; no-op if already there.
    fn advance_finalized(&mut self, anchor_hex: String) {
        if self.state < DeployTraceState::FinalizedOrdered {
            self.state = DeployTraceState::FinalizedOrdered;
            self.finalized_anchor_hex = Some(anchor_hex);
            self.finalized_at_secs = Some(now_secs());
        }
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Public tracer
// ────────────────────────────────────────────────────────────────────────────

/// Shared, cheaply-cloneable deploy lifecycle tracer.
///
/// Maintains a map of `signature_hex → TraceEntry` and exposes methods to
/// advance each entry through the four lifecycle states. The inner map is
/// guarded by a `Mutex` so that `DeployTracer` can be cloned into multiple
/// tasks (e.g., the ingress handler task and the mirror polling task) without
/// coordinating ownership.
///
/// ## Usage
///
/// ```no_run
/// use cordial_f1r3node_adapter::deploy_trace::{DeployTracer, TraceIngressSource};
///
/// let tracer = DeployTracer::new();
///
/// // 1. On ingress observation:
/// tracer.record_observed(&[0xAB; 64], TraceIngressSource::Grpc);
///
/// // 2. On f1r3node acceptance:
/// tracer.record_accepted(&[0xAB; 64]);
///
/// // 3. On block inclusion (from block body scanning):
/// let block_hash = vec![0xDE; 32];
/// tracer.record_block_included(&[0xAB; 64], &block_hash, 42);
///
/// // 4. On finalized ordered output:
/// let anchor = vec![0xCA; 32];
/// tracer.record_finalized(&[0xAB; 64], &anchor);
///
/// // Query:
/// if let Some(report) = tracer.get_deploy_trace(&[0xAB; 64]) {
///     println!("{}", report.summary_line());
/// }
/// ```
#[derive(Debug, Clone, Default)]
pub struct DeployTracer {
    inner: Arc<Mutex<HashMap<String, TraceEntry>>>,
}

impl DeployTracer {
    /// Create a new empty tracer.
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    // ── Transition methods ────────────────────────────────────────────────

    /// Record that a deploy with `sig` was observed at the ingress boundary.
    ///
    /// Creates a new trace entry in the `Observed` state. If an entry for
    /// this signature already exists (e.g., seen on both HTTP and gRPC), the
    /// existing entry is left unchanged (observation idempotency).
    pub fn record_observed(&self, sig: &[u8], source: TraceIngressSource) {
        let sig_hex = hex::encode(sig);
        let mut map = self.inner.lock().unwrap();
        map.entry(sig_hex.clone())
            .or_insert_with(|| TraceEntry::new(sig_hex, source));
    }

    /// Advance the trace for `sig` to `Accepted`.
    ///
    /// No-op if no entry exists yet or if the entry is already beyond
    /// `Accepted`. Creates an `Observed` entry implicitly if `sig` is new
    /// (covers the edge-case where the tracer was not attached to ingress).
    pub fn record_accepted(&self, sig: &[u8]) {
        let sig_hex = hex::encode(sig);
        let mut map = self.inner.lock().unwrap();
        let entry = map
            .entry(sig_hex.clone())
            .or_insert_with(|| TraceEntry::new(sig_hex, TraceIngressSource::Unknown));
        entry.advance_accepted();
    }

    /// Advance the trace for `sig` to `BlockIncluded`.
    ///
    /// `block_hash` is the 32-byte content hash of the containing block.
    /// `block_height` is `F1r3flyState::block_number` from the block body.
    pub fn record_block_included(&self, sig: &[u8], block_hash: &[u8], block_height: i64) {
        let sig_hex = hex::encode(sig);
        let block_hash_hex = hex::encode(block_hash);
        let mut map = self.inner.lock().unwrap();
        let entry = map
            .entry(sig_hex.clone())
            .or_insert_with(|| TraceEntry::new(sig_hex, TraceIngressSource::Unknown));
        entry.advance_block_included(block_hash_hex, block_height);
    }

    /// Advance the trace for `sig` to `FinalizedOrdered`.
    ///
    /// `anchor_hash` is the content hash of the finalized leader block that
    /// produced the ordered output in which `sig`'s containing block appears.
    pub fn record_finalized(&self, sig: &[u8], anchor_hash: &[u8]) {
        let sig_hex = hex::encode(sig);
        let anchor_hex = hex::encode(anchor_hash);
        let mut map = self.inner.lock().unwrap();
        let entry = map
            .entry(sig_hex.clone())
            .or_insert_with(|| TraceEntry::new(sig_hex, TraceIngressSource::Unknown));
        entry.advance_finalized(anchor_hex);
    }

    // ── Query methods ─────────────────────────────────────────────────────

    /// Retrieve the current lifecycle report for a deploy signature.
    ///
    /// Returns `None` if the signature has never been registered with the
    /// tracer.
    pub fn get_deploy_trace(&self, sig: &[u8]) -> Option<DeployTraceReport> {
        let sig_hex = hex::encode(sig);
        let map = self.inner.lock().unwrap();
        map.get(&sig_hex).map(|e| e.to_report())
    }

    /// List reports for all currently tracked deploys, regardless of state.
    ///
    /// Reports are returned in arbitrary order (HashMap iteration order).
    /// Callers that need a stable order should sort by `observed_at_secs`.
    pub fn list_active_traces(&self) -> Vec<DeployTraceReport> {
        let map = self.inner.lock().unwrap();
        map.values().map(|e| e.to_report()).collect()
    }

    /// List only deploys that have not yet reached `FinalizedOrdered`.
    pub fn list_pending_traces(&self) -> Vec<DeployTraceReport> {
        let map = self.inner.lock().unwrap();
        map.values()
            .filter(|e| e.state != DeployTraceState::FinalizedOrdered)
            .map(|e| e.to_report())
            .collect()
    }

    /// Total number of tracked deploy signatures (all states).
    pub fn len(&self) -> usize {
        self.inner.lock().unwrap().len()
    }

    /// Return `true` if no deploys have been registered with this tracer.
    pub fn is_empty(&self) -> bool {
        self.inner.lock().unwrap().is_empty()
    }

    // ── Bulk correlation helpers ──────────────────────────────────────────

    /// Scan a block's deploy signatures and advance any matching traces to
    /// `BlockIncluded`.
    ///
    /// `deploy_sigs` — iterator of raw signature byte slices found in the
    ///   block body (from `ProcessedDeploy::deploy.sig`).\
    /// `block_hash` — 32-byte content hash of the block.\
    /// `block_height` — `F1r3flyState::block_number` from the block body.
    ///
    /// Returns the count of traces that were advanced.
    pub fn correlate_block<'a>(
        &self,
        deploy_sigs: impl Iterator<Item = &'a [u8]>,
        block_hash: &[u8],
        block_height: i64,
    ) -> usize {
        let block_hash_hex = hex::encode(block_hash);
        let mut map = self.inner.lock().unwrap();
        let mut advanced = 0usize;

        for sig in deploy_sigs {
            let sig_hex = hex::encode(sig);
            if let Some(entry) = map.get_mut(&sig_hex)
                && entry.state < DeployTraceState::BlockIncluded
            {
                entry.advance_block_included(block_hash_hex.clone(), block_height);
                advanced += 1;
            }
        }

        advanced
    }

    /// Scan a finalized ordered output and advance any traces whose
    /// `block_hash_hex` matches a block in the output to `FinalizedOrdered`.
    ///
    /// `finalized_block_hashes` — hex-encoded content hashes of all blocks
    ///   in the ordered output.\
    /// `anchor_hash` — raw bytes of the finalized leader anchor block.
    ///
    /// Returns the count of traces that were advanced.
    pub fn correlate_finalized_output(
        &self,
        finalized_block_hashes: &[String],
        anchor_hash: &[u8],
    ) -> usize {
        let anchor_hex = hex::encode(anchor_hash);
        let finalized_set: std::collections::HashSet<&str> =
            finalized_block_hashes.iter().map(|s| s.as_str()).collect();

        let mut map = self.inner.lock().unwrap();
        let mut advanced = 0usize;

        for entry in map.values_mut() {
            if entry.state == DeployTraceState::BlockIncluded
                && let Some(bh) = &entry.block_hash_hex
                && finalized_set.contains(bh.as_str())
            {
                entry.advance_finalized(anchor_hex.clone());
                advanced += 1;
            }
        }

        advanced
    }
}

// ────────────────────────────────────────────────────────────────────────────
// Helpers
// ────────────────────────────────────────────────────────────────────────────

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

/// Return the first `n` chars of a hex string (without `0x` prefix).
fn short_hex(hex: &str, n: usize) -> String {
    hex.chars().take(n).collect()
}

// ────────────────────────────────────────────────────────────────────────────
// Unit tests
// ────────────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn sig(b: u8) -> Vec<u8> {
        vec![b; 64]
    }

    fn block_hash(b: u8) -> Vec<u8> {
        vec![b; 32]
    }

    fn anchor(b: u8) -> Vec<u8> {
        vec![b; 32]
    }

    #[test]
    fn new_tracer_is_empty() {
        let t = DeployTracer::new();
        assert!(t.is_empty());
        assert_eq!(t.len(), 0);
    }

    #[test]
    fn observe_creates_entry_in_observed_state() {
        let t = DeployTracer::new();
        t.record_observed(&sig(1), TraceIngressSource::Grpc);

        let report = t.get_deploy_trace(&sig(1)).unwrap();
        assert_eq!(report.state, DeployTraceState::Observed);
        assert_eq!(report.ingress_source, TraceIngressSource::Grpc);
        assert!(report.accepted_at_secs.is_none());
        assert!(report.block_hash_hex.is_none());
    }

    #[test]
    fn full_lifecycle_transitions() {
        let t = DeployTracer::new();
        let s = sig(2);
        let bh = block_hash(0xAB);
        let anc = anchor(0xCD);

        t.record_observed(&s, TraceIngressSource::Http);
        assert_eq!(
            t.get_deploy_trace(&s).unwrap().state,
            DeployTraceState::Observed
        );

        t.record_accepted(&s);
        assert_eq!(
            t.get_deploy_trace(&s).unwrap().state,
            DeployTraceState::Accepted
        );

        t.record_block_included(&s, &bh, 10);
        let rep = t.get_deploy_trace(&s).unwrap();
        assert_eq!(rep.state, DeployTraceState::BlockIncluded);
        assert_eq!(rep.block_height, Some(10));
        assert_eq!(rep.block_hash_hex, Some(hex::encode(&bh)));

        t.record_finalized(&s, &anc);
        let rep = t.get_deploy_trace(&s).unwrap();
        assert_eq!(rep.state, DeployTraceState::FinalizedOrdered);
        assert_eq!(rep.finalized_anchor_hex, Some(hex::encode(&anc)));
        assert!(rep.is_finalized());
    }

    #[test]
    fn states_are_monotonic_no_regression() {
        let t = DeployTracer::new();
        let s = sig(3);

        t.record_accepted(&s); // skips Observed
        assert_eq!(
            t.get_deploy_trace(&s).unwrap().state,
            DeployTraceState::Accepted
        );

        // Calling record_observed after accepted must not regress
        t.record_observed(&s, TraceIngressSource::Grpc);
        assert_eq!(
            t.get_deploy_trace(&s).unwrap().state,
            DeployTraceState::Accepted
        );
    }

    #[test]
    fn correlate_block_advances_matching_traces() {
        let t = DeployTracer::new();
        let s = sig(4);
        t.record_observed(&s, TraceIngressSource::Grpc);

        let bh = block_hash(0xAB);
        let sigs: Vec<Vec<u8>> = vec![sig(4), sig(5)]; // sig(5) not tracked
        let advanced = t.correlate_block(sigs.iter().map(|v| v.as_slice()), &bh, 7);

        assert_eq!(advanced, 1);
        assert_eq!(
            t.get_deploy_trace(&s).unwrap().state,
            DeployTraceState::BlockIncluded
        );
    }

    #[test]
    fn correlate_finalized_output_advances_block_included_traces() {
        let t = DeployTracer::new();
        let s = sig(5);
        let bh = block_hash(0xBB);
        let anc = anchor(0xCC);

        t.record_observed(&s, TraceIngressSource::Http);
        t.record_block_included(&s, &bh, 99);

        let finalized_hashes = vec![hex::encode(&bh)];
        let advanced = t.correlate_finalized_output(&finalized_hashes, &anc);

        assert_eq!(advanced, 1);
        assert_eq!(
            t.get_deploy_trace(&s).unwrap().state,
            DeployTraceState::FinalizedOrdered
        );
    }

    #[test]
    fn list_pending_excludes_finalized() {
        let t = DeployTracer::new();
        t.record_observed(&sig(6), TraceIngressSource::Grpc);
        t.record_observed(&sig(7), TraceIngressSource::Http);
        t.record_finalized(&sig(7), &anchor(0xDD));

        let pending = t.list_pending_traces();
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].signature_hex, hex::encode(sig(6)));
    }

    #[test]
    fn summary_line_finalized_contains_expected_fragments() {
        let t = DeployTracer::new();
        let s = sig(8);
        let bh = block_hash(0xEE);
        let anc = anchor(0xFF);

        t.record_observed(&s, TraceIngressSource::Grpc);
        t.record_accepted(&s);
        t.record_block_included(&s, &bh, 55);
        t.record_finalized(&s, &anc);

        let line = t.get_deploy_trace(&s).unwrap().summary_line();
        assert!(line.contains("[FinalizedOrdered]"), "got: {line}");
        assert!(line.contains("@height=55"), "got: {line}");
        assert!(line.contains("anchor="), "got: {line}");
    }
}
