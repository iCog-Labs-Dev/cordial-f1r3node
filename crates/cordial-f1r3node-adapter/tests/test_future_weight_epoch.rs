use std::collections::{HashMap, HashSet};

use cordial_f1r3node_adapter::{
    grpc_ingest::BlocklaceAdapter,
    live_ingress::{LiveIngress, PorWeightActivationError},
    shard_conf::CasperShardConf,
    shared_ordered_output::ReadOrderedOutput,
};
use cordial_miners_core::{Block, BlockContent, BlockIdentity, NodeId};

struct AcceptBlocks;

impl BlocklaceAdapter<BlockIdentity> for AcceptBlocks {
    fn on_block(&mut self, _block: Block) -> anyhow::Result<()> {
        Ok(())
    }
}

fn node(id: u8) -> NodeId {
    NodeId(vec![id])
}

fn weights(entries: &[(u8, u64)]) -> HashMap<NodeId, u64> {
    entries
        .iter()
        .map(|(id, weight)| (node(*id), *weight))
        .collect()
}

fn block(creator: u8, hash: u8, predecessors: &[&Block]) -> Block {
    let mut content_hash = [0; 32];
    content_hash[0] = creator;
    content_hash[1] = hash;
    Block {
        identity: BlockIdentity {
            content_hash,
            creator: node(creator),
            signature: Vec::new(),
        },
        content: BlockContent {
            payload: Vec::new(),
            predecessors: predecessors
                .iter()
                .map(|block| block.identity.clone())
                .collect::<HashSet<_>>(),
        },
    }
}

fn ingest(ingress: &mut LiveIngress<AcceptBlocks>, blocks: &[&Block]) {
    for block in blocks {
        ingress.ingest_trusted_block((*block).clone()).unwrap();
    }
}

#[test]
fn preview_preserves_published_prefix_and_uses_new_weights_only_for_future_wave() {
    let old = weights(&[(1, 1), (2, 3), (3, 3), (4, 3), (5, 1)]);
    let new = weights(&[(1, 1), (2, 1), (3, 1), (4, 1), (5, 8)]);
    let mut ingress = LiveIngress::with_consensus_view(
        AcceptBlocks,
        old.clone(),
        CasperShardConf::default(),
        "root",
    );

    let w0 = block(1, 1, &[]);
    let r1_2 = block(2, 2, &[&w0]);
    let r1_3 = block(3, 3, &[&w0]);
    let r1_4 = block(4, 4, &[&w0]);
    let r2_2 = block(2, 5, &[&r1_2, &r1_3, &r1_4]);
    let r2_3 = block(3, 6, &[&r1_2, &r1_3, &r1_4]);
    let r2_4 = block(4, 7, &[&r1_2, &r1_3, &r1_4]);
    ingest(
        &mut ingress,
        &[&w0, &r1_2, &r1_3, &r1_4, &r2_2, &r2_3, &r2_4],
    );
    let sealed = ingress.latest_finalized_ordered_output(3).unwrap();
    assert_eq!(sealed.anchor, Some(w0.identity.clone()));

    let pending = ingress.preview_future_weight_epoch(&new, 1).unwrap();
    assert_eq!(pending.blocks, sealed.blocks);
    assert_eq!(pending.anchor, sealed.anchor);

    let w1 = block(2, 8, &[&r2_2, &r2_3, &r2_4]);
    let w1_r1_2 = block(2, 9, &[&w1]);
    let w1_r1_5 = block(5, 10, &[&w1]);
    let w1_r2_2 = block(2, 11, &[&w1_r1_2, &w1_r1_5]);
    let w1_r2_5 = block(5, 12, &[&w1_r1_2, &w1_r1_5]);
    ingest(&mut ingress, &[&w1, &w1_r1_2, &w1_r1_5, &w1_r2_2, &w1_r2_5]);

    let future = ingress.preview_future_weight_epoch(&new, 1).unwrap();
    assert!(future.blocks.starts_with(&sealed.blocks));
    assert!(future.blocks.len() > sealed.blocks.len());
    assert_eq!(future.anchor, Some(w1.identity.clone()));
    assert_eq!(ingress.bonds(), &old);
    assert_eq!(
        ingress.ordered_output_reader().latest().unwrap().blocks,
        sealed.blocks
    );
    assert_eq!(
        ingress.preview_future_weight_epoch(&new, 1).unwrap().blocks,
        future.blocks
    );
}

#[test]
fn preview_rejects_missing_boundary_past_wave_and_membership_change() {
    let old = weights(&[(1, 1), (2, 3), (3, 3), (4, 3)]);
    let new = weights(&[(1, 1), (2, 1), (3, 1), (4, 5)]);
    let mut ingress =
        LiveIngress::with_consensus_view(AcceptBlocks, old, CasperShardConf::default(), "root");
    assert_eq!(
        ingress.preview_future_weight_epoch(&new, 1).unwrap_err(),
        PorWeightActivationError::MissingEpochBoundary
    );

    let w0 = block(1, 1, &[]);
    let r1_2 = block(2, 2, &[&w0]);
    let r1_3 = block(3, 3, &[&w0]);
    let r1_4 = block(4, 4, &[&w0]);
    let r2_2 = block(2, 5, &[&r1_2, &r1_3, &r1_4]);
    let r2_3 = block(3, 6, &[&r1_2, &r1_3, &r1_4]);
    let r2_4 = block(4, 7, &[&r1_2, &r1_3, &r1_4]);
    ingest(
        &mut ingress,
        &[&w0, &r1_2, &r1_3, &r1_4, &r2_2, &r2_3, &r2_4],
    );
    ingress.latest_finalized_ordered_output(3).unwrap();
    assert_eq!(
        ingress.preview_future_weight_epoch(&new, 0).unwrap_err(),
        PorWeightActivationError::EffectiveWaveAlreadyPublished {
            effective_wave: 0,
            published_wave: 0,
        }
    );
    assert_eq!(
        ingress
            .preview_future_weight_epoch(&weights(&[(1, 1)]), 1)
            .unwrap_err(),
        PorWeightActivationError::ActivatedValidatorSetMismatch {
            configured: 4,
            recorded: 1,
        }
    );
}
