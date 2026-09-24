//! End-to-end test for issue #243: proves that reputation weights
//! computed by `cordial-por`'s pipeline are the same weights Cordial
//! Miners' weighted consensus actually consumes, not a parallel or
//! untested path.
//!
//! This deliberately stays on the `cordial-por -> cordial-miners-core`
//! boundary only: no f1r3node live networking, no adapter event
//! extraction, and no RSpace execution are involved.
//!
//! Three tests, each independently reviewable:
//! - `por_weights_drive_cordial_miners_weighted_finality` drives four
//!   validators through every public pipeline stage (batch -> matrix ->
//!   normalize -> Liquid-Rank -> blend -> clamp), exports weights via
//!   `reputation_weights()`, confirms an ejected validator is dropped
//!   from the export, and feeds the result into `weighted_super_ratifies`.
//! - `weighted_finality_diverges_from_unweighted_supermajority` proves
//!   the weighted result actually depends on PoR-derived reputation, by
//!   constructing a case where a count-based supermajority of witnesses
//!   fails the weight-based check.
//! - `por_weights_drive_cordial_miners_weighted_tau_ordering` proves the
//!   same PoR-derived weights also drive `weighted_tau`, not just
//!   `weighted_super_ratifies`.
//!
//! One property of the pipeline is easy to miss when writing rating
//! fixtures: `normalize_recipient_group` normalizes each rating
//! relative to that specific recipient's own raters' min/max. Unanimous
//! agreement (including the trivial case of a single rater) always
//! collapses to `scale`, regardless of the absolute score given. So
//! separation between recipients' final reputation comes from *rater
//! count and disagreement*, not from picking different absolute score
//! values per recipient — both tests below are built around that fact.

use std::collections::{HashMap, HashSet};

use cordial_por::{
    MissingEntryPolicy, PorConfig, RatingRecord, ReputationEntry, ReputationState,
    ReputationVector, blend_reputation_transition, build_rating_batch, build_rating_matrix,
    clamp_reputation_transition, compute_liquid_rank_contribution, normalize_rating_matrix,
    replay_reputation_transition, reputation_weights,
};

use cordial_miners_core::consensus::{
    approved_blocks_for_leader, is_supermajority, super_ratifies, weighted_super_ratifies,
    weighted_tau, xsort,
};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, Blocklace, NodeId};

struct MockVerifier;
impl CryptoVerifier for MockVerifier {
    type Error = String;
    fn verify_block(
        &self,
        _c: &BlockContent,
        _s: &[u8],
        _creator: &NodeId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn block(creator_id: u8, tag: u8, predecessors: HashSet<BlockIdentity>) -> Block {
    let mut content_hash = [0u8; 32];
    content_hash[0] = creator_id;
    content_hash[1] = tag;
    Block {
        identity: BlockIdentity {
            content_hash,
            creator: node(creator_id),
            signature: vec![],
        },
        content: BlockContent {
            payload: vec![],
            predecessors,
        },
    }
}

fn insert(bl: &mut Blocklace, b: &Block) {
    bl.insert(b.clone(), &MockVerifier).expect("insert failed");
}

/// End-to-end proof that PoR ratings flow through the full reputation
/// pipeline into Cordial Miners' weighted finality check.
///
/// Flow covered:
/// RatingRecord -> RatingBatch -> RatingMatrix -> NormalizedRatingMatrix
/// -> Liquid-Rank contribution -> alpha blend -> clamp -> ReputationState
/// -> reputation_weights() -> weighted_super_ratifies
#[test]
fn por_weights_drive_cordial_miners_weighted_finality() {
    let a = node(1); // strong standing: two raters, both agree at the max
    let b = node(2); // medium standing: two raters, maximal disagreement
    let c = node(3); // weak standing: a single rater
    let d = node(4); // excluded: neither rates nor is rated

    let previous = ReputationVector {
        round: 0,
        values: vec![
            ReputationEntry::new(a.clone(), 50),
            ReputationEntry::new(b.clone(), 50),
            ReputationEntry::new(c.clone(), 50),
            ReputationEntry::new(d.clone(), 50),
        ],
    };

    // Constructed as a struct literal rather than via PorConfig::new(),
    // because new() hardcodes liquid_rank_alpha to 600_000_000 regardless
    // of the scale passed in, which fails blend_reputation_transition's
    // `alpha <= scale` check for any scale far from the crate default.
    let config = PorConfig {
        scale: 100,
        initial_reputation: 50,
        liquid_rank_alpha: 60,
        minimum_rating: 0,
        maximum_rating: 100,
        missing_entry_policy: MissingEntryPolicy::CarryForward,
    };

    // Note: normalize_recipient_group normalizes each rating relative to
    // that recipient's own raters' min/max. Unanimous agreement (or a
    // single rater trivially "agreeing" with itself) always collapses to
    // `scale`, regardless of the absolute score. So separation between
    // recipients comes from rater count and disagreement, not from the
    // raw score numbers.
    let raw_ratings = vec![
        // A: two raters, both agree unanimously -> normalized = 100 for both.
        RatingRecord::new(1, node(2), node(1), 100, vec![0xAB]), // B rates A
        RatingRecord::new(1, node(3), node(1), 100, vec![0xAB]), // C rates A
        // B: two raters, maximal disagreement -> normalized spread pulls the average down.
        RatingRecord::new(1, node(1), node(2), 100, vec![0xAB]), // A rates B
        RatingRecord::new(1, node(3), node(2), 0, vec![0xAB]),   // C rates B
        // C: a single rater -> half the weighted support of A's two raters.
        RatingRecord::new(1, node(1), node(3), 100, vec![0xAB]), // A rates C (B does not rate C)
    ];

    let batch = build_rating_batch(1, raw_ratings.clone(), &config).expect("valid batch");
    assert_eq!(batch.round, 1);
    assert_eq!(batch.ratings.len(), 5);

    let matrix = build_rating_matrix(&batch).expect("valid matrix");
    assert_eq!(matrix.ratings.len(), 5);

    let normalized = normalize_rating_matrix(&matrix, &config).expect("valid normalization");

    let contribution = compute_liquid_rank_contribution(&normalized, &previous, &config)
        .expect("valid contribution");
    assert_eq!(contribution.values.len(), 3);

    // alpha blend.
    let blended =
        blend_reputation_transition(&contribution, &previous, &config).expect("valid blend");
    // D reappears here via CarryForward, still at its previous value pre-clamp.
    assert_eq!(blended.values.len(), 4);

    let clamped = clamp_reputation_transition(&blended, &previous, &contribution, &config)
        .expect("valid clamp");

    let rep_of = |list: &ReputationVector, id: &NodeId| {
        list.values
            .iter()
            .find(|e| &e.node_id == id)
            .unwrap()
            .reputation
    };

    assert!(rep_of(&clamped, &a) > rep_of(&clamped, &b));
    assert!(rep_of(&clamped, &b) > rep_of(&clamped, &c));
    assert_eq!(rep_of(&clamped, &d), 50); // CarryForward: clamp is skipped for D, so it's untouched

    // Cross-check the manual stage-by-stage result against the crate's own
    // audit replay, and confirm re-running with identical inputs is
    // bit-for-bit identical: PoR computes deterministic reputation values
    // from the rating records.
    let audited = replay_reputation_transition(&previous, &raw_ratings, 1, &config)
        .expect("replay successful");
    assert_eq!(audited.entries, clamped.values);

    let audited_again = replay_reputation_transition(&previous, &raw_ratings, 1, &config).unwrap();
    assert_eq!(audited, audited_again);

    // apply the finalized vector to ReputationState.
    let mut state = ReputationState::new(0);
    state
        .apply_reputation_vector(ReputationVector {
            round: audited.round,
            values: audited.entries.clone(),
        })
        .expect("canonical vector");
    assert_eq!(state.reputation_list().entries, audited.entries);

    // Demonstrate exclusion: an ejected validator must not be exported as active weight.
    state.eject_validator(&d).expect("d is known to the state");
    assert!(state.is_ejected(&d));

    // export weights.
    let weights: HashMap<NodeId, u64> = reputation_weights(&state);

    // Exact equality, not just length/ordering: an exporter that corrupted
    // every value (e.g. squaring, doubling) would still pass a length or
    // ordering check. These values are the pipeline's real fixed-point
    // clamp output for this fixture (A: contribution 100 -> blend 80 ->
    // clamp 63; B: 75 -> 65 -> 55; C: 50 -> 50 -> 45), so this also pins the
    // pipeline's numeric output against future accidental changes upstream.
    assert_eq!(
        weights,
        HashMap::from([(a.clone(), 63), (b.clone(), 55), (c.clone(), 45)])
    );

    // feed the exported weights into a Cordial Miners
    // weighted API. Leader = C (lowest active weight), ratified by A and B
    // (the two highest). For any three positive, non-identical weights
    // x1 >= x2 >= x3, the top two always sum to strictly more than
    // two-thirds of the total (x1+x2 >= 2*x3, strict unless all three are
    // equal), so this fixture is guaranteed to pass regardless of the
    // exact clamp arithmetic.
    let leader = block(3, 1, HashSet::new()); // C leads
    let mut bl = Blocklace::new();
    insert(&mut bl, &leader);

    let wa = block(1, 2, HashSet::from([leader.identity.clone()]));
    let wb = block(2, 3, HashSet::from([leader.identity.clone()]));
    insert(&mut bl, &wa);
    insert(&mut bl, &wb);

    let preds = HashSet::from([wa.identity.clone(), wb.identity.clone()]);
    let ra = block(1, 4, preds.clone());
    let rb = block(2, 5, preds);
    insert(&mut bl, &ra);
    insert(&mut bl, &rb);

    let witnesses = HashSet::from([ra, rb]);

    assert!(weighted_super_ratifies(&bl, &witnesses, &leader, &weights));
}

/// Required assertion: weighted consensus results must actually change
/// according to PoR-derived reputation, not just accept a weights map
/// without using it. Constructs a scenario where a count-based
/// supermajority of witnesses fails the weight-based check.
#[test]
fn weighted_finality_diverges_from_unweighted_supermajority() {
    let a = node(1);
    let b = node(2);
    let c = node(3);
    let d = node(4);

    let previous = ReputationVector {
        round: 0,
        values: vec![
            ReputationEntry::new(a.clone(), 50),
            ReputationEntry::new(b.clone(), 50),
            ReputationEntry::new(c.clone(), 50),
            ReputationEntry::new(d.clone(), 50),
        ],
    };

    let config = PorConfig {
        scale: 100,
        initial_reputation: 50,
        liquid_rank_alpha: 60,
        minimum_rating: 0,
        maximum_rating: 100,
        missing_entry_policy: MissingEntryPolicy::CarryForward,
    };

    // A has three raters (B, C, D all agree at the max) -> three summed
    // weighted terms. B, C, D each have only a single rater (A) -> one
    // term each. Since agreement always normalizes to `scale`, it's rater
    // count that creates the separation here, not the score value.
    let skewed_ratings = vec![
        RatingRecord::new(1, node(2), node(1), 100, vec![0xCD]), // B rates A
        RatingRecord::new(1, node(3), node(1), 100, vec![0xCD]), // C rates A
        RatingRecord::new(1, node(4), node(1), 100, vec![0xCD]), // D rates A
        RatingRecord::new(1, node(1), node(2), 100, vec![0xCD]), // A rates B
        RatingRecord::new(1, node(1), node(3), 100, vec![0xCD]), // A rates C
        RatingRecord::new(1, node(1), node(4), 100, vec![0xCD]), // A rates D
    ];

    let skewed = replay_reputation_transition(&previous, &skewed_ratings, 1, &config)
        .expect("replay successful");

    let mut skewed_state = ReputationState::new(0);
    skewed_state
        .apply_reputation_vector(ReputationVector {
            round: 1,
            values: skewed.entries,
        })
        .expect("canonical vector");

    // All four validators remain active in this scenario (no ejection).
    let skewed_weights: HashMap<NodeId, u64> = reputation_weights(&skewed_state);
    assert_eq!(
        skewed_weights,
        HashMap::from([
            (a.clone(), 74),
            (b.clone(), 45),
            (c.clone(), 45),
            (d.clone(), 45),
        ])
    );

    let leader2 = block(1, 10, HashSet::new()); // A leads
    let mut bl2 = Blocklace::new();
    insert(&mut bl2, &leader2);

    let wb2 = block(2, 11, HashSet::from([leader2.identity.clone()]));
    let wc2 = block(3, 12, HashSet::from([leader2.identity.clone()]));
    let wd2 = block(4, 13, HashSet::from([leader2.identity.clone()]));
    insert(&mut bl2, &wb2);
    insert(&mut bl2, &wc2);
    insert(&mut bl2, &wd2);

    let preds2 = HashSet::from([
        wb2.identity.clone(),
        wc2.identity.clone(),
        wd2.identity.clone(),
    ]);
    let rb2 = block(2, 14, preds2.clone());
    let rc2 = block(3, 15, preds2.clone());
    let rd2 = block(4, 16, preds2);
    insert(&mut bl2, &rb2);
    insert(&mut bl2, &rc2);
    insert(&mut bl2, &rd2);

    let witnesses2 = HashSet::from([rb2, rc2, rd2]);

    // By count: 3 of 4 witnesses is a supermajority regardless of stake.
    assert!(is_supermajority(&witnesses2, 4, 0));
    assert!(super_ratifies(&bl2, &witnesses2, &leader2, 4, 0));

    // By PoR-derived stake: B+C+D hold far less than two-thirds once A dominates.
    assert!(!weighted_super_ratifies(
        &bl2,
        &witnesses2,
        &leader2,
        &skewed_weights
    ));
}

/// Proves weighted tau ordering also consumes PoR-derived weights, not
/// just weighted finality. Reuses the same C-leads/A+B-witness fixture shape as
/// `por_weights_drive_cordial_miners_weighted_finality`, extended one round
/// further to match the 3-round structure `weighted_tau` expects for a
/// wavelength of 3 mirrored from cordial-miners-core's own
/// `weighted_tau_returns_xsort_of_approved_blocks_for_single_weighted_final_leader`.
#[test]
fn por_weights_drive_cordial_miners_weighted_tau_ordering() {
    let a = node(1);
    let b = node(2);
    let c = node(3);
    let d = node(4);

    let previous = ReputationVector {
        round: 0,
        values: vec![
            ReputationEntry::new(a.clone(), 50),
            ReputationEntry::new(b.clone(), 50),
            ReputationEntry::new(c.clone(), 50),
            ReputationEntry::new(d.clone(), 50),
        ],
    };

    let config = PorConfig {
        scale: 100,
        initial_reputation: 50,
        liquid_rank_alpha: 60,
        minimum_rating: 0,
        maximum_rating: 100,
        missing_entry_policy: MissingEntryPolicy::CarryForward,
    };

    let raw_ratings = vec![
        RatingRecord::new(1, node(2), node(1), 100, vec![0xAB]), // B rates A
        RatingRecord::new(1, node(3), node(1), 100, vec![0xAB]), // C rates A
        RatingRecord::new(1, node(1), node(2), 100, vec![0xAB]), // A rates B
        RatingRecord::new(1, node(3), node(2), 0, vec![0xAB]),   // C rates B
        RatingRecord::new(1, node(1), node(3), 100, vec![0xAB]), // A rates C
    ];

    let audited = replay_reputation_transition(&previous, &raw_ratings, 1, &config)
        .expect("replay successful");

    let mut state = ReputationState::new(0);
    state
        .apply_reputation_vector(ReputationVector {
            round: audited.round,
            values: audited.entries,
        })
        .expect("canonical vector");

    state.eject_validator(&d).expect("d is known to the state");

    let weights: HashMap<NodeId, u64> = reputation_weights(&state);

    assert_eq!(
        weights,
        HashMap::from([(a.clone(), 63), (b.clone(), 55), (c.clone(), 45)])
    );

    // Round 0: leader block by C.
    let leader = block(3, 1, HashSet::new());
    let mut bl = Blocklace::new();
    insert(&mut bl, &leader);

    // Round 1: A and B observe the leader.
    let wa = block(1, 2, HashSet::from([leader.identity.clone()]));
    let wb = block(2, 3, HashSet::from([leader.identity.clone()]));
    insert(&mut bl, &wa);
    insert(&mut bl, &wb);

    // Round 2: A and B observe each other's round-1 blocks.
    let preds = HashSet::from([wa.identity.clone(), wb.identity.clone()]);
    let ra = block(1, 4, preds.clone());
    let rb = block(2, 5, preds);
    insert(&mut bl, &ra);
    insert(&mut bl, &rb);

    fn leader_c(_wave: u64) -> Option<NodeId> {
        Some(node(3))
    }

    let approved = approved_blocks_for_leader(&bl, &leader.identity);
    let ordered = weighted_tau(&bl, 3, &weights, leader_c).expect("weighted tau succeeds");

    assert!(!ordered.is_empty());
    assert_eq!(ordered, xsort(&approved).unwrap());
}
