use crate::symbol::ledger::SymbolLedger;
use serde::{Deserialize, Serialize};

/// A snapshot of symbol activation state at a given tick.
/// This is what gets passed upward to Layer 4 (SelfModel).
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SymbolActivationFrame {
    pub tick: u64,
    /// (symbol_index, token_string, activation_strength)
    pub active: Vec<(u32, String, f64)>,
    /// Dominant symbol (highest activation)
    pub dominant: Option<u32>,
}

impl SymbolActivationFrame {
    pub fn build(tick: u64, ledger: &SymbolLedger, threshold: f64) -> Self {
        let mut active: Vec<(u32, String, f64)> = ledger.active(threshold)
            .iter()
            .map(|s| (s.index, s.token.clone(), s.activation))
            .collect();
        active.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap());

        let dominant = active.first().map(|(idx, _, _)| *idx);

        Self { tick, active, dominant }
    }

    pub fn is_empty(&self) -> bool {
        self.active.is_empty()
    }

    /// Returns a compact string representation for logging/tracing.
    pub fn summary(&self) -> String {
        if self.active.is_empty() {
            return "[]".to_string();
        }
        let tokens: Vec<String> = self.active.iter()
            .take(5)
            .map(|(_, tok, str)| format!("{}:{:.2}", tok, str))
            .collect();
        format!("[{}]", tokens.join(" "))
    }
}
