use std::{
    collections::{HashMap, HashSet},
    fs,
};

use cordial_f1r3node_adapter::{
    ordered_output::OrderedFinalizedOutput,
    por::{
        finality::finalized_output_for_wave, local_ratings::stage_local_rating_batch,
        rating_key::load_bonded_rating_key, transport::outbox::RatingOutbox,
    },
};
use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId,
    consensus::{latest_weighted_final_leader, weighted_tau},
    crypto::CryptoVerifier,
};
use cordial_por::{PorConfig, ReputationState};
use k256::ecdsa::SigningKey;

struct AcceptAll;

impl CryptoVerifier for AcceptAll {
    type Error = String;

    fn verify_block(
        &self,
        _content: &BlockContent,
        _signature: &[u8],
        _creator: &NodeId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

fn node(seed: u8) -> NodeId {
    let signing_key = SigningKey::from_slice(&[seed; 32]).unwrap();
    NodeId(
        signing_key
            .verifying_key()
            .to_encoded_point(true)
            .as_bytes()
            .to_vec(),
    )
}

fn block(tag: u8, creator: u8, predecessor: Option<&BlockIdentity>) -> Block {
    block_with_predecessors(
        tag,
        creator,
        &predecessor.into_iter().cloned().collect::<Vec<_>>(),
    )
}

fn block_with_predecessors(tag: u8, creator: u8, predecessors: &[BlockIdentity]) -> Block {
    let mut content_hash = [0; 32];
    content_hash[0] = tag;
    Block {
        identity: BlockIdentity {
            content_hash,
            creator: node(creator),
            signature: vec![tag],
        },
        content: BlockContent {
            payload: vec![tag],
            predecessors: predecessors.iter().cloned().collect::<HashSet<_>>(),
        },
    }
}

#[test]
fn later_final_leader_replays_and_stages_the_expected_wave() {
    let directory = tempfile::tempdir().unwrap();
    let mut seeds = vec![1, 2, 3, 4];
    seeds.sort_by_key(|seed| node(*seed));
    let bonds: HashMap<_, _> = seeds.iter().map(|seed| (node(*seed), 1)).collect();
    let key_path = directory.path().join("validator.key");
    fs::write(&key_path, hex::encode([seeds[3]; 32])).unwrap();
    let key = load_bonded_rating_key(&key_path, &bonds).unwrap();
    let outbox = RatingOutbox::open(directory.path()).unwrap();
    let mut state = ReputationState::new(0);
    for validator in bonds.keys() {
        state.set_reputation(validator.clone(), 100);
    }
    let mut blocklace = Blocklace::new();
    let first = block_with_predecessors(1, seeds[0], &[]);
    blocklace.insert(first.clone(), &AcceptAll).unwrap();
    let mut frontier = vec![first.identity.clone()];
    let mut tag = 2;
    for round in 1..=5 {
        let producers: Vec<u8> = if round == 3 {
            vec![seeds[1]]
        } else if round < 3 {
            seeds
                .iter()
                .copied()
                .filter(|seed| *seed != seeds[0])
                .collect()
        } else {
            seeds
                .iter()
                .copied()
                .filter(|seed| *seed != seeds[1])
                .collect()
        };
        let mut next = Vec::new();
        for producer in producers {
            let entry = block_with_predecessors(tag, producer, &frontier);
            blocklace.insert(entry.clone(), &AcceptAll).unwrap();
            next.push(entry.identity);
            tag += 1;
        }
        frontier = next;
    }
    let leaders: Vec<_> = seeds.iter().map(|seed| node(*seed)).collect();
    let leader_selection = |wave: u64| Some(leaders[(wave as usize) % leaders.len()].clone());
    let latest_leader =
        latest_weighted_final_leader(&blocklace, 3, &bonds, leader_selection).unwrap();
    let ordered = weighted_tau(&blocklace, 3, &bonds, leader_selection).unwrap();
    let latest = OrderedFinalizedOutput::new(
        ordered,
        Some(latest_leader),
        3,
        bonds.len(),
        blocklace.dom().len(),
    );
    let wave0 = finalized_output_for_wave(&blocklace, &latest, &bonds, 0)
        .unwrap()
        .unwrap();
    assert_eq!(
        cordial_f1r3node_adapter::por::finality::finalized_wave_from_output(&blocklace, &latest)
            .unwrap(),
        Some(1)
    );
    let staged = stage_local_rating_batch(
        &blocklace,
        &wave0,
        &state,
        &PorConfig::default(),
        &key,
        &outbox,
    )
    .unwrap()
    .unwrap();
    assert_eq!(staged.finalized_wave, 0);
    assert!(staged.rating_count > 0);
    assert_eq!(
        outbox.load(0, key.rater()).unwrap().unwrap().len(),
        staged.rating_count
    );
}

#[test]
fn next_finalized_wave_stages_a_signed_batch_without_committing_state() {
    let directory = tempfile::tempdir().unwrap();
    let key_path = directory.path().join("validator.key");
    fs::write(&key_path, hex::encode([2; 32])).unwrap();
    let bonds = HashMap::from([(node(1), 1), (node(2), 1), (node(3), 1)]);
    let key = load_bonded_rating_key(&key_path, &bonds).unwrap();
    let outbox = RatingOutbox::open(directory.path()).unwrap();
    let mut state = ReputationState::new(0);
    for validator in bonds.keys() {
        state.set_reputation(validator.clone(), 100);
    }
    let config = PorConfig::default();
    let mut blocklace = Blocklace::new();
    let leader = block(1, 1, None);
    let second = block(2, 2, Some(&leader.identity));
    let third = block(3, 3, Some(&second.identity));
    for block in [&leader, &second, &third] {
        blocklace.insert(block.clone(), &AcceptAll).unwrap();
    }
    let output = OrderedFinalizedOutput::new(
        vec![
            leader.identity.clone(),
            second.identity.clone(),
            third.identity.clone(),
        ],
        Some(leader.identity.clone()),
        3,
        3,
        3,
    )
    .with_timestamp(0);

    let staged = stage_local_rating_batch(&blocklace, &output, &state, &config, &key, &outbox)
        .unwrap()
        .unwrap();
    assert_eq!(staged.finalized_wave, 0);
    assert_eq!(staged.rating_round, 1);
    assert_eq!(staged.rating_count, 2);
    assert_eq!(outbox.load(0, key.rater()).unwrap().unwrap().len(), 2);
    assert_eq!(state.round(), 0);
    drop(outbox);
    let outbox = RatingOutbox::open(directory.path()).unwrap();
    assert_eq!(
        stage_local_rating_batch(&blocklace, &output, &state, &config, &key, &outbox).unwrap(),
        Some(staged)
    );

    let later = block(4, 1, Some(&third.identity));
    blocklace.insert(later.clone(), &AcceptAll).unwrap();
    let skipped = OrderedFinalizedOutput::new(output.blocks.clone(), Some(later.identity), 3, 3, 4)
        .with_timestamp(0);
    assert!(
        stage_local_rating_batch(&blocklace, &skipped, &state, &config, &key, &outbox).is_err()
    );
    assert!(outbox.load(1, key.rater()).unwrap().is_none());
}

#[test]
fn output_without_finality_does_not_stage_a_batch() {
    let directory = tempfile::tempdir().unwrap();
    let key_path = directory.path().join("validator.key");
    fs::write(&key_path, hex::encode([2; 32])).unwrap();
    let key = load_bonded_rating_key(&key_path, &HashMap::from([(node(2), 1)])).unwrap();
    let outbox = RatingOutbox::open(directory.path()).unwrap();
    assert_eq!(
        stage_local_rating_batch(
            &Blocklace::new(),
            &OrderedFinalizedOutput::default(),
            &ReputationState::new(0),
            &PorConfig::default(),
            &key,
            &outbox
        )
        .unwrap(),
        None
    );
}
