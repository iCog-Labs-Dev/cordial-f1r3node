//! Equivocation and cordiality predicates for Cordial Miners.
//!
//! This module gathers the protocol-facing DAG predicates that sit between the
//! structural helpers (`round`, `wave`) and the enforcement layer
//! (`validation`).
//!
//! The paper distinguishes:
//! - equivocation: a validator produces multiple conflicting blocks
//! - cordiality: a block does not hide relevant information from the DAG view
//!
//! In this implementation, the "known" portion of "known equivocations" is
//! interpreted conservatively as "already present in the local blocklace".
//! That makes these predicates usable inside block validation, where the
//! creator's private local view is not available.

use std::collections::{HashMap, HashSet};

use crate::block::Block;
use crate::blocklace::Blocklace;
use crate::consensus::round::{blocks_at_depth, depth};
use crate::types::{BlockIdentity, NodeId};

/// A same-round equivocation detected in the blocklace.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Equivocation {
    pub creator: NodeId,
    pub round: u64,
    pub blocks: Vec<BlockIdentity>
}