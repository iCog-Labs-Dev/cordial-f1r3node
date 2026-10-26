//! Strict authentication of retained, otherwise untrusted equivocation claims.
//!
//! The blocklace supplies trusted, complete ancestry, not authorization to punish.
//! Unlike normal test-friendly block admission, unsigned evidence is never valid.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

use crate::{Block, Blocklace, NodeId, consensus::evidence::CordialEquivocationEvidence, crypto};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EvidenceVerificationError {
    TooFewBlocks,
    MalformedEncoding,
    WrongCreator,
    InvalidHash,
    InvalidSignature,
    DuplicateBlock,
    InvalidBlock,
    MissingHistory,
    WrongRound,
    ComparableBlocks,
}

impl fmt::Display for EvidenceVerificationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid Cordial equivocation evidence: {self:?}")
    }
}

impl std::error::Error for EvidenceVerificationError {}

/// Authenticated conflicting blocks. No public constructor, mutable accessor,
/// or deserializer can manufacture this capability from a peer's claim.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VerifiedEquivocationEvidence {
    validator: NodeId,
    round: u64,
    blocks: Vec<Block>,
}

impl VerifiedEquivocationEvidence {
    pub fn validator(&self) -> &NodeId {
        &self.validator
    }

    /// Earliest depth among the conflicting blocks, derived from their ancestry.
    pub fn round(&self) -> u64 {
        self.round
    }

    pub fn blocks(&self) -> &[Block] {
        &self.blocks
    }

    /// Full, deterministic proof bytes, including signatures and signed content.
    /// Predecessor references use the same signature-independent identity as
    /// content hashing. HashSet iteration order never reaches the commitment.
    pub fn canonical_bytes(&self) -> Vec<u8> {
        fn put_bytes(output: &mut Vec<u8>, bytes: &[u8]) {
            output.extend_from_slice(&(bytes.len() as u64).to_be_bytes());
            output.extend_from_slice(bytes);
        }
        let mut bytes = b"cordial:verified-equivocation:v1".to_vec();
        put_bytes(&mut bytes, &self.validator.0);
        bytes.extend_from_slice(&self.round.to_be_bytes());
        bytes.extend_from_slice(&(self.blocks.len() as u64).to_be_bytes());
        for block in &self.blocks {
            bytes.extend_from_slice(&block.identity.content_hash);
            put_bytes(&mut bytes, &block.identity.creator.0);
            put_bytes(&mut bytes, &block.identity.signature);
            put_bytes(&mut bytes, &block.content.payload);
            let mut predecessors: Vec<_> = block.content.predecessors.iter().collect();
            predecessors.sort_by_key(|id| (id.content_hash, &id.creator));
            bytes.extend_from_slice(&(predecessors.len() as u64).to_be_bytes());
            for predecessor in predecessors {
                bytes.extend_from_slice(&predecessor.content_hash);
                put_bytes(&mut bytes, &predecessor.creator.0);
            }
        }
        bytes
    }
}

/// Verify both signed contents, distinct consensus identities, complete ancestry,
/// derived round, and pairwise incomparability. Signature variants of the same
/// content are not equivocation. Unknown/pruned ancestry fails closed.
///
/// The blocklace must be the caller's validated Cordial view. Membership and
/// finalized authorization belong to the consuming protocol's policy.
pub fn verify_equivocation_evidence(
    raw: &CordialEquivocationEvidence,
    blocklace: &Blocklace,
) -> Result<VerifiedEquivocationEvidence, EvidenceVerificationError> {
    use EvidenceVerificationError as Error;

    if raw.blocks.len() < 2 {
        return Err(Error::TooFewBlocks);
    }
    let mut identities = BTreeSet::new();
    let mut earliest_round = u64::MAX;
    let mut histories = Vec::new();
    for block in &raw.blocks {
        if block.identity.creator != raw.validator {
            return Err(Error::WrongCreator);
        }
        if crypto::block_hash(&block.identity.creator, &block.content)
            != block.identity.content_hash
        {
            return Err(Error::InvalidHash);
        }
        if !crypto::verify(
            &block.identity.content_hash,
            &raw.validator.0,
            &block.identity.signature,
        ) {
            return Err(Error::InvalidSignature);
        }
        if !identities.insert(block.identity.consensus_identity()) {
            return Err(Error::DuplicateBlock);
        }
        let (round, ancestors) = complete_history(block, blocklace)?;
        histories.push(ancestors);
        earliest_round = earliest_round.min(round);
        // Hash, signature and full closure were checked above. Do not run
        // insertion validation: a different valid signature already stored for
        // this consensus identity must not change whether this proof is valid.
    }
    if earliest_round != raw.round {
        return Err(Error::WrongRound);
    }
    for (index, left) in raw.blocks.iter().enumerate() {
        for (offset, right) in raw.blocks[index + 1..].iter().enumerate() {
            let right_index = index + 1 + offset;
            if histories[index].contains(&right.identity.consensus_identity())
                || histories[right_index].contains(&left.identity.consensus_identity())
            {
                return Err(Error::ComparableBlocks);
            }
        }
    }
    let mut blocks = raw.blocks.clone();
    blocks.sort_by(|left, right| left.identity.cmp(&right.identity));
    Ok(VerifiedEquivocationEvidence {
        validator: raw.validator.clone(),
        round: earliest_round,
        blocks,
    })
}

// Do not treat an unresolved/pruned ancestor as proof of non-comparability.
// Walk full signed-content ancestry, ignoring trusted checkpoint depth shortcuts.
fn complete_history(
    block: &Block,
    blocklace: &Blocklace,
) -> Result<(u64, BTreeSet<crate::BlockIdentity>), EvidenceVerificationError> {
    use EvidenceVerificationError as Error;

    let mut stack = vec![(block.clone(), false)];
    let mut rounds = BTreeMap::new();
    let mut visiting = BTreeSet::new();
    let mut ancestors = BTreeSet::new();
    while let Some((current, expanded)) = stack.pop() {
        let identity = current.identity.consensus_identity();
        if rounds.contains_key(&identity) {
            continue;
        }
        if crypto::block_hash(&current.identity.creator, &current.content)
            != current.identity.content_hash
        {
            return Err(Error::InvalidHash);
        }
        if expanded {
            let mut round = 0u64;
            for predecessor in &current.content.predecessors {
                let predecessor_round: u64 = *rounds
                    .get(&predecessor.consensus_identity())
                    .ok_or(Error::MissingHistory)?;
                round = round.max(
                    predecessor_round
                        .checked_add(1)
                        .ok_or(Error::InvalidBlock)?,
                );
            }
            visiting.remove(&identity);
            rounds.insert(identity, round);
        } else {
            if !visiting.insert(identity.clone()) {
                return Err(Error::InvalidBlock);
            }
            stack.push((current.clone(), true));
            for predecessor in &current.content.predecessors {
                ancestors.insert(predecessor.consensus_identity());
                let ancestor = blocklace
                    .get(&predecessor.consensus_identity())
                    .ok_or(Error::MissingHistory)?;
                stack.push((ancestor, false));
            }
        }
    }
    let round = rounds[&block.identity.consensus_identity()];
    Ok((round, ancestors))
}

/// Decode canonical proof bytes into an UNVERIFIED claim. This grants no
/// capability: callers must run verify_equivocation_evidence against their DAG.
pub fn decode_equivocation_evidence(
    bytes: &[u8],
) -> Result<CordialEquivocationEvidence, EvidenceVerificationError> {
    use EvidenceVerificationError::MalformedEncoding as Bad;
    struct Reader<'a>(&'a [u8]);
    impl<'a> Reader<'a> {
        fn take(&mut self, n: usize) -> Result<&'a [u8], EvidenceVerificationError> {
            if n > self.0.len() {
                return Err(Bad);
            }
            let (head, tail) = self.0.split_at(n);
            self.0 = tail;
            Ok(head)
        }
        fn number(&mut self) -> Result<u64, EvidenceVerificationError> {
            Ok(u64::from_be_bytes(
                self.take(8)?.try_into().map_err(|_| Bad)?,
            ))
        }
        fn bytes(&mut self) -> Result<Vec<u8>, EvidenceVerificationError> {
            let n = usize::try_from(self.number()?).map_err(|_| Bad)?;
            Ok(self.take(n)?.to_vec())
        }
        fn count(&mut self) -> Result<usize, EvidenceVerificationError> {
            let n = usize::try_from(self.number()?).map_err(|_| Bad)?;
            if n > self.0.len() / 32 {
                return Err(Bad);
            }
            Ok(n)
        }
    }
    let mut r = Reader(
        bytes
            .strip_prefix(b"cordial:verified-equivocation:v1")
            .ok_or(Bad)?,
    );
    let validator = NodeId(r.bytes()?);
    let round = r.number()?;
    let count = r.count()?;
    let mut blocks = Vec::new();
    for _ in 0..count {
        let content_hash = r.take(32)?.try_into().map_err(|_| Bad)?;
        let creator = NodeId(r.bytes()?);
        let signature = r.bytes()?;
        let payload = r.bytes()?;
        let mut predecessors = std::collections::HashSet::new();
        for _ in 0..r.count()? {
            let identity = crate::BlockIdentity {
                content_hash: r.take(32)?.try_into().map_err(|_| Bad)?,
                creator: NodeId(r.bytes()?),
                signature: vec![],
            };
            if !predecessors.insert(identity) {
                return Err(Bad);
            }
        }
        blocks.push(Block {
            identity: crate::BlockIdentity {
                content_hash,
                creator,
                signature,
            },
            content: crate::BlockContent {
                payload,
                predecessors,
            },
        });
    }
    if !r.0.is_empty() {
        return Err(Bad);
    }
    Ok(CordialEquivocationEvidence::new(validator, round, blocks))
}
