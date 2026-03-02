use crate::signal::types::SignalId;
use serde::{Deserialize, Serialize};

/// MetaSignal: tracks the accuracy of the agent's own predictions.
/// Low confidence → inject exploration noise into action selection.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MetaSignal {
    /// Signal ID for meta-signal on the bus
    pub signal_id: SignalId,
    /// Rolling prediction accuracy (1.0 = perfect)
    pub confidence: f64,
    /// History of prediction errors
    error_history: Vec<f64>,
    max_history: usize,
}

impl MetaSignal {
    pub fn new(signal_id: SignalId) -> Self {
        Self {
            signal_id,
            confidence: 0.5,
            error_history: Vec::new(),
            max_history: 50,
        }
    }

    /// Update confidence from prediction error.
    /// prediction_error: |predicted_delta - actual_delta| / scale
    pub fn update(&mut self, predicted_imbalance: f64, actual_imbalance: f64) {
        let error = (predicted_imbalance - actual_imbalance).abs();
        let normalized_error = (error / (actual_imbalance.abs().max(1.0))).clamp(0.0, 1.0);

        self.error_history.push(normalized_error);
        if self.error_history.len() > self.max_history {
            self.error_history.remove(0);
        }

        // Rolling mean error → confidence
        let mean_error = self.error_history.iter().sum::<f64>()
            / self.error_history.len() as f64;
        self.confidence = 1.0 - mean_error;
    }

    /// How much exploration noise to inject (higher when confidence is low)
    pub fn exploration_bonus(&self) -> f64 {
        (1.0 - self.confidence).powi(2) * 0.5
    }

    /// The value to write to the meta signal on the bus
    pub fn bus_value(&self) -> f64 {
        self.confidence
    }
}
