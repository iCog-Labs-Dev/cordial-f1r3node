use std::collections::HashSet;

use cordial_f1r3node_adapter::{
    grpc_ingest::BlocklaceAdapter,
    live_ingress::{LiveIngress, PorWeightActivationError},
    ordered_output::OrderedFinalizedOutput,
    por::{
        PorFinalityError, PorWeightEpochPolicy, PorWeightEpochPolicyError,
        finalized_prefix_checkpoint,
    },
    shard_conf::CasperShardConf,
};
use cordial_miners_core::{
    Block, BlockContent, BlockIdentity, Blocklace, NodeId,
    consensus::{depth, wave_of_round},
    crypto::CryptoVerifier,
};

struct AcceptBlocks;

impl CryptoVerifier for AcceptBlocks {
    type Error = &'static str;

    fn verify_block(
        &self,
        _content: &BlockContent,
        _signature: &[u8],
        _creator: &NodeId,
    ) -> Result<(), Self::Error> {
        Ok(())
    }
}

impl BlocklaceAdapter<BlockIdentity> for AcceptBlocks {
    fn on_block(&mut self, _block: Block) -> anyhow::Result<()> {
        Ok(())
    }
}

fn chain(length: u8) -> (Blocklace, Vec<BlockIdentity>) {
    let mut blocklace = Blocklace::new();
    let mut identities = Vec::new();
    for index in 0..length {
        let mut hash = [0; 32];
        hash[0] = index + 1;
        let identity = BlockIdentity {
            content_hash: hash,
            creator: NodeId(vec![1]),
            signature: Vec::new(),
        };
        let predecessors = identities
            .last()
            .cloned()
            .into_iter()
            .collect::<HashSet<_>>();
        blocklace
            .insert(
                Block {
                    identity: identity.clone(),
                    content: BlockContent {
                        payload: Vec::new(),
                        predecessors,
                    },
                },
                &AcceptBlocks,
            )
            .unwrap();
        identities.push(identity);
    }
    (blocklace, identities)
}

fn output(blocks: &[BlockIdentity]) -> OrderedFinalizedOutput {
    OrderedFinalizedOutput::new(blocks.to_vec(), blocks.last().cloned(), 3, 4, blocks.len())
}

#[test]
fn effective_wave_is_independent_of_later_local_head() {
    let (blocklace, blocks) = chain(20);
    let early = output(&blocks[..16]);
    let later = output(&blocks);
    let source = finalized_prefix_checkpoint(&early, 1).unwrap().unwrap();
    let policy = PorWeightEpochPolicy::new(4).unwrap();

    assert_eq!(policy.delay_waves(), 4);
    assert_eq!(
        policy.effective_wave(&blocklace, &early, &source).unwrap(),
        9
    );
    assert_eq!(
        policy.effective_wave(&blocklace, &later, &source).unwrap(),
        9
    );
}

#[test]
fn policy_rejects_zero_delay_changed_source_unknown_terminal_and_overflow() {
    assert_eq!(
        PorWeightEpochPolicy::new(0),
        Err(PorWeightEpochPolicyError::ZeroDelay)
    );
    let (blocklace, blocks) = chain(16);
    let published = output(&blocks);
    let source = finalized_prefix_checkpoint(&published, 1).unwrap().unwrap();

    let mut changed = source.clone();
    changed.prefix_hash[0] ^= 1;
    assert_eq!(
        PorWeightEpochPolicy::new(1)
            .unwrap()
            .effective_wave(&blocklace, &published, &changed),
        Err(PorWeightEpochPolicyError::Source(
            PorFinalityError::PrefixCheckpointMismatch
        ))
    );
    assert_eq!(
        PorWeightEpochPolicy::new(1).unwrap().effective_wave(
            &Blocklace::new(),
            &published,
            &source
        ),
        Err(PorWeightEpochPolicyError::UnknownTerminalBlock)
    );
    assert_eq!(
        PorWeightEpochPolicy::new(u64::MAX)
            .unwrap()
            .effective_wave(&blocklace, &published, &source),
        Err(PorWeightEpochPolicyError::WaveOverflow)
    );
}

#[test]
fn checkpoint_preview_waits_for_exact_boundary_and_rejects_late_cutover() {
    let validator = NodeId(vec![1]);
    let old = std::collections::HashMap::from([(validator.clone(), 10)]);
    let projected = std::collections::HashMap::from([(validator, 20)]);
    let mut ingress = LiveIngress::with_consensus_view(
        AcceptBlocks,
        old.clone(),
        CasperShardConf::default(),
        "root",
    );
    let mut previous: Option<Block> = None;
    for index in 0..30u8 {
        let mut hash = [0; 32];
        hash[0] = index + 1;
        let block = Block {
            identity: BlockIdentity {
                content_hash: hash,
                creator: NodeId(vec![1]),
                signature: Vec::new(),
            },
            content: BlockContent {
                payload: Vec::new(),
                predecessors: previous
                    .as_ref()
                    .map(|block| block.identity.clone())
                    .into_iter()
                    .collect(),
            },
        };
        ingress.ingest_trusted_block(block.clone()).unwrap();
        previous = Some(block);
    }
    let published = ingress.latest_finalized_ordered_output(3).unwrap();
    let source = finalized_prefix_checkpoint(&published, 1)
        .unwrap()
        .expect("one finalized checkpoint");
    let published_wave = wave_of_round(
        depth(ingress.blocklace(), published.anchor.as_ref().unwrap()).unwrap(),
        3,
    )
    .unwrap();

    let future_policy = PorWeightEpochPolicy::new(10).unwrap();
    let future_wave = future_policy
        .effective_wave(ingress.blocklace(), &published, &source)
        .unwrap();
    assert_eq!(
        ingress
            .preview_checkpoint_weight_epoch(&projected, &source, future_policy)
            .unwrap_err(),
        PorWeightActivationError::EpochBoundaryWavePending {
            required: future_wave - 1,
            published: published_wave,
        }
    );

    let late_policy = PorWeightEpochPolicy::new(1).unwrap();
    let late_wave = late_policy
        .effective_wave(ingress.blocklace(), &published, &source)
        .unwrap();
    assert!(published_wave >= late_wave);
    assert_eq!(
        ingress
            .preview_checkpoint_weight_epoch(&projected, &source, late_policy)
            .unwrap_err(),
        PorWeightActivationError::EffectiveWaveAlreadyPublished {
            effective_wave: late_wave,
            published_wave,
        }
    );
    let exact_delay = published_wave + 2 - late_wave;
    let (effective_wave, exact_preview) = ingress
        .preview_checkpoint_weight_epoch(
            &projected,
            &source,
            PorWeightEpochPolicy::new(exact_delay).unwrap(),
        )
        .unwrap();
    assert_eq!(effective_wave, published_wave + 1);
    assert_eq!(exact_preview.blocks, published.blocks);
    assert_eq!(ingress.bonds(), &old);
}
