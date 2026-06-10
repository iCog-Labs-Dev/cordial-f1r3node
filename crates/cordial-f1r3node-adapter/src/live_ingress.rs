//! Live ingress scaffolding for Cordial Miners integration with f1r3node.
//!
//! This module is the adapter-side home for the "live interception" path.
//! It is intentionally narrow in scope for now: the first step is to make
//! the responsibility boundary explicit before wiring real transport traffic.
//!
//! ## Responsibility
//!
//! `live_ingress` is where runtime-facing integration should live once we
//! connect the adapter crate to real `f1r3node` message flow.
//!
//! In future steps this module will be responsible for:
//!
//! - receiving live block-bearing messages from a host node
//! - translating them through existing adapter logic
//! - feeding translated blocks into a local Cordial blocklace mirror
//! - exposing enough state for snapshot, finality, and ordering work
//!
//! ## Non-goals in this first step
//!
//! This module does **not** yet:
//!
//! - attach to a real gRPC or transport server
//! - maintain a full blocklace mirror
//! - run finality or tau ordering
//! - compare against HTTP-visible node state
//!
//! Those come in follow-up increments.

/// High-level runtime phase for the live ingress adapter.
///
/// Keeping this explicit helps later commits avoid mixing "discovery only"
/// state with fully wired live ingestion state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum LiveIngressPhase {
    /// Module exists, but no live runtime path is attached yet.
    #[default]
    Defined,
    /// A host-side ingress seam has been identified and documented.
    Traced,
    /// Live block messages are being accepted by the adapter boundary.
    Connected,
}

/// Minimal adapter-side entry point for future live interception work.
///
/// The generic `A` is the stateful adapter/runtime component that later
/// commits will use to store mirrored state, update snapshots, or expose
/// ordered output. This first increment keeps the shape intentionally small.
#[derive(Debug)]
pub struct LiveIngress<A> {
    phase: LiveIngressPhase,
    adapter: A,
}

impl<A> LiveIngress<A> {
    /// Create a new live-ingress wrapper around an adapter-side component.
    pub fn new(adapter: A) -> Self {
        Self {
            phase: LiveIngressPhase::Defined,
            adapter,
        }
    }

    /// Return the current runtime phase of the live ingress component.
    pub fn phase(&self) -> LiveIngressPhase {
        self.phase
    }

    /// Mark that the host ingress seam has been traced and identified.
    pub fn mark_traced(&mut self) {
        self.phase = LiveIngressPhase::Traced;
    }

    /// Mark that live block-bearing traffic is attached to this adapter.
    pub fn mark_connected(&mut self) {
        self.phase = LiveIngressPhase::Connected;
    }

    /// Borrow the wrapped adapter/runtime component.
    pub fn adapter(&self) -> &A {
        &self.adapter
    }

    /// Mutably borrow the wrapped adapter/runtime component.
    pub fn adapter_mut(&mut self) -> &mut A {
        &mut self.adapter
    }

    /// Consume the wrapper and return the inner adapter/runtime component.
    pub fn into_inner(self) -> A {
        self.adapter
    }
}
