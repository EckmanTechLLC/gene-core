use crate::signal::types::SignalId;
use crate::symbol::activation::SymbolActivationFrame;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// A coarse-grained self-history entry — stored at lower resolution than the signal ledger.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfHistoryEntry {
    pub tick: u64,
    pub dominant_symbol: Option<u32>,
    pub active_symbols: Vec<u32>,
    pub imbalance: f64,
    pub action_taken: Option<u32>,
}

/// The agent's model of itself.
///
/// Tracks:
///   - Which symbols have been most active historically (symbolic signature)
///   - Which action sequences have correlated with regulation improvement
///   - A "characteristic signature" — the agent's structural identity
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct SelfModel {
    /// Coarse history (one entry per N ticks)
    history: Vec<SelfHistoryEntry>,
    /// Per-symbol total activation weight across all time
    pub symbol_weights: HashMap<u32, f64>,
    /// Per-action historical mean imbalance improvement
    pub action_preferences: HashMap<u32, f64>,
    /// The current structural identity: top symbols by weight
    pub identity_signature: Vec<(u32, f64)>,
    /// How many ticks of history to retain
    max_history: usize,
    /// Coarsening factor: record one entry every N ticks
    pub coarsen_factor: u64,
    /// Tick of last self-model update
    last_update: u64,
}

impl SelfModel {
    pub fn new(max_history: usize, coarsen_factor: u64) -> Self {
        Self {
            max_history,
            coarsen_factor,
            ..Default::default()
        }
    }

    /// Update the self-model with the current frame.
    pub fn update(
        &mut self,
        tick: u64,
        frame: &SymbolActivationFrame,
        imbalance: f64,
        action_taken: Option<u32>,
        imbalance_delta: f64,
    ) {
        // Only record at coarsened rate
        if tick % self.coarsen_factor != 0 {
            return;
        }

        let entry = SelfHistoryEntry {
            tick,
            dominant_symbol: frame.dominant,
            active_symbols: frame.active.iter().map(|(i, _, _)| *i).collect(),
            imbalance,
            action_taken,
        };

        // Update symbol weights (exponential moving average)
        for (sym_idx, _, strength) in &frame.active {
            let w = self.symbol_weights.entry(*sym_idx).or_insert(0.0);
            *w = 0.95 * *w + 0.05 * strength;
        }

        // Update action preferences from observed delta
        if let Some(aid) = action_taken {
            let pref = self.action_preferences.entry(aid).or_insert(0.0);
            *pref = 0.9 * *pref + 0.1 * (-imbalance_delta); // negative delta = improvement
        }

        self.history.push(entry);
        if self.history.len() > self.max_history {
            self.history.drain(0..self.max_history / 4);
        }

        // Recompute identity signature
        self.recompute_identity();
        self.last_update = tick;
    }

    fn recompute_identity(&mut self) {
        let mut sig: Vec<(u32, f64)> = self.symbol_weights
            .iter()
            .map(|(k, v)| (*k, *v))
            .collect();
        sig.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        sig.truncate(10);
        self.identity_signature = sig;
    }

    /// Returns the agent's preferred action given the current dominant symbol context.
    /// Preference is weighted by both self-model action preferences and symbol co-occurrence.
    pub fn preferred_action(
        &self,
        current_symbols: &[u32],
        candidate_actions: &[u32],
    ) -> Option<u32> {
        if candidate_actions.is_empty() {
            return None;
        }

        // Find history entries with similar symbol context
        let similar_entries: Vec<&SelfHistoryEntry> = self.history.iter()
            .filter(|e| {
                let overlap = e.active_symbols.iter()
                    .filter(|s| current_symbols.contains(s))
                    .count();
                overlap > 0
            })
            .collect();

        if similar_entries.is_empty() {
            // Fall back to action preferences
            return candidate_actions.iter()
                .max_by(|a, b| {
                    let pa = self.action_preferences.get(a).copied().unwrap_or(0.0);
                    let pb = self.action_preferences.get(b).copied().unwrap_or(0.0);
                    pa.partial_cmp(&pb).unwrap()
                })
                .copied();
        }

        // Score candidate actions by co-occurrence in similar contexts
        let mut action_scores: HashMap<u32, f64> = HashMap::new();
        for entry in &similar_entries {
            if let Some(aid) = entry.action_taken {
                if candidate_actions.contains(&aid) {
                    let pref = self.action_preferences.get(&aid).copied().unwrap_or(0.0);
                    *action_scores.entry(aid).or_insert(0.0) += 1.0 + pref;
                }
            }
        }

        action_scores.into_iter()
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .map(|(aid, _)| aid)
    }

    pub fn history(&self) -> &[SelfHistoryEntry] {
        &self.history
    }

    pub fn identity_description(&self) -> String {
        if self.identity_signature.is_empty() {
            return "identity: unformed".to_string();
        }
        let parts: Vec<String> = self.identity_signature.iter()
            .take(5)
            .map(|(idx, w)| format!("Φ_{:04}:{:.3}", idx, w))
            .collect();
        format!("identity: [{}]", parts.join(" "))
    }
}
