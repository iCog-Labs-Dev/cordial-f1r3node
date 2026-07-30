//! Property tests for weighted ratification threshold behavior.
//!
//! Covers:
//! - `is_weighted_supermajority` matches the strict-two-thirds definition
//!   directly (support_weight * 3 > total_weight * 2) for arbitrary bond
//!   assignments.
//! - `is_weighted_supermajority` is monotonic: growing the support set can
//!   never turn a passing check into a failing one.
//! - Weighted and unweighted finality agree on generated round-based DAGs
//!   when every validator is equally staked (generalizes the fixed example
//!   `weighted_and_unweighted_agree_when_high_stake_ratifiers` in
//!   `test_finality.rs` across random validator counts / wavelengths).
//!
//! Failing cases print the exact bonds map / validator subset / `DagSpec`
//! needed to reproduce, and are persisted under
//! `crates/cordial-miners-core/proptest-regressions/prop_finality.txt`.

use std::collections::{HashMap, HashSet};

use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{
    is_final_leader, is_weighted_final_leader, is_weighted_supermajority,
};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};

use proptest::prelude::*;

struct MockVerifier;

impl CryptoVerifier for MockVerifier {
    type Error = String;

    fn verify_block(
        &self,
        _content: &BlockContent,
        _sig: &[u8],
        _creator: &NodeId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn bonds(ids: &[u8], stake: u64) -> HashMap<NodeId, u64> {
    ids.iter().map(|id| (node(*id), stake)).collect()
}

fn make_block(creator_id: u8, tag: u8, predecessors: HashSet<BlockIdentity>) -> Block {
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
            payload: vec![tag],
            predecessors,
        },
    }
}

fn insert(blocklace: &mut Blocklace, block: &Block) {
    let verifier = MockVerifier;
    blocklace
        .insert(block.clone(), &verifier)
        .expect("generator must only ever produce closure-valid blocks");
}

/// Parameters for a generated round-based DAG. `Debug` is derived so that a
/// proptest failure prints the exact spec needed to reproduce the DAG.
#[derive(Debug, Clone)]
struct DagSpec {
    validators: Vec<u8>,
    /// Highest round index present in the DAG (rounds run `0..=max_round`).
    max_round: u8,
    /// Optional single equivocation point: (validator id, round).
    equivocation: Option<(u8, u8)>,
}

struct GeneratedDag {
    blocklace: Blocklace,
    /// Blocks grouped by the round they were generated in (round 0 first).
    blocks_by_round: Vec<Vec<Block>>,
}

/// A proptest strategy over [`DagSpec`]: 2-5 validators, 0-4 extra rounds
/// beyond genesis, and an optional equivocation point.
fn dag_spec_strategy() -> impl Strategy<Value = DagSpec> {
    (2usize..=5, 0u8..=4).prop_flat_map(|(num_validators, max_round)| {
        let validators: Vec<u8> = (1..=num_validators as u8).collect();
        let validators_for_equiv = validators.clone();

        (
            Just(validators),
            Just(max_round),
            prop::option::of((prop::sample::select(validators_for_equiv), 0u8..=max_round)),
        )
            .prop_map(|(validators, max_round, equivocation)| DagSpec {
                validators,
                max_round,
                equivocation,
            })
    })
}

/// Materialize a [`DagSpec`] into an actual [`Blocklace`].
fn build_dag(spec: &DagSpec) -> GeneratedDag {
    let mut blocklace = Blocklace::new();
    let mut blocks_by_round: Vec<Vec<Block>> = Vec::new();
    let mut tag: u8 = 1;
    let mut previous_round_ids: HashSet<BlockIdentity> = HashSet::new();

    for round in 0..=spec.max_round {
        let mut this_round: Vec<Block> = Vec::new();

        for &validator in &spec.validators {
            let is_equivocator = spec.equivocation == Some((validator, round));
            let branch_count = if is_equivocator { 2 } else { 1 };

            for _ in 0..branch_count {
                let block = make_block(validator, tag, previous_round_ids.clone());
                tag += 1;
                insert(&mut blocklace, &block);
                this_round.push(block);
            }
        }

        previous_round_ids = this_round.iter().map(|b| b.identity.clone()).collect();
        blocks_by_round.push(this_round);
    }

    GeneratedDag {
        blocklace,
        blocks_by_round,
    }
}

/// Standard BFT fault tolerance for `n` validators: the largest `f` with
/// `n >= 3f + 1`.
fn fault_tolerance(n: usize) -> usize {
    n.saturating_sub(1) / 3
}

/// Manual reference implementation of the strict two-thirds threshold, kept
/// independent of the implementation under test.
fn manual_strict_two_thirds(support: &HashSet<NodeId>, bonds_map: &HashMap<NodeId, u64>) -> bool {
    let total: u128 = bonds_map.values().map(|w| u128::from(*w)).sum();
    if total == 0 {
        return false;
    }
    let support_weight: u128 = support
        .iter()
        .filter_map(|creator| bonds_map.get(creator))
        .map(|w| u128::from(*w))
        .sum();

    support_weight * 3 > total * 2
}

fn bonds_strategy() -> impl Strategy<Value = HashMap<NodeId, u64>> {
    prop::collection::vec((1u8..=8, 0u64..=1000), 1..=8).prop_map(|entries| {
        entries
            .into_iter()
            .map(|(id, weight)| (node(id), weight))
            .collect()
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(128))]

    /// `is_weighted_supermajority` must agree with the direct strict
    /// two-thirds definition for any bonds map and any subset of it chosen
    /// as the "support" set.
    #[test]
    fn weighted_supermajority_matches_manual_threshold(
        bonds_map in bonds_strategy(),
        support_mask in prop::collection::vec(any::<bool>(), 1..=8),
    ) {
        let creators: Vec<NodeId> = bonds_map.keys().cloned().collect();
        let support: HashSet<NodeId> = creators
            .iter()
            .zip(support_mask.iter().cycle())
            .filter(|(_, include)| **include)
            .map(|(creator, _)| creator.clone())
            .collect();

        let expected = manual_strict_two_thirds(&support, &bonds_map);
        let actual = is_weighted_supermajority(&support, &bonds_map);
        prop_assert_eq!(actual, expected);
    }

    /// Growing the support set can never turn passing supermajority into
    /// failing supermajority: weighted stake support is monotonic.
    #[test]
    fn weighted_supermajority_is_monotonic_in_support(
        bonds_map in bonds_strategy(),
        support_mask in prop::collection::vec(any::<bool>(), 1..=8),
        extra_mask in prop::collection::vec(any::<bool>(), 1..=8),
    ) {
        let creators: Vec<NodeId> = bonds_map.keys().cloned().collect();
        let small: HashSet<NodeId> = creators
            .iter()
            .zip(support_mask.iter().cycle())
            .filter(|(_, include)| **include)
            .map(|(creator, _)| creator.clone())
            .collect();

        // `large` is guaranteed to be a superset of `small` by construction.
        let mut large = small.clone();
        for (creator, include_extra) in creators.iter().zip(extra_mask.iter().cycle()) {
            if *include_extra {
                large.insert(creator.clone());
            }
        }

        if is_weighted_supermajority(&small, &bonds_map) {
            prop_assert!(
                is_weighted_supermajority(&large, &bonds_map),
                "growing support from {:?} to {:?} lost supermajority",
                small,
                large
            );
        }
    }

    /// On a fully-participating, equally-staked round-based DAG, weighted
    /// finality and unweighted finality of the round-0 leader must agree —
    /// equal stake means "supermajority of validators" and "two-thirds of
    /// stake" describe the same threshold.
    #[test]
    fn weighted_and_unweighted_finality_agree_under_equal_stake(
        spec in dag_spec_strategy().prop_filter(
            "need at least 2 full rounds beyond the leader round to reach finality",
            |spec| spec.max_round >= 2 && spec.equivocation.is_none(),
        ),
        stake in 1u64..=1000,
    ) {
        let wavelength = 3u64;
        let n = spec.validators.len();
        let f = fault_tolerance(n);
        let leader_id = spec.validators[0];
        let leader_selection = move |_wave: u64| Some(node(leader_id));

        let dag = build_dag(&spec);
        let leader_identity = dag.blocks_by_round[0][0].identity.clone();
        let equal_bonds = bonds(&spec.validators, stake);

        let unweighted =
            is_final_leader(&dag.blocklace, &leader_identity, wavelength, n, f, leader_selection);
        let weighted = is_weighted_final_leader(
            &dag.blocklace,
            &leader_identity,
            wavelength,
            &equal_bonds,
            leader_selection,
        );

        prop_assert_eq!(unweighted, weighted);
    }
}
