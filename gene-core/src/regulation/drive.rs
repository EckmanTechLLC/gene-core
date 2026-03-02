use crate::signal::types::SignalId;

/// Minimum ticks between circuit breaker fires.
/// Prevents the selector from being reset faster than causal learning can accumulate.
const CIRCUIT_BREAK_COOLDOWN: u64 = 1000;

/// The RegulationDrive translates imbalance into urgency.
/// Urgency affects action selection strategy: high urgency = greedy,
/// low urgency = more exploratory.
pub struct RegulationDrive {
    /// The signal ID on the bus that reflects the drive level
    pub drive_signal_id: SignalId,
    /// If imbalance has not decreased in this many consecutive ticks → circuit breaker fires
    pub stagnation_limit: u32,
    stagnation_count: u32,
    last_imbalance: f64,
    /// Last tick the circuit breaker fired — enforces CIRCUIT_BREAK_COOLDOWN
    last_circuit_break_tick: u64,
}

impl RegulationDrive {
    pub fn new(drive_signal_id: SignalId, stagnation_limit: u32) -> Self {
        Self {
            drive_signal_id,
            stagnation_limit,
            stagnation_count: 0,
            last_imbalance: f64::MAX,
            last_circuit_break_tick: 0,
        }
    }

    /// Update drive state. Returns true if the circuit breaker should fire.
    /// Fires at most once per CIRCUIT_BREAK_COOLDOWN ticks.
    pub fn update(&mut self, current_imbalance: f64, tick: u64) -> bool {
        if current_imbalance >= self.last_imbalance - 1e-6 {
            self.stagnation_count += 1;
        } else {
            self.stagnation_count = 0;
        }
        self.last_imbalance = current_imbalance;

        if self.stagnation_count >= self.stagnation_limit
            && tick.saturating_sub(self.last_circuit_break_tick) >= CIRCUIT_BREAK_COOLDOWN
        {
            self.last_circuit_break_tick = tick;
            return true;
        }
        false
    }

    pub fn reset_stagnation(&mut self) {
        self.stagnation_count = 0;
    }

    /// Urgency in [0, 1]: how urgently should the system act?
    pub fn urgency(&self, imbalance: f64, max_expected: f64) -> f64 {
        (imbalance / max_expected.max(1e-9)).clamp(0.0, 1.0)
    }
}
