use crate::signal::types::{SignalId, SignalSnapshot};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A single observation: action A was taken at pre-state, post-state was observed.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CausalObservation {
    pub action_id: u32,
    pub tick: u64,
    /// Imbalance before the action
    pub pre_imbalance: f64,
    /// Imbalance after the action
    pub post_imbalance: f64,
    /// Net delta in imbalance (negative = improvement)
    pub imbalance_delta: f64,
    /// Per-signal deltas observed
    pub signal_deltas: HashMap<SignalId, f64>,
}

/// Aggregate statistics for a (action_id, signal_id) causal pair.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CausalStats {
    pub count: u32,
    pub mean_delta: f64,
    pub variance: f64,
    /// Running M2 for Welford variance
    m2: f64,
}

impl CausalStats {
    pub fn update(&mut self, new_delta: f64) {
        self.count += 1;
        let old_mean = self.mean_delta;
        self.mean_delta += (new_delta - self.mean_delta) / self.count as f64;
        self.m2 += (new_delta - old_mean) * (new_delta - self.mean_delta);
        self.variance = if self.count > 1 { self.m2 / (self.count - 1) as f64 } else { 0.0 };
    }
}

/// Tracks the causal relationship between actions and signal outcomes.
/// key: (action_id, signal_id)
#[derive(Clone, Serialize, Deserialize)]
pub struct CausalTracer {
    pub stats: HashMap<(u32, SignalId), CausalStats>,
    /// Mean imbalance improvement per action_id
    pub action_imbalance_stats: HashMap<u32, CausalStats>,
    pub observations: Vec<CausalObservation>,
    /// Max observations to retain in memory
    pub max_history: usize,
}

impl CausalTracer {
    pub fn new(max_history: usize) -> Self {
        Self {
            stats: HashMap::new(),
            action_imbalance_stats: HashMap::new(),
            observations: Vec::new(),
            max_history,
        }
    }

    pub fn record(
        &mut self,
        action_id: u32,
        tick: u64,
        pre_snap: &SignalSnapshot,
        post_snap: &SignalSnapshot,
    ) {
        let imbalance_delta = post_snap.imbalance - pre_snap.imbalance;

        let mut signal_deltas = HashMap::new();
        let pre_map: HashMap<SignalId, f64> = pre_snap.values.iter().cloned().collect();
        for (id, post_val) in &post_snap.values {
            let pre_val = pre_map.get(id).copied().unwrap_or(0.0);
            let delta = post_val - pre_val;
            if delta.abs() > 1e-9 {
                signal_deltas.insert(*id, delta);
                self.stats
                    .entry((action_id, *id))
                    .or_default()
                    .update(delta);
            }
        }

        self.action_imbalance_stats
            .entry(action_id)
            .or_default()
            .update(imbalance_delta);

        let obs = CausalObservation {
            action_id,
            tick,
            pre_imbalance: pre_snap.imbalance,
            post_imbalance: post_snap.imbalance,
            imbalance_delta,
            signal_deltas,
        };

        self.observations.push(obs);
        if self.observations.len() > self.max_history {
            self.observations.drain(0..self.max_history / 4);
        }
    }

    /// Expected imbalance delta for a given action (negative = good).
    pub fn expected_improvement(&self, action_id: u32) -> Option<f64> {
        self.action_imbalance_stats
            .get(&action_id)
            .filter(|s| s.count >= 3)
            .map(|s| s.mean_delta)
    }

    pub fn observations(&self) -> &[CausalObservation] {
        &self.observations
    }
}
