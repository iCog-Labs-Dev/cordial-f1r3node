pub trait ConsensusEngine {
    type BlockId;
    type Error;

    fn on_block(&mut self, block_id: Self::BlockId) -> Result<(), Self::Error>;
    fn canonical_order_prefix(&self) -> Vec<Self::BlockId>;
}

pub trait BlockProvider<B, Id> {
    fn block(&self, id: &Id) -> Option<&B>;
    fn contains_block(&self, id: &Id) -> bool;
}

pub trait ValidatorSet<V> {
    type Weight;

    fn weight_of(&self, validator: &V) -> Option<Self::Weight>;
    fn total_weight(&self) -> Self::Weight;
}
