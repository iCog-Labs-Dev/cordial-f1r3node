use std::any::Any;
use std::str::FromStr;

use clap::ValueEnum;
use cordial_miners_core::cordiality::ConsensusEngine;

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
#[value(rename_all = "kebab-case")]
pub enum ConsensusKind {
    Legacy,
    CordialMiners,
}

impl Default for ConsensusKind {
    fn default() -> Self {
        Self::Legacy
    }
}

impl FromStr for ConsensusKind {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s {
            "legacy" => Ok(Self::Legacy),
            "cordial-miners" => Ok(Self::CordialMiners),
            _ => Err(format!("unsupported consensus kind: {s}")),
        }
    }
}

pub trait AdapterConsensusEngine:
    ConsensusEngine<BlockId = Vec<u8>, Error = AdapterError> + Any
{
    fn as_any(&self) -> &dyn Any;
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct AdapterError;

#[derive(Debug, Default)]
pub struct LegacyStubAdapter {
    canonical_prefix: Vec<Vec<u8>>,
}

impl ConsensusEngine for LegacyStubAdapter {
    type BlockId = Vec<u8>;
    type Error = AdapterError;

    fn on_block(&mut self, block_id: Self::BlockId) -> Result<(), Self::Error> {
        self.canonical_prefix.push(block_id);
        Ok(())
    }

    fn canonical_order_prefix(&self) -> Vec<Self::BlockId> {
        self.canonical_prefix.clone()
    }
}

impl AdapterConsensusEngine for LegacyStubAdapter {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug, Default)]
pub struct CordialStubAdapter {
    canonical_prefix: Vec<Vec<u8>>,
}

impl ConsensusEngine for CordialStubAdapter {
    type BlockId = Vec<u8>;
    type Error = AdapterError;

    fn on_block(&mut self, block_id: Self::BlockId) -> Result<(), Self::Error> {
        self.canonical_prefix.push(block_id);
        Ok(())
    }

    fn canonical_order_prefix(&self) -> Vec<Self::BlockId> {
        self.canonical_prefix.clone()
    }
}

impl AdapterConsensusEngine for CordialStubAdapter {
    fn as_any(&self) -> &dyn Any {
        self
    }
}

pub trait ConsensusFactory {
    fn build(&self, kind: ConsensusKind) -> Box<dyn AdapterConsensusEngine>;
}

#[derive(Debug)]
pub struct DefaultConsensusFactory;

impl Default for DefaultConsensusFactory {
    fn default() -> Self {
        Self
    }
}

impl ConsensusFactory for DefaultConsensusFactory {
    fn build(&self, kind: ConsensusKind) -> Box<dyn AdapterConsensusEngine> {
        match kind {
            ConsensusKind::Legacy => Box::new(LegacyStubAdapter::default()),
            ConsensusKind::CordialMiners => Box::new(CordialStubAdapter::default()),
        }
    }
}

pub struct NodeStartup {
    pub engine: Box<dyn AdapterConsensusEngine>,
}

pub fn start_node_with_consensus<F>(factory: &F, kind: ConsensusKind) -> NodeStartup
where
    F: ConsensusFactory,
{
    let engine = factory.build(kind);
    NodeStartup { engine }
}

#[cfg(test)]
mod tests {
    use super::*;
    use clap::Parser;

    #[derive(Debug, Parser)]
    struct CliArgs {
        #[arg(long, value_enum, default_value_t = ConsensusKind::Legacy)]
        consensus: ConsensusKind,
    }

    #[test]
    fn parses_consensus_flag_for_cordial_miners() {
        let args = CliArgs::parse_from(["cordial-node", "--consensus", "cordial-miners"]);
        assert_eq!(args.consensus, ConsensusKind::CordialMiners);
    }

    #[test]
    fn factory_builds_legacy_engine_when_legacy_selected() {
        let factory = DefaultConsensusFactory;
        let engine = factory.build(ConsensusKind::Legacy);
        assert!(engine.as_any().is::<LegacyStubAdapter>());
    }

    #[test]
    fn factory_builds_cordial_engine_when_cordial_selected() {
        let factory = DefaultConsensusFactory;
        let engine = factory.build(ConsensusKind::CordialMiners);
        assert!(engine.as_any().is::<CordialStubAdapter>());
    }

    #[test]
    fn startup_with_cordial_mode_returns_cordial_stub() {
        let factory = DefaultConsensusFactory;
        let startup = start_node_with_consensus(&factory, ConsensusKind::CordialMiners);
        assert!(startup.engine.as_any().is::<CordialStubAdapter>());
    }

    #[test]
    fn cordial_stub_engine_accepts_blocks_without_error() {
        let mut engine = CordialStubAdapter::default();
        assert!(engine.on_block(vec![1, 2, 3]).is_ok());
        assert_eq!(engine.canonical_order_prefix(), vec![vec![1, 2, 3]]);
    }
}
