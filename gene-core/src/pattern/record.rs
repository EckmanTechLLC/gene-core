use crate::signal::types::SignalId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// The temporal shape of a signal within a pattern window.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TemporalShape {
    Rising,
    Falling,
    Plateau,
    Spiking,
}

/// A compact summary of a recurring signal co-activation cluster.
/// No human-assigned meaning. Identified by a structural hash.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatternRecord {
    /// Structural identifier — hash of signal set + magnitude bucket
    pub id: u64,
    /// Which signals were co-active (above deviation threshold)
    pub signal_set: Vec<SignalId>,
    /// Mean magnitude of each signal's deviation in this pattern
    pub mean_magnitudes: HashMap<SignalId, f64>,
    /// Temporal shape per signal
    pub shapes: HashMap<SignalId, TemporalShape>,
    /// How many times this pattern has been observed
    pub frequency: u32,
    /// Mean imbalance score when this pattern was active
    pub mean_imbalance: f64,
    /// Action IDs that co-occurred with this pattern (and their counts)
    pub co_actions: HashMap<u32, u32>,
    /// Tick of first observation
    pub first_seen: u64,
    /// Tick of most recent observation
    pub last_seen: u64,
}

impl PatternRecord {
    pub fn new(id: u64, signal_set: Vec<SignalId>, tick: u64) -> Self {
        Self {
            id,
            signal_set,
            mean_magnitudes: HashMap::new(),
            shapes: HashMap::new(),
            frequency: 1,
            mean_imbalance: 0.0,
            co_actions: HashMap::new(),
            first_seen: tick,
            last_seen: tick,
        }
    }

    pub fn merge_observation(
        &mut self,
        magnitudes: &HashMap<SignalId, f64>,
        shapes: &HashMap<SignalId, TemporalShape>,
        imbalance: f64,
        action_id: Option<u32>,
        tick: u64,
    ) {
        self.frequency += 1;
        self.last_seen = tick;

        // Welford update for mean magnitudes
        for (id, mag) in magnitudes {
            let entry = self.mean_magnitudes.entry(*id).or_insert(0.0);
            *entry += (mag - *entry) / self.frequency as f64;
        }

        // Update shapes (most recent wins for simplicity)
        for (id, shape) in shapes {
            self.shapes.insert(*id, shape.clone());
        }

        // Running mean imbalance
        self.mean_imbalance += (imbalance - self.mean_imbalance) / self.frequency as f64;

        if let Some(aid) = action_id {
            *self.co_actions.entry(aid).or_insert(0) += 1;
        }
    }

    /// Cosine-like similarity between this pattern's signal set and another.
    /// Returns 0.0–1.0.
    pub fn similarity(&self, other: &PatternRecord) -> f64 {
        let set_a: std::collections::HashSet<_> = self.signal_set.iter().collect();
        let set_b: std::collections::HashSet<_> = other.signal_set.iter().collect();
        let intersection = set_a.intersection(&set_b).count() as f64;
        let union = set_a.union(&set_b).count() as f64;
        if union < 1e-9 { return 0.0; }
        intersection / union
    }

    /// Similarity between this pattern and a live signal activation map.
    pub fn similarity_to_live(&self, active_signals: &HashMap<SignalId, f64>) -> f64 {
        if self.signal_set.is_empty() || active_signals.is_empty() {
            return 0.0;
        }
        let my_set: std::collections::HashSet<_> = self.signal_set.iter().collect();
        let live_set: std::collections::HashSet<_> = active_signals.keys().collect();
        let intersection = my_set.intersection(&live_set).count() as f64;
        let union = my_set.union(&live_set).count() as f64;
        intersection / union
    }
}
