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

/// Leader-selection policy applied to one authorized weight snapshot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LeaderSelectionPolicy {
    /// Existing Cordial behavior: every authorized key appears once in
    /// canonical `NodeId` order, including zero-weight keys.
    RoundRobin,
    /// Bounded deterministic schedule. Every positive-weight key receives one
    /// baseline slot and remaining slots are distributed proportionally.
    PorWeightedFair { schedule_length: usize },
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
            LeaderSelectionPolicy::PorWeightedFair { .. } => usize::try_from(offset).ok()?,
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
) -> Result<LeaderSelectionComparison, LeaderScheduleError> {
    let round_robin =
        LeaderSchedule::from_weights(LeaderSelectionPolicy::RoundRobin, effective_wave, weights)?;
    let por_weighted_fair = LeaderSchedule::from_weights(
        LeaderSelectionPolicy::PorWeightedFair {
            schedule_length: por_schedule_length,
        },
        effective_wave,
        weights,
    )?;

    Ok(LeaderSelectionComparison {
        wave,
        round_robin_leader: round_robin.leader_for_wave(wave),
        por_weighted_fair_leader: por_weighted_fair.leader_for_wave(wave),
    })
}

fn weighted_fair_schedule(
    weights: &HashMap<NodeId, u64>,
    schedule_length: usize,
) -> Result<Vec<NodeId>, LeaderScheduleError> {
    if schedule_length > MAX_POR_LEADER_SCHEDULE_LENGTH {
        return Err(LeaderScheduleError::ScheduleTooLong {
            schedule_length,
            max_schedule_length: MAX_POR_LEADER_SCHEDULE_LENGTH,
        });
    }

    let mut eligible: Vec<_> = weights
        .iter()
        .filter(|(_, weight)| **weight > 0)
        .map(|(node_id, weight)| (node_id.clone(), *weight))
        .collect();
    eligible.sort_by(|left, right| left.0.cmp(&right.0));

    if eligible.is_empty() {
        return Err(LeaderScheduleError::ZeroActiveReputation);
    }
    if schedule_length < eligible.len() {
        return Err(LeaderScheduleError::ScheduleTooShort {
            schedule_length,
            eligible_validators: eligible.len(),
        });
    }

    let total_weight = eligible.iter().try_fold(0u128, |total, (_, weight)| {
        total
            .checked_add(u128::from(*weight))
            .ok_or(LeaderScheduleError::ArithmeticOverflow)
    })?;
    let remaining_slots = schedule_length - eligible.len();
    let remaining_slots =
        u128::try_from(remaining_slots).map_err(|_| LeaderScheduleError::ArithmeticOverflow)?;

    // Largest-remainder apportionment gives every eligible validator one
    // liveness slot, then allocates the bounded remainder proportionally.
    let mut quotas = Vec::with_capacity(eligible.len());
    let mut assigned = 0usize;
    for (node_id, weight) in eligible {
        let numerator = u128::from(weight)
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
        quotas.push((node_id, quota, numerator % total_weight));
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
