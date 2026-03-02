use serde::{Deserialize, Serialize};
use std::fmt;

/// Opaque identifier for a signal. Carries no semantic meaning.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize, PartialOrd, Ord)]
pub struct SignalId(pub u32);

impl fmt::Display for SignalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "s_{:04}", self.0)
    }
}

/// Classification of a signal's role in the system.
/// These are structural roles only — no semantic meaning is assigned.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SignalClass {
    /// Raw internal state variable
    Somatic,
    /// Computed from a combination of other signals
    Derived,
    /// Fed back from the output of an executed action
    Efferent,
    /// Existential continuity signal — asymmetric cost function applies
    Continuity,
    /// Input polled from the external environment (OS metrics) — read-only from agent's perspective
    World,
}

/// A single signal in the system.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Signal {
    pub id: SignalId,
    pub class: SignalClass,
    /// Current value
    pub value: f64,
    /// The resting point this signal decays toward
    pub baseline: f64,
    /// Per-tick decay rate toward baseline (0.0 = no decay, 1.0 = instant reset)
    pub decay_rate: f64,
    /// Scoring weight in the imbalance function
    pub weight: f64,
}

impl Signal {
    pub fn new(id: SignalId, class: SignalClass, baseline: f64, decay_rate: f64, weight: f64) -> Self {
        Self {
            id,
            class,
            value: baseline,
            baseline,
            decay_rate,
            weight,
        }
    }

    /// Apply one tick of decay toward baseline.
    pub fn decay(&mut self) {
        let delta = self.baseline - self.value;
        self.value += delta * self.decay_rate;
    }

    /// Deviation from baseline.
    pub fn deviation(&self) -> f64 {
        self.value - self.baseline
    }

    /// Apply a delta to this signal.
    pub fn apply_delta(&mut self, delta: f64) {
        self.value += delta;
    }
}

/// A timestamped snapshot of all signal values at a single tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalSnapshot {
    pub tick: u64,
    pub timestamp_ms: i64,
    /// Ordered by SignalId
    pub values: Vec<(SignalId, f64)>,
    /// Imbalance score at this tick
    pub imbalance: f64,
}

/// A delta event: a signal changed by this amount at this tick.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SignalDelta {
    pub tick: u64,
    pub id: SignalId,
    pub delta: f64,
    pub source: DeltaSource,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DeltaSource {
    Action(u32),
    External,
    Derived,
    Decay,
}
