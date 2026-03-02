use crate::signal::types::{DeltaSource, Signal, SignalClass, SignalDelta, SignalId, SignalSnapshot};
use anyhow::Result;
use chrono::Utc;
use crossbeam_channel::Sender;
use std::collections::HashMap;

/// Central registry for all live signals. Each tick:
///   1. Applies decay toward each signal's baseline
///   2. Applies any queued deltas
///   3. Records a snapshot
///   4. Broadcasts deltas to subscribers
pub struct SignalBus {
    signals: HashMap<SignalId, Signal>,
    next_id: u32,
    /// Queued deltas to apply this tick
    pending: Vec<(SignalId, f64, DeltaSource)>,
    /// Optional channel to broadcast snapshots to pattern extractor etc.
    snapshot_tx: Option<Sender<SignalSnapshot>>,
    /// Optional channel to broadcast deltas
    delta_tx: Option<Sender<SignalDelta>>,
}

impl SignalBus {
    pub fn new() -> Self {
        Self {
            signals: HashMap::new(),
            next_id: 0,
            pending: Vec::new(),
            snapshot_tx: None,
            delta_tx: None,
        }
    }

    pub fn set_snapshot_tx(&mut self, tx: Sender<SignalSnapshot>) {
        self.snapshot_tx = Some(tx);
    }

    pub fn set_delta_tx(&mut self, tx: Sender<SignalDelta>) {
        self.delta_tx = Some(tx);
    }

    /// Register a new signal, returns its ID.
    pub fn register(&mut self, class: SignalClass, baseline: f64, decay_rate: f64, weight: f64) -> SignalId {
        let id = SignalId(self.next_id);
        self.next_id += 1;
        self.signals.insert(id, Signal::new(id, class, baseline, decay_rate, weight));
        id
    }

    /// Register a signal with a specific ID (used for well-known continuity signals).
    pub fn register_with_id(&mut self, id: SignalId, class: SignalClass, baseline: f64, decay_rate: f64, weight: f64) {
        if id.0 >= self.next_id {
            self.next_id = id.0 + 1;
        }
        self.signals.insert(id, Signal::new(id, class, baseline, decay_rate, weight));
    }

    /// Queue a delta to be applied on the next tick.
    pub fn queue_delta(&mut self, id: SignalId, delta: f64, source: DeltaSource) {
        self.pending.push((id, delta, source));
    }

    /// Apply external stimulus immediately (bypasses queue — use sparingly).
    pub fn inject(&mut self, id: SignalId, delta: f64) {
        if let Some(sig) = self.signals.get_mut(&id) {
            sig.apply_delta(delta);
        }
    }

    pub fn get(&self, id: SignalId) -> Option<&Signal> {
        self.signals.get(&id)
    }

    pub fn get_value(&self, id: SignalId) -> f64 {
        self.signals.get(&id).map(|s| s.value).unwrap_or(0.0)
    }

    pub fn all_signals(&self) -> &HashMap<SignalId, Signal> {
        &self.signals
    }

    /// Returns a sorted vec of (id, value) for snapshot and display.
    pub fn snapshot_values(&self) -> Vec<(SignalId, f64)> {
        let mut v: Vec<(SignalId, f64)> = self.signals.iter()
            .map(|(id, s)| (*id, s.value))
            .collect();
        v.sort_by_key(|(id, _)| *id);
        v
    }

    /// Execute one tick: decay → apply pending → compute imbalance → broadcast.
    /// Returns the imbalance score and the produced snapshot.
    pub fn tick(&mut self, tick: u64) -> (f64, SignalSnapshot) {
        // 1. Decay
        for sig in self.signals.values_mut() {
            sig.decay();
        }

        // 2. Apply pending deltas
        let pending = std::mem::take(&mut self.pending);
        for (id, delta, source) in pending {
            if let Some(sig) = self.signals.get_mut(&id) {
                sig.apply_delta(delta);
            }
            if let Some(tx) = &self.delta_tx {
                let _ = tx.try_send(SignalDelta { tick, id, delta, source });
            }
        }

        // 3. Compute imbalance
        let imbalance = self.compute_imbalance();

        // 4. Build snapshot
        let snapshot = SignalSnapshot {
            tick,
            timestamp_ms: Utc::now().timestamp_millis(),
            values: self.snapshot_values(),
            imbalance,
        };

        // 5. Broadcast
        if let Some(tx) = &self.snapshot_tx {
            let _ = tx.try_send(snapshot.clone());
        }

        (imbalance, snapshot)
    }

    /// Weighted imbalance score.
    /// Continuity signals use an exponential penalty.
    /// All others use weighted squared deviation.
    pub fn compute_imbalance(&self) -> f64 {
        let mut score = 0.0;
        for sig in self.signals.values() {
            let dev = sig.value - sig.baseline;
            score += match sig.class {
                SignalClass::Continuity => {
                    // Exponential penalty: near 0 = catastrophic
                    let v = sig.value.clamp(0.0, sig.baseline);
                    let normalized = v / sig.baseline.max(1e-9);
                    sig.weight * ((1.0 - normalized) * 10.0).exp()
                }
                _ => sig.weight * dev * dev,
            };
        }
        score
    }

    /// Set a signal's value directly (used by persistence on reload).
    pub fn set_value(&mut self, id: SignalId, value: f64) {
        if let Some(sig) = self.signals.get_mut(&id) {
            sig.value = value;
        }
    }

    /// Set a signal's weight in the imbalance function.
    pub fn set_weight(&mut self, id: SignalId, weight: f64) {
        if let Some(sig) = self.signals.get_mut(&id) {
            sig.weight = weight;
        }
    }
}

impl Default for SignalBus {
    fn default() -> Self {
        Self::new()
    }
}
