use cordial_miners_core::cordiality::ConsensusEngine;

#[derive(Default)]
struct MockConsensusEngine {
    prefix: Vec<u64>,
}

impl ConsensusEngine for MockConsensusEngine {
    type BlockId = u64;
    type Error = ();

    fn on_block(&mut self, block_id: Self::BlockId) -> Result<(), Self::Error> {
        self.prefix.push(block_id);
        Ok(())
    }

    fn canonical_order_prefix(&self) -> Vec<Self::BlockId> {
        self.prefix.clone()
    }
}

fn node_runner<E>(engine: &mut E) -> Result<Vec<E::BlockId>, E::Error>
where
    E: ConsensusEngine,
    E::BlockId: Default,
{
    engine.on_block(E::BlockId::default())?;
    Ok(engine.canonical_order_prefix())
}

#[test]
fn mock_engine_can_be_used_through_generic_trait_bound() {
    let mut engine = MockConsensusEngine::default();
    let prefix = node_runner(&mut engine).expect("mock engine should not fail");

    assert_eq!(prefix, vec![0]);
}
