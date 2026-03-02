use crate::pattern::record::PatternRecord;
use crate::signal::types::SignalId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// An internally-generated token representing a recurring internal state.
/// The token itself carries no meaning — meaning is in the grounding.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Symbol {
    /// Token: Φ_NNNN (basic) or Φ_C_NNNN (composite)
    pub token: String,
    /// Numeric index
    pub index: u32,
    /// The pattern record this symbol is grounded in (0 for composites)
    pub pattern_id: u64,
    /// Activation strength — how strongly this symbol is currently active
    pub activation: f64,
    /// Total times this symbol has been activated
    pub activation_count: u64,
    /// Mean imbalance context at activation
    pub mean_imbalance_context: f64,
    /// Associated directives (from directives.toml self-modification)
    pub directive_notes: Vec<String>,
    /// True if this symbol is a composition of two or more parent symbols
    pub is_composite: bool,
    /// Parent symbol indices (empty for basic symbols)
    pub parents: Vec<u32>,
    /// Signal cluster: signals this symbol is grounded in.
    /// For basic symbols: from the pattern record's signal_set.
    /// For composites: intersection of parent signal clusters.
    pub signal_cluster: Vec<SignalId>,
    /// Tick at which this symbol was coined
    pub coined_at_tick: u64,
    /// Tick of most recent activation (0 if never activated)
    pub last_activated_tick: u64,
}

impl Symbol {
    pub fn new(index: u32, pattern_id: u64, tick: u64) -> Self {
        Self {
            token: format!("Φ_{:04}", index),
            index,
            pattern_id,
            activation: 0.0,
            activation_count: 0,
            mean_imbalance_context: 0.0,
            directive_notes: Vec::new(),
            is_composite: false,
            parents: Vec::new(),
            signal_cluster: Vec::new(),
            coined_at_tick: tick,
            last_activated_tick: 0,
        }
    }

    pub fn new_composite(index: u32, parents: Vec<u32>, signal_cluster: Vec<SignalId>, tick: u64) -> Self {
        Self {
            token: format!("Φ_C_{:04}", index),
            index,
            pattern_id: 0,
            activation: 0.0,
            activation_count: 0,
            mean_imbalance_context: 0.0,
            directive_notes: Vec::new(),
            is_composite: true,
            parents,
            signal_cluster,
            coined_at_tick: tick,
            last_activated_tick: 0,
        }
    }

    pub fn activate(&mut self, strength: f64, imbalance: f64, tick: u64) {
        self.activation = strength;
        self.activation_count += 1;
        self.mean_imbalance_context +=
            (imbalance - self.mean_imbalance_context) / self.activation_count as f64;
        self.last_activated_tick = tick;
    }

    pub fn decay_activation(&mut self, rate: f64) {
        self.activation *= 1.0 - rate;
    }
}

/// Maps pattern IDs to coined symbols.
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct SymbolLedger {
    symbols: HashMap<u32, Symbol>,
    pattern_to_symbol: HashMap<u64, u32>,
    next_index: u32,
}

impl SymbolLedger {
    pub fn new() -> Self { Self::default() }

    /// Coin a new symbol for a pattern, or return the existing one.
    pub fn coin(&mut self, pattern_id: u64, tick: u64) -> u32 {
        self.coin_with_cluster(pattern_id, Vec::new(), tick)
    }

    /// Coin a new basic symbol with a known signal cluster, or return the existing one.
    pub fn coin_with_cluster(&mut self, pattern_id: u64, cluster: Vec<SignalId>, tick: u64) -> u32 {
        if let Some(&idx) = self.pattern_to_symbol.get(&pattern_id) {
            return idx;
        }
        let idx = self.next_index;
        self.next_index += 1;
        let mut sym = Symbol::new(idx, pattern_id, tick);
        sym.signal_cluster = cluster;
        tracing::info!("symbol coined: {} ← pattern {:016x}", sym.token, pattern_id);
        self.symbols.insert(idx, sym);
        self.pattern_to_symbol.insert(pattern_id, idx);
        idx
    }

    /// Coin a composite symbol from parent symbols and their cluster intersection.
    pub fn coin_composite(&mut self, parents: Vec<u32>, cluster: Vec<SignalId>, tick: u64) -> u32 {
        let idx = self.next_index;
        self.next_index += 1;
        let sym = Symbol::new_composite(idx, parents, cluster, tick);
        tracing::info!(
            "composite coined: {} ← parents {:?}",
            sym.token,
            sym.parents.iter().map(|p| format!("Φ_{:04}", p)).collect::<Vec<_>>()
        );
        self.symbols.insert(idx, sym);
        idx
    }

    /// Prune composites unactivated (or uncoined) for longer than age_threshold ticks.
    /// Returns the indices of pruned composites for CompositionEngine cleanup.
    pub fn prune_composites(&mut self, current_tick: u64, age_threshold: u64) -> Vec<u32> {
        let stale: Vec<u32> = self.symbols.values()
            .filter(|s| s.is_composite)
            .filter(|s| {
                let last = s.last_activated_tick.max(s.coined_at_tick);
                current_tick.saturating_sub(last) > age_threshold
            })
            .map(|s| s.index)
            .collect();
        for idx in &stale {
            self.symbols.remove(idx);
            // composites have pattern_id = 0, no pattern_to_symbol entry to clean
        }
        stale
    }

    pub fn get(&self, index: u32) -> Option<&Symbol> {
        self.symbols.get(&index)
    }

    pub fn get_mut(&mut self, index: u32) -> Option<&mut Symbol> {
        self.symbols.get_mut(&index)
    }

    pub fn by_pattern(&self, pattern_id: u64) -> Option<u32> {
        self.pattern_to_symbol.get(&pattern_id).copied()
    }

    pub fn all(&self) -> impl Iterator<Item = &Symbol> {
        self.symbols.values()
    }

    /// Iterator over composite symbols only.
    pub fn composites(&self) -> impl Iterator<Item = &Symbol> {
        self.symbols.values().filter(|s| s.is_composite)
    }

    /// All currently active symbols (activation > threshold).
    pub fn active(&self, threshold: f64) -> Vec<&Symbol> {
        self.symbols.values()
            .filter(|s| s.activation > threshold)
            .collect()
    }

    pub fn len(&self) -> usize {
        self.symbols.len()
    }

    /// Add a directive note to a symbol (from self-modification).
    pub fn annotate(&mut self, index: u32, note: String) {
        if let Some(sym) = self.symbols.get_mut(&index) {
            if !sym.directive_notes.contains(&note) {
                sym.directive_notes.push(note);
            }
        }
    }
}
