//! Threshold certificates: the evidence behind a weighted quorum decision.
//!
//! Issue #189 (gap 1): weighted ratification and super-ratification already
//! compute exactly which validators supported a block, with which blocks, for
//! how much stake, against which weight table — and then discard all of it,
//! returning a bare `bool`. The conclusion survives; the evidence does not.
//!
//! That forces anyone who wants to check the decision — a light client, an
//! auditor, the Lean conformance replay — to re-derive the whole computation
//! from the blocklace and trust that their answer matches. A
//! [`ThresholdCertificate`] carries the evidence out with the decision, so the
//! boolean becomes a derived, independently re-checkable fact.
//!
//! The field list mirrors the canonical trace's `ThresholdCertificateEvent`
//! one-for-one, so the object and the event it serializes into cannot drift.

use crate::consensus::weight_snapshot::{WeightSnapshot, WeightSnapshotId};
use crate::trace;
use crate::types::{BlockIdentity, NodeId};

/// Which rung of the approval ladder this certificate attests.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CertificateKind {
    /// One block observed a two-thirds stake majority approving the target.
    Ratification,
    /// A two-thirds stake majority ratified the target.
    SuperRatification,
}

impl CertificateKind {
    /// The discriminant as it appears in the canonical trace.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Ratification => "ratification",
            Self::SuperRatification => "super_ratification",
        }
    }
}

/// Portable evidence that a weighted quorum supported a block.
///
/// Everything needed to re-check the decision is here: who supported it, with
/// which blocks, how much stake that was, out of what total, and which weight
/// table those numbers were measured against.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThresholdCertificate {
    pub kind: CertificateKind,
    /// The block this certificate attests.
    pub leader: BlockIdentity,
    /// For ratification, the block whose view observed the quorum.
    /// `None` for super-ratification, which has no single observer.
    pub ratifier: Option<BlockIdentity>,
    /// Stable id linking a finality decision to this certificate.
    pub certificate_id: String,
    /// The evidence blocks, in canonical identity order.
    pub approver_blocks: Vec<BlockIdentity>,
    /// The distinct supporting validators, in canonical order.
    pub approvers: Vec<NodeId>,
    /// Combined bonded stake of `approvers`.
    pub approver_weight: u128,
    /// Total bonded stake in the weight table this was judged against.
    pub total_weight: u128,
    /// Identity of that weight table.
    pub weight_snapshot: WeightSnapshotId,
}

impl ThresholdCertificate {
    /// Assemble a certificate from the evidence a quorum decision produced.
    ///
    /// Both collections are sorted here so canonical order is a structural
    /// property of the type rather than a convention each call site has to
    /// remember.
    pub(crate) fn new(
        kind: CertificateKind,
        leader: &BlockIdentity,
        ratifier: Option<&BlockIdentity>,
        mut approver_blocks: Vec<BlockIdentity>,
        mut approvers: Vec<NodeId>,
        weights: &WeightSnapshot,
    ) -> Self {
        approver_blocks.sort();
        approvers.sort();

        let leader_hash = trace::hex(&leader.content_hash);
        let ratifier_hash = ratifier.map(|id| trace::hex(&id.content_hash));
        let certificate_id =
            trace::certificate_id(kind.as_str(), &leader_hash, ratifier_hash.as_deref());

        // Overflow folds to zero, matching the fail-closed accumulation the
        // quorum predicate itself uses.
        let approver_weight = approvers
            .iter()
            .try_fold(0u128, |total, creator| {
                total.checked_add(u128::from(weights.weight_of(creator)))
            })
            .unwrap_or(0);

        Self {
            kind,
            leader: leader.clone(),
            ratifier: ratifier.cloned(),
            certificate_id,
            approver_blocks,
            approvers,
            approver_weight,
            total_weight: weights.total().unwrap_or(0),
            weight_snapshot: weights.id().clone(),
        }
    }

    /// Number of distinct supporting validators.
    pub fn approver_count(&self) -> usize {
        self.approvers.len()
    }

    /// Re-check the strict two-thirds threshold from the carried numbers
    /// alone, with no blocklace and no recomputation.
    ///
    /// This is the point of the whole type: `3 * support > 2 * total` is
    /// arithmetic anyone can verify. It deliberately spells the rule out
    /// rather than calling the consensus predicate, so it stays an
    /// independent check rather than an echo of the code that produced it.
    pub fn verify_quorum(&self) -> bool {
        let (Some(support), Some(threshold)) = (
            self.approver_weight.checked_mul(3),
            self.total_weight.checked_mul(2),
        ) else {
            return false;
        };

        self.total_weight > 0 && support > threshold
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    fn node(id: u8) -> NodeId {
        NodeId(vec![id])
    }

    fn block(seed: u8, creator: u8) -> BlockIdentity {
        let mut content_hash = [0u8; 32];
        content_hash[0] = seed;
        BlockIdentity {
            content_hash,
            creator: node(creator),
            signature: vec![],
        }
    }

    fn weights(per_validator: u64, count: u8) -> WeightSnapshot {
        let bonds: HashMap<NodeId, u64> = (1..=count).map(|id| (node(id), per_validator)).collect();
        WeightSnapshot::from_bonds(&bonds)
    }

    fn certificate(supporters: &[u8]) -> ThresholdCertificate {
        ThresholdCertificate::new(
            CertificateKind::SuperRatification,
            &block(1, 1),
            None,
            supporters.iter().map(|id| block(*id, *id)).collect(),
            supporters.iter().map(|id| node(*id)).collect(),
            &weights(100, 7),
        )
    }

    /// The capability gap 1 exists to create: check a finality decision from
    /// the certificate alone, with no blocklace in sight.
    #[test]
    fn quorum_verifies_from_the_certificate_alone() {
        let cert = certificate(&[1, 2, 3, 4, 5]);
        assert_eq!(cert.approver_weight, 500);
        assert_eq!(cert.total_weight, 700);
        assert_eq!(cert.approver_count(), 5);
        // 3 * 500 = 1500 > 2 * 700 = 1400
        assert!(cert.verify_quorum());
    }

    #[test]
    fn quorum_rejects_a_bare_majority() {
        let cert = certificate(&[1, 2, 3, 4]);
        assert_eq!(cert.approver_weight, 400);
        // 3 * 400 = 1200, not > 2 * 700 = 1400 — a simple majority is not enough
        assert!(!cert.verify_quorum());
    }

    #[test]
    fn quorum_is_strict_at_the_exact_boundary() {
        // Six of nine equal validators: 3 * 600 == 2 * 900, so not strictly greater.
        let cert = ThresholdCertificate::new(
            CertificateKind::SuperRatification,
            &block(1, 1),
            None,
            (1..=6).map(|id| block(id, id)).collect(),
            (1..=6).map(node).collect(),
            &weights(100, 9),
        );
        assert_eq!((cert.approver_weight, cert.total_weight), (600, 900));
        assert!(!cert.verify_quorum());
    }

    #[test]
    fn evidence_is_stored_in_canonical_order() {
        let cert = certificate(&[5, 1, 3]);
        let mut sorted_approvers = cert.approvers.clone();
        sorted_approvers.sort();
        assert_eq!(cert.approvers, sorted_approvers);

        let mut sorted_blocks = cert.approver_blocks.clone();
        sorted_blocks.sort();
        assert_eq!(cert.approver_blocks, sorted_blocks);
    }

    /// The id must match what the canonical trace carries, or replay cannot
    /// link a finality decision to the certificate that justified it.
    #[test]
    fn certificate_id_matches_the_canonical_trace_encoding() {
        let leader = block(1, 1);
        let ratifier = block(2, 2);
        let leader_hash = trace::hex(&leader.content_hash);
        let ratifier_hash = trace::hex(&ratifier.content_hash);

        let cert = ThresholdCertificate::new(
            CertificateKind::Ratification,
            &leader,
            Some(&ratifier),
            vec![],
            vec![node(1)],
            &weights(100, 7),
        );

        assert_eq!(
            cert.certificate_id,
            trace::certificate_id("ratification", &leader_hash, Some(&ratifier_hash))
        );
    }

    #[test]
    fn an_empty_weight_table_never_reaches_quorum() {
        let bonds: HashMap<NodeId, u64> = HashMap::new();
        let cert = ThresholdCertificate::new(
            CertificateKind::SuperRatification,
            &block(1, 1),
            None,
            vec![],
            vec![],
            &WeightSnapshot::from_bonds(&bonds),
        );
        assert_eq!(cert.total_weight, 0);
        assert!(!cert.verify_quorum());
    }
}
