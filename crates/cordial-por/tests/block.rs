use cordial_miners_core::NodeId;
use cordial_por::{
    PorError, ReputationBlockHeader, ReputationEntry, ReputationList, build_reputation_block,
};

fn entry(node: u8, reputation: u64) -> ReputationEntry {
    ReputationEntry::new(NodeId(vec![node]), reputation)
}

fn list(round: u64, entries: Vec<ReputationEntry>) -> ReputationList {
    ReputationList { round, entries }
}

fn header(round: u64) -> ReputationBlockHeader {
    ReputationBlockHeader {
        round,
        previous_reputation_hash: Some(vec![0x01]),
        ratings_hash: vec![0x02],
        reputation_root: vec![0x03],
    }
}

#[test]
fn builds_reputation_block_from_finalized_list() {
    let header = header(7);
    let reputation_list = list(7, vec![entry(1, 90), entry(3, 30)]);

    let block = build_reputation_block(header.clone(), reputation_list.clone()).unwrap();

    assert_eq!(block.header, header);
    assert_eq!(block.reputation_list, reputation_list);
}

#[test]
fn allows_empty_reputation_list_with_required_header_fields() {
    let block = build_reputation_block(header(8), list(8, Vec::new())).unwrap();

    assert_eq!(block.reputation_list.round, 8);
    assert!(block.reputation_list.entries.is_empty());
}

#[test]
fn allows_genesis_block_without_previous_reputation_hash() {
    let mut header = header(0);
    header.previous_reputation_hash = None;

    let block = build_reputation_block(header, list(0, vec![entry(1, 100)])).unwrap();

    assert!(block.header.previous_reputation_hash.is_none());
}

#[test]
fn rejects_round_mismatch() {
    let result = build_reputation_block(header(8), list(7, vec![entry(1, 90)]));

    assert_eq!(result, Err(PorError::InvalidReputationBlockRound));
}

#[test]
fn rejects_duplicate_reputation_entries() {
    let result = build_reputation_block(header(7), list(7, vec![entry(1, 90), entry(1, 30)]));

    assert_eq!(result, Err(PorError::DuplicateReputationEntry));
}

#[test]
fn rejects_unsorted_reputation_entries() {
    let result = build_reputation_block(header(7), list(7, vec![entry(2, 30), entry(1, 90)]));

    assert_eq!(result, Err(PorError::UnsortedReputationVector));
}

#[test]
fn rejects_empty_ratings_hash() {
    let mut header = header(7);
    header.ratings_hash.clear();

    let result = build_reputation_block(header, list(7, vec![entry(1, 90)]));

    assert_eq!(result, Err(PorError::MissingReputationBlockRatingsHash));
}

#[test]
fn rejects_empty_reputation_root() {
    let mut header = header(7);
    header.reputation_root.clear();

    let result = build_reputation_block(header, list(7, vec![entry(1, 90)]));

    assert_eq!(result, Err(PorError::MissingReputationBlockRoot));
}
