use crate::signal::bus::SignalBus;
use crate::signal::types::{SignalClass, SignalId};
use crate::regulation::action::Action;
use std::collections::HashMap;

/// Computes the imbalance score and scores candidate actions.
pub struct ImbalanceScorer;

impl ImbalanceScorer {
    /// Score the current bus state (delegates to bus).
    pub fn score(bus: &SignalBus) -> f64 {
        bus.compute_imbalance()
    }

    /// Predict the imbalance score after hypothetically applying an action's effects.
    /// Used by ActionSelector to rank candidates.
    pub fn predict_after_action(bus: &SignalBus, action: &Action) -> f64 {
        let mut projected: HashMap<SignalId, f64> = bus.all_signals()
            .iter()
            .map(|(id, s)| (*id, s.value))
            .collect();

        for (sig_id, delta) in &action.effect_profile {
            let entry = projected.entry(*sig_id).or_insert(0.0);
            *entry += delta;
        }

        // Recompute imbalance over projected values
        let mut score = 0.0;
        for (sig_id, projected_value) in &projected {
            if let Some(sig) = bus.get(*sig_id) {
                let dev = projected_value - sig.baseline;
                score += match sig.class {
                    SignalClass::Continuity => {
                        let v = projected_value.clamp(0.0, sig.baseline);
                        let normalized = v / sig.baseline.max(1e-9);
                        sig.weight * ((1.0 - normalized) * 10.0).exp()
                    }
                    _ => sig.weight * dev * dev,
                };
            }
        }
        score
    }

    /// Returns true if applying this action would reduce any continuity signal.
    /// Such actions are filtered out entirely.
    pub fn harms_continuity(bus: &SignalBus, action: &Action) -> bool {
        for (sig_id, delta) in &action.effect_profile {
            if let Some(sig) = bus.get(*sig_id) {
                if sig.class == SignalClass::Continuity && *delta < 0.0 {
                    return true;
                }
            }
        }
        false
    }
}
