//! Deterministic leader schedules for comparing Cordial round robin with a
//! bounded Proof-of-Reputation weighted-fair rotation.
//!
//! This module only constructs and queries schedules. It does not activate an
//! epoch or decide finality. Callers must bind a schedule to an agreed future
//! `effective_wave` and retain older schedules for historical replay.

use std::{
    cmp::Ordering,
    collections::{BinaryHeap, HashMap},
};

use cordial_miners_core::NodeId;
use thiserror::Error;

/// Upper bound for materialized comparison schedules.
pub const MAX_POR_LEADER_SCHEDULE_LENGTH: usize = 65_536;
pub const DEFAULT_POR_LEADER_THRESHOLD_NUMERATOR: u64 = 1;
pub const DEFAULT_POR_LEADER_THRESHOLD_DENOMINATOR: u64 = 2;

/// Strict fraction of total active reputation required for the leader set.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReputationThreshold {
    numerator: u64,
    denominator: u64,
}

impl ReputationThreshold {
    pub fn new(numerator: u64, denominator: u64) -> Result<Self, LeaderScheduleError> {
        if numerator == 0 || denominator == 0 || numerator >= denominator {
            return Err(LeaderScheduleError::InvalidCandidateThreshold {
                numerator,
                denominator,
            });
        }
        Ok(Self {
            numerator,
            denominator,
        })
    }

    pub fn numerator(self) -> u64 {
        self.numerator
    }

    pub fn denominator(self) -> u64 {
        self.denominator
    }
}

impl Default for ReputationThreshold {
    fn default() -> Self {
        Self {
            numerator: DEFAULT_POR_LEADER_THRESHOLD_NUMERATOR,
            denominator: DEFAULT_POR_LEADER_THRESHOLD_DENOMINATOR,
        }
    }
}

/// One Cordial-authorized validator admitted to the PoR leader-candidate set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReputationLeaderCandidate {
    pub node_id: NodeId,
    pub reputation: u64,
}

/// Leader-selection policy applied to one authorized weight snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaderSelectionPolicy {
    /// Existing Cordial behavior: every authorized key appears once in
    /// canonical `NodeId` order, including zero-weight keys.
    RoundRobin,
    /// Bounded deterministic schedule. Every positive-weight key receives one
    /// baseline slot and remaining slots are distributed proportionally.
    PorWeightedFair { schedule_length: usize },
    /// Weighted-fair schedule restricted to the smallest highest-reputation
    /// prefix that strictly exceeds `candidate_threshold`.
    PorThresholdWeightedFair {
        schedule_length: usize,
        candidate_threshold: ReputationThreshold,
    },
}

/// A leader schedule that becomes valid at one protocol-agreed wave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaderSchedule {
    policy: LeaderSelectionPolicy,
    effective_wave: u64,
    leaders: Vec<NodeId>,
}

/// Non-authoritative result for comparing both policies at one wave.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LeaderSelectionComparison {
    pub wave: u64,
    pub round_robin_leader: Option<NodeId>,
    pub por_weighted_fair_leader: Option<NodeId>,
    pub por_leader_candidates: Vec<ReputationLeaderCandidate>,
}

impl LeaderSelectionComparison {
    pub fn differs(&self) -> bool {
        self.round_robin_leader != self.por_weighted_fair_leader
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LeaderScheduleError {
    #[error("leader selection requires at least one authorized validator")]
    EmptyAuthorizedValidatorSet,
    #[error("PoR leader selection requires positive active reputation weight")]
    ZeroActiveReputation,
    #[error(
        "PoR leader candidate threshold must satisfy 0 < numerator < denominator, got {numerator}/{denominator}"
    )]
    InvalidCandidateThreshold { numerator: u64, denominator: u64 },
    #[error(
        "PoR schedule length {schedule_length} is smaller than its {eligible_validators} eligible validators"
    )]
    ScheduleTooShort {
        schedule_length: usize,
        eligible_validators: usize,
    },
    #[error("PoR schedule length {schedule_length} exceeds the maximum {max_schedule_length}")]
    ScheduleTooLong {
        schedule_length: usize,
        max_schedule_length: usize,
    },
    #[error("PoR leader schedule arithmetic overflowed")]
    ArithmeticOverflow,
}

impl LeaderSchedule {
    /// Construct a canonical schedule from Cordial's complete authorized weight
    /// map. Membership remains owned by Cordial; the PoR policy only excludes
    /// zero-weight keys from leadership eligibility.
    pub fn from_weights(
        policy: LeaderSelectionPolicy,
        effective_wave: u64,
        weights: &HashMap<NodeId, u64>,
    ) -> Result<Self, LeaderScheduleError> {
        if weights.is_empty() {
            return Err(LeaderScheduleError::EmptyAuthorizedValidatorSet);
        }

        let leaders = match policy {
            LeaderSelectionPolicy::RoundRobin => {
                let mut validators: Vec<_> = weights.keys().cloned().collect();
                validators.sort();
                validators
            }
            LeaderSelectionPolicy::PorWeightedFair { schedule_length } => {
                weighted_fair_schedule(weights, schedule_length)?
            }
            LeaderSelectionPolicy::PorThresholdWeightedFair {
                schedule_length,
                candidate_threshold,
            } => {
                validate_schedule_length_limit(schedule_length)?;
                let candidates = select_reputation_leader_candidates(weights, candidate_threshold)?;
                weighted_fair_candidate_schedule(&candidates, schedule_length)?
            }
        };

        Ok(Self {
            policy,
            effective_wave,
            leaders,
        })
    }

    pub fn policy(&self) -> LeaderSelectionPolicy {
        self.policy
    }

    pub fn effective_wave(&self) -> u64 {
        self.effective_wave
    }

    pub fn leaders(&self) -> &[NodeId] {
        &self.leaders
    }

    /// Select the leader for `wave`. Waves before this schedule's activation
    /// must be resolved by an older schedule epoch.
    pub fn leader_for_wave(&self, wave: u64) -> Option<NodeId> {
        let offset = wave.checked_sub(self.effective_wave)?;
        let schedule_index = match self.policy {
            // Preserve the existing global wave-indexed rotation exactly.
            LeaderSelectionPolicy::RoundRobin => usize::try_from(wave).ok()?,
            LeaderSelectionPolicy::PorWeightedFair { .. }
            | LeaderSelectionPolicy::PorThresholdWeightedFair { .. } => {
                usize::try_from(offset).ok()?
            }
        };
        Some(self.leaders[schedule_index % self.leaders.len()].clone())
    }
}

/// Compare both policies without changing the selector used by consensus.
pub fn compare_leader_selection(
    weights: &HashMap<NodeId, u64>,
    effective_wave: u64,
    wave: u64,
    por_schedule_length: usize,
    candidate_threshold: ReputationThreshold,
) -> Result<LeaderSelectionComparison, LeaderScheduleError> {
    validate_schedule_length_limit(por_schedule_length)?;
    let round_robin =
        LeaderSchedule::from_weights(LeaderSelectionPolicy::RoundRobin, effective_wave, weights)?;
    let por_leader_candidates = select_reputation_leader_candidates(weights, candidate_threshold)?;
    let por_weighted_fair = LeaderSchedule {
        policy: LeaderSelectionPolicy::PorThresholdWeightedFair {
            schedule_length: por_schedule_length,
            candidate_threshold,
        },
        effective_wave,
        leaders: weighted_fair_candidate_schedule(&por_leader_candidates, por_schedule_length)?,
    };

    Ok(LeaderSelectionComparison {
        wave,
        round_robin_leader: round_robin.leader_for_wave(wave),
        por_weighted_fair_leader: por_weighted_fair.leader_for_wave(wave),
        por_leader_candidates,
    })
}

/// Derive a deterministic leader-candidate subset without changing Cordial
/// membership. Zero-weight validators are ineligible. Candidates are returned
/// in reputation-descending order with `NodeId` as the exact-tie breaker.
pub fn select_reputation_leader_candidates(
    weights: &HashMap<NodeId, u64>,
    threshold: ReputationThreshold,
) -> Result<Vec<ReputationLeaderCandidate>, LeaderScheduleError> {
    if weights.is_empty() {
        return Err(LeaderScheduleError::EmptyAuthorizedValidatorSet);
    }

    let mut ranked: Vec<_> = weights
        .iter()
        .filter(|(_, weight)| **weight > 0)
        .map(|(node_id, reputation)| ReputationLeaderCandidate {
            node_id: node_id.clone(),
            reputation: *reputation,
        })
        .collect();
    if ranked.is_empty() {
        return Err(LeaderScheduleError::ZeroActiveReputation);
    }
    ranked.sort_by(|left, right| {
        right
            .reputation
            .cmp(&left.reputation)
            .then_with(|| left.node_id.cmp(&right.node_id))
    });

    let total_reputation = ranked.iter().try_fold(0u128, |total, candidate| {
        total
            .checked_add(u128::from(candidate.reputation))
            .ok_or(LeaderScheduleError::ArithmeticOverflow)
    })?;
    let required_reputation = strict_required_reputation(total_reputation, threshold)?;
    let mut selected_reputation = 0u128;
    let mut selected = Vec::new();
    for candidate in ranked {
        selected_reputation = selected_reputation
            .checked_add(u128::from(candidate.reputation))
            .ok_or(LeaderScheduleError::ArithmeticOverflow)?;
        selected.push(candidate);
        if selected_reputation >= required_reputation {
            break;
        }
    }

    Ok(selected)
}

fn strict_required_reputation(
    total_reputation: u128,
    threshold: ReputationThreshold,
) -> Result<u128, LeaderScheduleError> {
    let numerator = u128::from(threshold.numerator());
    let denominator = u128::from(threshold.denominator());
    let whole = (total_reputation / denominator)
        .checked_mul(numerator)
        .ok_or(LeaderScheduleError::ArithmeticOverflow)?;
    let remainder = (total_reputation % denominator)
        .checked_mul(numerator)
        .ok_or(LeaderScheduleError::ArithmeticOverflow)?
        / denominator;
    whole
        .checked_add(remainder)
        .and_then(|required| required.checked_add(1))
        .ok_or(LeaderScheduleError::ArithmeticOverflow)
}

fn weighted_fair_schedule(
    weights: &HashMap<NodeId, u64>,
    schedule_length: usize,
) -> Result<Vec<NodeId>, LeaderScheduleError> {
    validate_schedule_length_limit(schedule_length)?;
    let eligible: Vec<_> = weights
        .iter()
        .filter(|(_, weight)| **weight > 0)
        .map(|(node_id, reputation)| ReputationLeaderCandidate {
            node_id: node_id.clone(),
            reputation: *reputation,
        })
        .collect();
    weighted_fair_candidate_schedule(&eligible, schedule_length)
}

fn weighted_fair_candidate_schedule(
    candidates: &[ReputationLeaderCandidate],
    schedule_length: usize,
) -> Result<Vec<NodeId>, LeaderScheduleError> {
    validate_schedule_length_limit(schedule_length)?;
    if candidates.is_empty() {
        return Err(LeaderScheduleError::ZeroActiveReputation);
    }
    if schedule_length < candidates.len() {
        return Err(LeaderScheduleError::ScheduleTooShort {
            schedule_length,
            eligible_validators: candidates.len(),
        });
    }

    let mut eligible = candidates.to_vec();
    eligible.sort_by(|left, right| left.node_id.cmp(&right.node_id));
    let total_weight = eligible.iter().try_fold(0u128, |total, candidate| {
        total
            .checked_add(u128::from(candidate.reputation))
            .ok_or(LeaderScheduleError::ArithmeticOverflow)
    })?;
    let remaining_slots = schedule_length - eligible.len();
    let remaining_slots =
        u128::try_from(remaining_slots).map_err(|_| LeaderScheduleError::ArithmeticOverflow)?;

    // Largest-remainder apportionment gives every eligible validator one
    // liveness slot, then allocates the bounded remainder proportionally.
    let mut quotas = Vec::with_capacity(eligible.len());
    let mut assigned = 0usize;
    for candidate in eligible {
        let numerator = u128::from(candidate.reputation)
            .checked_mul(remaining_slots)
            .ok_or(LeaderScheduleError::ArithmeticOverflow)?;
        let extra = usize::try_from(numerator / total_weight)
            .map_err(|_| LeaderScheduleError::ArithmeticOverflow)?;
        let quota = 1usize
            .checked_add(extra)
            .ok_or(LeaderScheduleError::ArithmeticOverflow)?;
        assigned = assigned
            .checked_add(quota)
            .ok_or(LeaderScheduleError::ArithmeticOverflow)?;
        quotas.push((candidate.node_id, quota, numerator % total_weight));
    }

    let leftovers = schedule_length
        .checked_sub(assigned)
        .ok_or(LeaderScheduleError::ArithmeticOverflow)?;
    let mut remainder_order: Vec<_> = (0..quotas.len()).collect();
    remainder_order.sort_by(|left, right| {
        quotas[*right]
            .2
            .cmp(&quotas[*left].2)
            .then_with(|| quotas[*left].0.cmp(&quotas[*right].0))
    });
    for index in remainder_order.into_iter().take(leftovers) {
        quotas[index].1 = quotas[index]
            .1
            .checked_add(1)
            .ok_or(LeaderScheduleError::ArithmeticOverflow)?;
    }

    weighted_fair_rotation(&quotas, schedule_length)
}

fn validate_schedule_length_limit(schedule_length: usize) -> Result<(), LeaderScheduleError> {
    if schedule_length > MAX_POR_LEADER_SCHEDULE_LENGTH {
        return Err(LeaderScheduleError::ScheduleTooLong {
            schedule_length,
            max_schedule_length: MAX_POR_LEADER_SCHEDULE_LENGTH,
        });
    }
    Ok(())
}

fn weighted_fair_rotation(
    quotas: &[(NodeId, usize, u128)],
    schedule_length: usize,
) -> Result<Vec<NodeId>, LeaderScheduleError> {
    let mut pending: BinaryHeap<FairSlot> = quotas
        .iter()
        .map(|(node_id, quota, _)| FairSlot {
            node_id: node_id.clone(),
            quota: *quota,
            emitted: 0,
        })
        .collect();
    let mut schedule = Vec::with_capacity(schedule_length);

    for _ in 0..schedule_length {
        let mut selected = pending
            .pop()
            .ok_or(LeaderScheduleError::ZeroActiveReputation)?;
        schedule.push(selected.node_id.clone());
        selected.emitted = selected
            .emitted
            .checked_add(1)
            .ok_or(LeaderScheduleError::ArithmeticOverflow)?;
        if selected.emitted < selected.quota {
            pending.push(selected);
        }
    }

    Ok(schedule)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FairSlot {
    node_id: NodeId,
    quota: usize,
    emitted: usize,
}

impl Ord for FairSlot {
    fn cmp(&self, other: &Self) -> Ordering {
        let self_next = (self.emitted as u128 + 1) * other.quota as u128;
        let other_next = (other.emitted as u128 + 1) * self.quota as u128;

        // `BinaryHeap` pops the greatest item. Reverse the rational comparison
        // so the earliest virtual finish wins, then the smaller NodeId.
        other_next
            .cmp(&self_next)
            .then_with(|| other.node_id.cmp(&self.node_id))
            .then_with(|| self.quota.cmp(&other.quota))
            .then_with(|| self.emitted.cmp(&other.emitted))
    }
}

impl PartialOrd for FairSlot {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
