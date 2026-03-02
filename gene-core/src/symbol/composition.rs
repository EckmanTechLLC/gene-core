use crate::signal::types::SignalId;
use crate::symbol::ledger::SymbolLedger;
use std::collections::{HashMap, HashSet};

/// Tracks symbol co-activations and coins composite symbols when two symbols
/// co-activate together at least `threshold` times.
///
/// Composites are tokens Φ_C_NNNN grounded in the intersection of their parents'
/// signal clusters. Recursion is natural: composites appear in activation frames
/// and can themselves become parents of higher-order composites.
pub struct CompositionEngine {
    /// Co-activation counts: (lower_idx, higher_idx) → count
    co_activation_counts: HashMap<(u32, u32), u64>,
    /// Pairs that have already been composed — skipped in future observations
    composed_pairs: HashSet<(u32, u32)>,
    /// Number of co-activations required to trigger composition
    pub threshold: u64,
}

impl CompositionEngine {
    pub fn new(threshold: u64) -> Self {
        Self {
            co_activation_counts: HashMap::new(),
            composed_pairs: HashSet::new(),
            threshold,
        }
    }

    /// Rebuild `composed_pairs` from composites already stored in the ledger.
    /// Call this after loading a checkpoint so we don't re-coin existing pairs.
    pub fn seed_from_ledger(&mut self, ledger: &SymbolLedger) {
        for sym in ledger.composites() {
            if sym.parents.len() == 2 {
                let a = sym.parents[0].min(sym.parents[1]);
                let b = sym.parents[0].max(sym.parents[1]);
                self.composed_pairs.insert((a, b));
            }
        }
    }

    /// Record co-activations from the current frame.
    /// Only tracks pairs of basic (non-composite) symbols to prevent exponential growth.
    /// `active` is the Vec<(symbol_index, token, strength)> from SymbolActivationFrame.
    /// `ledger` is used to filter out composite symbols.
    pub fn observe(&mut self, active: &[(u32, String, f64)], ledger: &SymbolLedger) {
        // Only basic (non-composite) symbols can become parents
        let indices: Vec<u32> = active.iter()
            .filter_map(|(idx, _, _)| {
                ledger.get(*idx).filter(|s| !s.is_composite).map(|_| *idx)
            })
            .collect();
        for i in 0..indices.len() {
            for j in (i + 1)..indices.len() {
                let a = indices[i].min(indices[j]);
                let b = indices[i].max(indices[j]);
                if !self.composed_pairs.contains(&(a, b)) {
                    *self.co_activation_counts.entry((a, b)).or_insert(0) += 1;
                }
            }
        }
    }

    /// Maximum number of composites the engine will coin in total.
    pub const MAX_COMPOSITES: usize = 200;

    /// Remove all co_activation_counts and composed_pairs referencing pruned indices.
    pub fn purge_symbols(&mut self, pruned: &[u32]) {
        let set: std::collections::HashSet<u32> = pruned.iter().copied().collect();
        self.co_activation_counts.retain(|&(a, b), _| !set.contains(&a) && !set.contains(&b));
        self.composed_pairs.retain(|&(a, b)| !set.contains(&a) && !set.contains(&b));
    }

    /// Coin composites for any pair that has crossed the threshold.
    /// Returns the indices of newly coined composites.
    pub fn maybe_compose(&mut self, ledger: &mut SymbolLedger, tick: u64) -> Vec<u32> {
        // Safety cap: don't coin more composites if we're at the limit
        if ledger.composites().count() >= Self::MAX_COMPOSITES {
            return Vec::new();
        }

        let ready: Vec<(u32, u32)> = self.co_activation_counts
            .iter()
            .filter(|(&pair, &count)| count >= self.threshold && !self.composed_pairs.contains(&pair))
            .map(|(&pair, _)| pair)
            .collect();

        let mut newly_coined = Vec::new();

        for (a, b) in ready {
            let cluster_a = ledger.get(a).map(|s| s.signal_cluster.clone()).unwrap_or_default();
            let cluster_b = ledger.get(b).map(|s| s.signal_cluster.clone()).unwrap_or_default();

            let set_b: HashSet<SignalId> = cluster_b.iter().copied().collect();
            let intersection: Vec<SignalId> = cluster_a.iter()
                .copied()
                .filter(|id| set_b.contains(id))
                .collect();

            let composite_idx = ledger.coin_composite(vec![a, b], intersection, tick);
            self.composed_pairs.insert((a, b));
            self.co_activation_counts.remove(&(a, b));

            newly_coined.push(composite_idx);
        }

        newly_coined
    }
}
