use std::{
    collections::{HashMap, HashSet},
    fs,
};

use cordial_f1r3node_adapter::{
    ordered_output::OrderedFinalizedOutput,
    por::{
        finality::finalized_output_for_wave,
        local_ratings::{
            close_rating_exchange_if_complete, observe_rating_exchange, stage_local_rating_batch,
        },
        rating_key::load_bonded_rating_key,
        transition::apply_completed_reputation_round,
        transport::{file_exchange::FileRatingExchange, outbox::RatingOutbox},
    },
};
use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId,
    consensus::{latest_weighted_final_leader, weighted_tau},
    crypto::CryptoVerifier,
};
use cordial_por::{PorConfig, ReputationState, authorized_validator_weights};
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
    let mut seeds = [1, 2, 3, 4];
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

#[test]
fn distinct_observers_exchange_evidence_checked_ratings_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let bonds = HashMap::from([(node(1), 1), (node(2), 1), (node(3), 1)]);
    let authorized: HashSet<_> = bonds.keys().cloned().collect();
    let mut state = ReputationState::new(0);
    for validator in bonds.keys() {
        state.set_reputation(validator.clone(), 100);
    }
    let config = PorConfig::default();
    let mut blocklace = Blocklace::new();
    let leader = block(1, 1, None);
    let second = block(2, 2, Some(&leader.identity));
    let third = block(3, 3, Some(&second.identity));
    for entry in [&leader, &second, &third] {
        blocklace.insert(entry.clone(), &AcceptAll).unwrap();
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
    );
    let exchange_root = directory.path().join("exchange");
    let exchange = FileRatingExchange::open(&exchange_root, b"root").unwrap();
    let mut observers = Vec::new();
    for seed in [1, 2, 3] {
        let data_dir = directory.path().join(format!("observer-{seed}"));
        fs::create_dir_all(&data_dir).unwrap();
        let key_path = data_dir.join("validator.key");
        fs::write(&key_path, hex::encode([seed; 32])).unwrap();
        let key = load_bonded_rating_key(&key_path, &bonds).unwrap();
        let outbox = RatingOutbox::open(&data_dir).unwrap();
        stage_local_rating_batch(&blocklace, &output, &state, &config, &key, &outbox)
            .unwrap()
            .unwrap();
        observers.push((data_dir, key, outbox));
    }

    let mut changed_evidence = output.clone();
    changed_evidence.blocks.retain(|id| id == &leader.identity);
    assert!(
        observe_rating_exchange(
            &blocklace,
            &changed_evidence,
            &state,
            &config,
            &observers[0].1,
            &authorized,
            &observers[0].2,
            &exchange,
        )
        .is_err()
    );
    assert!(
        close_rating_exchange_if_complete(
            &blocklace,
            &changed_evidence,
            &state,
            &config,
            &observers[0].1,
            &authorized,
            &observers[0].2,
            &exchange,
        )
        .is_err()
    );
    assert!(
        exchange
            .read_peer_envelopes(0, &node(2), &authorized)
            .unwrap()
            .is_empty()
    );

    let observe = |index: usize, exchange: &FileRatingExchange| {
        observe_rating_exchange(
            &blocklace,
            &output,
            &state,
            &config,
            &observers[index].1,
            &authorized,
            &observers[index].2,
            exchange,
        )
        .unwrap()
    };
    let first = observe(0, &exchange);
    assert_eq!(first.local_envelopes, 2);
    assert_eq!(first.peer_envelopes, 0);
    assert_eq!(first.completed_raters, 1);
    assert!(!first.quorum_reached);
    let second = observe(1, &exchange);
    assert_eq!(second.peer_envelopes, 2);
    assert_eq!(second.completed_raters, 2);
    assert!(!second.quorum_reached);
    assert!(
        close_rating_exchange_if_complete(
            &blocklace,
            &output,
            &state,
            &config,
            &observers[0].1,
            &authorized,
            &observers[0].2,
            &exchange,
        )
        .unwrap()
        .is_none()
    );
    let third = observe(2, &exchange);
    assert_eq!(third.peer_envelopes, 4);
    assert!(third.all_raters_complete);
    assert!(third.quorum_reached);
    assert_eq!(observe(0, &exchange).completed_raters, 3);

    drop(exchange);
    let reopened = FileRatingExchange::open(&exchange_root, b"root").unwrap();
    let reopened_outbox = RatingOutbox::open(&observers[0].0).unwrap();
    let recovered = observe_rating_exchange(
        &blocklace,
        &output,
        &state,
        &config,
        &observers[0].1,
        &authorized,
        &reopened_outbox,
        &reopened,
    )
    .unwrap();
    assert!(recovered.all_raters_complete);
    assert_eq!(recovered.peer_envelopes, 4);
    assert_eq!(state.round(), 0);

    let mut projections = Vec::new();
    for (_, key, outbox) in &observers {
        let completed = close_rating_exchange_if_complete(
            &blocklace,
            &output,
            &state,
            &config,
            key,
            &authorized,
            outbox,
            &reopened,
        )
        .unwrap()
        .unwrap();
        let mut projected = state.clone();
        let applied =
            apply_completed_reputation_round(&completed, &mut projected, &config, b"root").unwrap();
        let mut validators: Vec<_> = bonds.keys().cloned().collect();
        validators.sort();
        projections.push((
            completed.batch().clone(),
            applied.block,
            authorized_validator_weights(&projected, &validators).unwrap(),
        ));
    }
    assert_eq!(projections[0], projections[1]);
    assert_eq!(projections[1], projections[2]);
    assert_eq!(state.round(), 0);
}

#[test]
fn weighted_quorum_does_not_close_an_incomplete_exchange() {
    let directory = tempfile::tempdir().unwrap();
    let bonds = HashMap::from([(node(1), 1), (node(2), 1), (node(3), 1), (node(4), 1)]);
    let authorized: HashSet<_> = bonds.keys().cloned().collect();
    let mut state = ReputationState::new(0);
    for validator in bonds.keys() {
        state.set_reputation(validator.clone(), 100);
    }
    let config = PorConfig::default();
    let mut blocklace = Blocklace::new();
    let first = block(1, 1, None);
    let second = block(2, 2, Some(&first.identity));
    let third = block(3, 3, Some(&second.identity));
    for entry in [&first, &second, &third] {
        blocklace.insert(entry.clone(), &AcceptAll).unwrap();
    }
    let output = OrderedFinalizedOutput::new(
        vec![
            first.identity.clone(),
            second.identity.clone(),
            third.identity.clone(),
        ],
        Some(first.identity),
        3,
        4,
        3,
    );
    let exchange = FileRatingExchange::open(&directory.path().join("exchange"), b"root").unwrap();
    let mut observers = Vec::new();
    for seed in 1..=4 {
        let data_dir = directory.path().join(format!("observer-{seed}"));
        fs::create_dir_all(&data_dir).unwrap();
        let key_path = data_dir.join("validator.key");
        fs::write(&key_path, hex::encode([seed; 32])).unwrap();
        let key = load_bonded_rating_key(&key_path, &bonds).unwrap();
        let outbox = RatingOutbox::open(&data_dir).unwrap();
        stage_local_rating_batch(&blocklace, &output, &state, &config, &key, &outbox)
            .unwrap()
            .unwrap();
        observers.push((key, outbox));
    }
    for (key, outbox) in observers.iter().take(3) {
        observe_rating_exchange(
            &blocklace,
            &output,
            &state,
            &config,
            key,
            &authorized,
            outbox,
            &exchange,
        )
        .unwrap();
    }
    let (key, outbox) = &observers[0];
    let progress = observe_rating_exchange(
        &blocklace,
        &output,
        &state,
        &config,
        key,
        &authorized,
        outbox,
        &exchange,
    )
    .unwrap();
    assert!(progress.quorum_reached);
    assert_eq!(progress.completed_raters, 3);
    assert!(!progress.all_raters_complete);
    assert!(
        close_rating_exchange_if_complete(
            &blocklace,
            &output,
            &state,
            &config,
            key,
            &authorized,
            outbox,
            &exchange,
        )
        .unwrap()
        .is_none()
    );
    let (last_key, last_outbox) = &observers[3];
    observe_rating_exchange(
        &blocklace,
        &output,
        &state,
        &config,
        last_key,
        &authorized,
        last_outbox,
        &exchange,
    )
    .unwrap();
    assert!(
        close_rating_exchange_if_complete(
            &blocklace,
            &output,
            &state,
            &config,
            key,
            &authorized,
            outbox,
            &exchange,
        )
        .unwrap()
        .is_some()
    );
}
