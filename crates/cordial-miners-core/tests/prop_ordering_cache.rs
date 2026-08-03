use std::collections::HashSet;

use cordial_miners_core::blocklace::Blocklace;
use cordial_miners_core::consensus::{
    OrderingCache, checkpoint_after_finality, tau, tau_with_cache,
};
use cordial_miners_core::crypto::CryptoVerifier;
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};

use proptest::prelude::*;

const MAX_VALIDATORS: u8 = 5;
const WAVELENGTH: u64 = 3;
const SELECTION_ID: u64 = 0;

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

fn make_block(creator_id: u8, tag: u16, predecessors: HashSet<BlockIdentity>) -> Block {
    let mut content_hash = [0u8; 32];
    content_hash[0] = creator_id;
    content_hash[1..3].copy_from_slice(&tag.to_le_bytes());

    Block {
        identity: BlockIdentity {
            content_hash,
            creator: node(creator_id),
            signature: vec![],
        },
        content: BlockContent {
            payload: tag.to_le_bytes().to_vec(),
            predecessors,
        },
    }
}

fn insert(blocklace: &mut Blocklace, block: &Block) -> bool {
    blocklace.insert(block.clone(), &MockVerifier).is_ok()
}

fn leader_v1(_wave: u64) -> Option<NodeId> {
    Some(node(1))
}

fn fault_tolerance(n: usize) -> usize {
    n.saturating_sub(1) / 3
}

#[derive(Debug, Clone)]
struct RawStep {
    creator_raw: u8,
    predecessor_picks: Vec<usize>,
    reference_all_tips: bool,
}

#[derive(Debug, Clone)]
enum StepSpec {
    Insert(RawStep),
    Prune,
}

#[derive(Debug, Clone)]
struct MutationSpec {
    num_validators: u8,
    steps: Vec<StepSpec>,
}

fn step_spec_strategy() -> impl Strategy<Value = StepSpec> {
    let insert_strategy = (
        any::<u8>(),
        prop::collection::vec(0usize..60, 0..=4),
        prop::bool::weighted(0.5),
    )
        .prop_map(|(creator_raw, predecessor_picks, reference_all_tips)| {
            StepSpec::Insert(RawStep {
                creator_raw,
                predecessor_picks,
                reference_all_tips,
            })
        });
    prop_oneof![
        25 => insert_strategy,
        1 => Just(StepSpec::Prune),
    ]
}

fn mutation_spec_strategy() -> impl Strategy<Value = MutationSpec> {
    (
        2u8..=MAX_VALIDATORS,
        prop::collection::vec(step_spec_strategy(), 1..=80),
    )
        .prop_map(|(num_validators, steps)| MutationSpec {
            num_validators,
            steps,
        })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(64))]

    #[test]
    fn cached_tau_matches_fresh_tau_across_inserts_and_prunes(spec in mutation_spec_strategy()) {
        let mut blocklace = Blocklace::new();
        let mut cache = OrderingCache::default();
        let mut identities: Vec<Option<BlockIdentity>> = Vec::with_capacity(spec.steps.len());
        let mut tips: HashSet<BlockIdentity> = HashSet::new();

        let n = spec.num_validators as usize;
        let f = fault_tolerance(n);
        let mut tag: u16 = 1;

        for step in &spec.steps {
            match step {
                StepSpec::Insert(raw) => {
                    let creator = 1 + (raw.creator_raw % spec.num_validators);
                    let predecessors: HashSet<BlockIdentity> = if raw.reference_all_tips {
                        tips.iter().cloned().collect()
                    } else {
                        raw.predecessor_picks
                            .iter()
                            .copied()
                            .filter(|&idx| idx < identities.len())
                            .filter_map(|idx| identities[idx].clone())
                            .collect()
                    };
                    let block = make_block(creator, tag, predecessors.clone());
                    tag += 1;
                    let inserted = insert(&mut blocklace, &block);
                    if inserted {
                        for pred in predecessors {
                            tips.remove(&pred);
                        }
                        tips.insert(block.identity.clone());
                    }
                    identities.push(if inserted { Some(block.identity.clone()) } else { None });
                }
                StepSpec::Prune => {
                    let _ = checkpoint_after_finality(&mut blocklace, WAVELENGTH, n, f, leader_v1);

                    let live: HashSet<BlockIdentity> = blocklace.dom().iter().map(|id| (*id).clone()).collect();
                    for slot in identities.iter_mut() {
                        if let Some(existing) = slot
                            && !live.contains(existing) {
                                *slot = None;

                        }
                    }
                    tips.retain(|id| live.contains(id));
                    identities.push(None);
                }
            }

            let cached =tau_with_cache(
                &blocklace,
                WAVELENGTH,
                n,
                f,
                SELECTION_ID,
                leader_v1,
                &mut cache,
            );
            let fresh = tau(&blocklace, WAVELENGTH, n, f, leader_v1);
            prop_assert_eq!(&cached, &fresh);

            if cached.is_ok() {
                prop_assert_eq!(
                    cache.generation(),
                    blocklace.generation(),
                );
            }
        }
    }
}
