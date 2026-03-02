use crate::pattern::index::PatternIndex;
use crate::pattern::record::PatternRecord;
use crate::signal::types::SignalId;
use crate::symbol::ledger::SymbolLedger;
use std::collections::HashMap;

const SALIENCE_IMBALANCE_WEIGHT: f64 = 0.4;
const SALIENCE_FREQUENCY_WEIGHT: f64 = 0.6;

/// Manages the grounding of symbols to pattern records.
/// When patterns become salient, symbols are coined.
/// When the current signal state resembles a grounded pattern, that symbol activates.
pub struct SymbolGrounder {
    /// Activation decay rate per tick
    pub activation_decay: f64,
}

impl SymbolGrounder {
    pub fn new(activation_decay: f64) -> Self {
        Self { activation_decay }
    }

    /// Process salient patterns from the index, coin symbols as needed.
    pub fn process_salience(
        &self,
        index: &PatternIndex,
        ledger: &mut SymbolLedger,
        tick: u64,
    ) {
        for record in index.salient() {
            ledger.coin_with_cluster(record.id, record.signal_set.clone(), tick);
        }
    }

    /// Update symbol activations based on current signal state similarity.
    /// Returns a list of (symbol_index, activation_strength) for active symbols.
    pub fn update_activations(
        &self,
        index: &PatternIndex,
        ledger: &mut SymbolLedger,
        active_signals: &HashMap<SignalId, f64>,
        current_imbalance: f64,
        tick: u64,
    ) -> Vec<(u32, f64)> {
        // Decay all activations first
        let indices: Vec<u32> = ledger.all().map(|s| s.index).collect();
        for idx in &indices {
            if let Some(sym) = ledger.get_mut(*idx) {
                sym.decay_activation(self.activation_decay);
            }
        }

        // Find patterns similar to current state
        let similar = index.find_similar(active_signals, 0.4);

        let mut active = Vec::new();
        for (pattern_id, similarity) in similar {
            if let Some(sym_idx) = ledger.by_pattern(pattern_id) {
                // Salience-weighted activation strength
                let freq = index.get(pattern_id)
                    .map(|r| r.frequency as f64)
                    .unwrap_or(1.0);
                let strength = similarity * (1.0 + freq.ln() * 0.1).min(1.0);

                if let Some(sym) = ledger.get_mut(sym_idx) {
                    sym.activate(strength, current_imbalance, tick);
                }
                active.push((sym_idx, strength));
            }
        }
        // Activate composites: fires when ALL parents are active
        let composite_indices: Vec<u32> = ledger.composites().map(|s| s.index).collect();
        for idx in composite_indices {
            let (parent_strengths, all_active) = {
                let sym = match ledger.get(idx) { Some(s) => s, None => continue };
                if sym.parents.is_empty() { continue; }
                let strengths: Vec<f64> = sym.parents.iter()
                    .filter_map(|&pid| ledger.get(pid))
                    .filter(|p| p.activation > 0.0)
                    .map(|p| p.activation)
                    .collect();
                let all = strengths.len() == sym.parents.len();
                (strengths, all)
            };
            if all_active {
                let strength = parent_strengths.iter().cloned().fold(f64::INFINITY, f64::min);
                if let Some(sym) = ledger.get_mut(idx) {
                    sym.activate(strength, current_imbalance, tick);
                }
                active.push((idx, strength));
            }
        }

        active
    }

    /// Compute a salience score for a pattern record.
    /// Higher = more likely to be symbolified and retained.
    pub fn salience(record: &PatternRecord, max_frequency: u32) -> f64 {
        let freq_score = record.frequency as f64 / max_frequency.max(1) as f64;
        let imbalance_score = record.mean_imbalance / 10.0; // normalize
        SALIENCE_FREQUENCY_WEIGHT * freq_score + SALIENCE_IMBALANCE_WEIGHT * imbalance_score
    }
}
