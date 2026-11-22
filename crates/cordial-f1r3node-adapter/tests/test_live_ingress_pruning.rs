use std::collections::{HashMap, HashSet};

use cordial_f1r3node_adapter::{
    grpc_ingest::BlocklaceAdapter,
    live_ingress::{LiveIngress, MirrorDisposition},
    ordered_output::OrderedFinalizedOutput,
    por::{PorWeightEpochPolicy, PorWeightEpochRecord, finalized_prefix_checkpoint},
    shard_conf::CasperShardConf,
    shared_ordered_output::ReadOrderedOutput,
    snapshot::{CORDIAL_WAVELENGTH as W, CasperSnapshot},
};
use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, NodeId,
    consensus::{PruneReport, depth, wave_of_round},
    crypto::block_hash,
    execution::{BlockState, CordialBlockPayload},
};

struct AcceptBlock;
impl BlocklaceAdapter<BlockIdentity> for AcceptBlock {
    fn on_block(&mut self, _: Block) -> anyhow::Result<()> {
        Ok(())
    }
}
type Ingress = LiveIngress<AcceptBlock>;

fn make_block(validator: u8, round: u64, predecessors: HashSet<BlockIdentity>) -> Block {
    let creator = NodeId(vec![validator]);
    let content = BlockContent {
        predecessors,
        payload: CordialBlockPayload {
            por_authorizations: vec![],
            state: BlockState {
                pre_state_hash: vec![0; 32],
                post_state_hash: vec![1; 32],
                bonds: vec![],
                block_number: round,
            },
            deploys: vec![],
            rejected_deploys: vec![],
            system_deploys: vec![],
        }
        .to_bytes(),
    };
    Block {
        identity: BlockIdentity {
            content_hash: block_hash(&creator, &content),
            creator,
            signature: vec![],
        },
        content,
    }
}

fn stream(rounds: u64) -> Vec<Block> {
    let mut history = Vec::new();
    let mut parents = HashSet::new();
    for round in 0..rounds {
        let batch: Vec<_> = (1..=4)
            .map(|v| make_block(v, round, parents.clone()))
            .collect();
        parents = batch.iter().map(|b| b.identity.clone()).collect();
        history.extend(batch);
    }
    history
}

fn feed(live: &mut Ingress, block: Block) {
    assert_eq!(
        live.ingest_trusted_block(block).unwrap().disposition,
        MirrorDisposition::Applied
    );
}

fn ingress(rounds: u64) -> Ingress {
    let bonds = (1..=4).map(|v| (NodeId(vec![v]), 25)).collect();
    let mut live =
        LiveIngress::with_consensus_view(AcceptBlock, bonds, CasperShardConf::default(), "root");
    for block in stream(rounds) {
        feed(&mut live, block);
    }
    live
}

fn output(live: &mut Ingress) -> OrderedFinalizedOutput {
    live.latest_finalized_ordered_output(W).unwrap()
}

fn checkpoint(live: &mut Ingress) -> PruneReport<BlockIdentity> {
    live.checkpoint_and_prune(W).unwrap().unwrap()
}

fn assert_noop(live: &mut Ingress) {
    let state = |i: &Ingress| {
        (
            i.blocklace().generation(),
            i.blocklace().dom().len(),
            i.blocklace().checkpoint().cloned(),
            i.phase(),
            i.bonds().clone(),
            i.future_epoch().cloned(),
            i.pending_blocks().clone(),
            i.ordered_output_reader().latest().cloned(),
            i.ordering_diagnostics().clone(),
            i.shard_conf().clone(),
            i.shard_id().to_owned(),
        )
    };
    let before = state(live);
    assert!(matches!(live.checkpoint_and_prune(W), Ok(None)));
    assert_eq!(state(live), before);
}

fn assert_snapshot_eq(before: &CasperSnapshot, after: &CasperSnapshot) {
    // Compare fields because map/set display order is unstable and the
    // production snapshot mirrors do not implement PartialEq.
    macro_rules! same {
        ($($($field:ident).+),+ $(,)?) => {
            $(assert_eq!(before.$($field).+, after.$($field).+, stringify!($($field).+));)+
        };
    }
    same!(
        dag.dag_set,
        dag.latest_messages_map,
        dag.child_map,
        dag.height_map,
        dag.block_number_map,
        dag.invalid_blocks_set,
        dag.last_finalized_block_hash,
        dag.finalized_blocks_set,
        last_finalized_block,
        ordered_finalized_blocks,
        lca,
        tips,
        parents,
        justifications,
        invalid_blocks,
        deploys_in_scope,
        max_block_num,
        max_seq_nums,
        on_chain_state.bonds_map,
        on_chain_state.active_validators,
        on_chain_state.shard_conf.fault_tolerance_threshold,
        on_chain_state.shard_conf.shard_name,
        on_chain_state.shard_conf.max_number_of_parents,
        on_chain_state.shard_conf.max_parent_depth,
        on_chain_state.shard_conf.deploy_lifespan,
        on_chain_state.shard_conf.min_phlo_price,
    );
}

#[test]
fn no_finality_yet_is_a_noop() {
    let mut live = ingress(2);
    assert!(output(&mut live).anchor.is_none());
    assert_noop(&mut live);
}

#[test]
fn empty_bonds_are_a_noop() {
    let mut live = ingress(6);
    live.set_bonds(HashMap::new());
    assert_noop(&mut live);
}

#[test]
fn checkpoint_is_idempotent() {
    let mut live = ingress(6);
    let report = checkpoint(&mut live);
    assert_eq!(live.blocklace().checkpoint(), Some(&report.checkpoint));
    assert_noop(&mut live);
}

#[test]
fn active_future_epoch_is_a_noop() {
    // Eight rounds leave the first 16-block source incomplete; nine complete it.
    let mut short = ingress(8);
    assert!(
        finalized_prefix_checkpoint(&output(&mut short), 1)
            .unwrap()
            .is_none()
    );
    let mut live = ingress(9);
    let published = output(&mut live);
    let source = finalized_prefix_checkpoint(&published, 1).unwrap().unwrap();
    let wave = |id| wave_of_round(depth(live.blocklace(), id).unwrap(), W).unwrap();
    let delay = wave(published.anchor.as_ref().unwrap()) - wave(&source.terminal_block) + 1;
    let projected = (1..=4).map(|v| (NodeId(vec![v]), 50)).collect();
    let record = PorWeightEpochRecord::prepare(
        &live,
        &source,
        [42; 32],
        &projected,
        PorWeightEpochPolicy::new(delay).unwrap(),
    )
    .unwrap();
    live.install_future_epoch(record, [42; 32]).unwrap();
    // A shared head gives these snapshots one LCA without changing fork choice.
    let parents = stream(9)
        .into_iter()
        .rev()
        .take(4)
        .map(|b| b.identity)
        .collect();
    let head = make_block(1, 9, parents);
    let head_id = head.identity.clone();
    feed(&mut live, head);
    for v in 2..=4 {
        feed(
            &mut live,
            make_block(v, 10, HashSet::from([head_id.clone()])),
        );
    }
    let before = live.snapshot().unwrap();
    let hash = live.last_finalized_block_hash().unwrap();
    assert_noop(&mut live);
    assert_snapshot_eq(&before, &live.snapshot().unwrap());
    assert_eq!(live.last_finalized_block_hash().unwrap(), hash);
}
