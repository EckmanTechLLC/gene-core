use crate::regulation::action::{Action, ActionSpace};
use crate::regulation::causal::CausalTracer;
use crate::regulation::scorer::ImbalanceScorer;
use crate::signal::bus::SignalBus;
use rand::Rng;
use std::collections::HashMap;

/// Minimum ticks between executions of a system action.
/// Prevents runaway build/write loops.
const SYSTEM_ACTION_COOLDOWN: u64 = 2000;

/// Minimum ticks between ANY system action (global rate limit).
const SYSTEM_ACTION_GLOBAL_COOLDOWN: u64 = 500;

pub struct ActionSelector {
    pub causal_weight: f64,
    pub exploration_rate: f64,
    last_action: Option<u32>,
    same_action_count: u32,
    repetition_limit: u32,
    /// Per-action: last tick it was executed
    last_executed: HashMap<u32, u64>,
    /// Last tick any system action ran
    last_system_action_tick: u64,
}

impl ActionSelector {
    pub fn new(exploration_rate: f64, repetition_limit: u32) -> Self {
        Self {
            causal_weight: 0.5,
            exploration_rate,
            last_action: None,
            same_action_count: 0,
            repetition_limit,
            last_executed: HashMap::new(),
            last_system_action_tick: 0,
        }
    }

    pub fn select(
        &mut self,
        bus: &SignalBus,
        space: &ActionSpace,
        causal: &CausalTracer,
        urgency: f64,
        current_tick: u64,
    ) -> Option<u32> {
        let mut rng = rand::thread_rng();
        let force_explore = self.same_action_count >= self.repetition_limit;

        let eff_exploration = if force_explore {
            1.0
        } else {
            self.exploration_rate * (1.0 - urgency * 0.8)
        };

        // Build candidate list with cooldown filtering applied
        let candidates: Vec<&Action> = space.all().iter()
            .filter(|a| {
                if ImbalanceScorer::harms_continuity(bus, a) {
                    return false;
                }
                // Per-action cooldown
                if a.is_system_action() {
                    let last = self.last_executed.get(&a.id).copied().unwrap_or(0);
                    if current_tick.saturating_sub(last) < SYSTEM_ACTION_COOLDOWN {
                        return false;
                    }
                    // Global system action rate limit
                    if current_tick.saturating_sub(self.last_system_action_tick) < SYSTEM_ACTION_GLOBAL_COOLDOWN {
                        return false;
                    }
                }
                true
            })
            .collect();

        if candidates.is_empty() {
            return None;
        }

        if rng.gen::<f64>() < eff_exploration {
            let idx = rng.gen_range(0..candidates.len());
            let chosen = candidates[idx].id;
            self.record_execution(chosen, current_tick, space);
            return Some(chosen);
        }

        let current_imbalance = ImbalanceScorer::score(bus);
        let mut best_id: Option<u32> = None;
        let mut best_score = f64::MAX;

        for action in &candidates {
            let predicted = ImbalanceScorer::predict_after_action(bus, action);
            let predicted_improvement = predicted - current_imbalance;

            let learned = causal.expected_improvement(action.id)
                .unwrap_or(predicted_improvement);

            let has_data = causal.action_imbalance_stats
                .get(&action.id)
                .map(|s| s.count)
                .unwrap_or(0);

            let blend = if has_data >= 10 {
                self.causal_weight
            } else {
                (has_data as f64 / 10.0) * self.causal_weight
            };

            // System actions get a significant score penalty so they're only
            // selected when they're clearly better than all somatic options.
            // This prevents them winning by default on an empty effect profile.
            let system_penalty = if action.is_system_action() { 2.0 } else { 0.0 };

            let score = (1.0 - blend) * predicted_improvement + blend * learned + system_penalty;

            if score < best_score {
                best_score = score;
                best_id = Some(action.id);
            }
        }

        if let Some(id) = best_id {
            self.record_execution(id, current_tick, space);
        }
        best_id
    }

    fn record_execution(&mut self, chosen: u32, tick: u64, space: &ActionSpace) {
        if self.last_action == Some(chosen) {
            self.same_action_count += 1;
        } else {
            self.same_action_count = 0;
            self.last_action = Some(chosen);
        }
        self.last_executed.insert(chosen, tick);
        if space.get(chosen).map(|a| a.is_system_action()).unwrap_or(false) {
            self.last_system_action_tick = tick;
        }
    }
}
