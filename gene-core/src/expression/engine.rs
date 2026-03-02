use crate::regulation::causal::CausalTracer;
use crate::selfmodel::meta::MetaSignal;
use crate::selfmodel::model::SelfModel;
use crate::signal::bus::SignalBus;
use crate::symbol::activation::SymbolActivationFrame;
use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;

const MAX_ROLLING: usize = 10_000;
const SIGNAL_DRIVER_THRESHOLD: f64 = 0.1;
const TREND_WINDOW: usize = 20;

/// Well-known signal names by ID.
fn signal_name(id: u32) -> String {
    match id {
        0 => "s_continuity".to_string(),
        1 => "s_integrity".to_string(),
        2 => "s_coherence".to_string(),
        3 => "s_memory".to_string(),
        4 => "s_disk".to_string(),
        5 => "s_meta".to_string(),
        6 => "s_drive".to_string(),
        17 => "s_cpu_load".to_string(),
        18 => "s_net_rx".to_string(),
        19 => "s_net_tx".to_string(),
        20 => "s_disk_io".to_string(),
        21 => "s_uptime_cycle".to_string(),
        22 => "s_proc_count".to_string(),
        n => format!("s_{:04}", n),
    }
}

/// Emits structured JSONL expression records every N ticks.
///
/// Each record captures the dominant symbol, active cluster, current imbalance,
/// trend direction, last action taken, significant signal deviations, and
/// self-model confidence. Written to `gene-data/expression.log` (rolling 10K lines).
pub struct ExpressionEngine {
    pub emit_interval: u64,
    log_path: PathBuf,
    rolling: VecDeque<serde_json::Value>,
    recent_imbalance: VecDeque<f64>,
}

impl ExpressionEngine {
    pub fn new(data_dir: &std::path::Path, emit_interval: u64) -> Self {
        Self {
            emit_interval,
            log_path: data_dir.join("expression.log"),
            rolling: VecDeque::new(),
            recent_imbalance: VecDeque::new(),
        }
    }

    /// Call every tick. Returns the emitted record when one is produced.
    pub fn maybe_emit(
        &mut self,
        tick: u64,
        frame: &SymbolActivationFrame,
        bus: &SignalBus,
        self_model: &SelfModel,
        meta: &MetaSignal,
        last_action_id: Option<u32>,
        _causal: &CausalTracer,
    ) -> Option<serde_json::Value> {
        if tick % self.emit_interval != 0 {
            return None;
        }

        let imbalance = bus.compute_imbalance();

        // Update imbalance trend window
        self.recent_imbalance.push_back(imbalance);
        if self.recent_imbalance.len() > TREND_WINDOW {
            self.recent_imbalance.pop_front();
        }

        let dominant: serde_json::Value = if !frame.active.is_empty() {
            serde_json::Value::String(frame.active[0].1.clone())
        } else {
            serde_json::Value::Null
        };

        let cluster: Vec<String> = frame.active.iter().map(|(_, tok, _)| tok.clone()).collect();

        let trend = self.compute_trend();

        let action_context = last_action_id
            .map(|id| format!("action_{}", id))
            .unwrap_or_else(|| "none".to_string());

        let signal_drivers = self.compute_signal_drivers(bus);

        let identity_alignment = self.compute_identity_alignment(frame, self_model);

        let record = serde_json::json!({
            "tick": tick,
            "dominant": dominant,
            "cluster": cluster,
            "imbalance": (imbalance * 1000.0).round() / 1000.0,
            "imbalance_trend": trend,
            "action_context": action_context,
            "signal_drivers": signal_drivers,
            "self_model_confidence": (meta.confidence * 10000.0).round() / 10000.0,
            "identity_alignment": (identity_alignment * 100.0).round() / 100.0,
        });

        self.append(record.clone());
        Some(record)
    }

    /// Returns the last `n` expression records (most recent last).
    pub fn recent(&self, n: usize) -> Vec<serde_json::Value> {
        self.rolling.iter().rev().take(n).cloned().collect::<Vec<_>>()
            .into_iter().rev().collect()
    }

    // ── Private helpers ──────────────────────────────────────────────────────

    fn compute_trend(&self) -> &'static str {
        let len = self.recent_imbalance.len();
        if len < TREND_WINDOW {
            return "stable";
        }
        let half = len / 2;
        let older: f64 = self.recent_imbalance.iter().take(half).sum::<f64>() / half as f64;
        let newer: f64 = self.recent_imbalance.iter().skip(half).sum::<f64>() / half as f64;
        let diff = newer - older;
        if diff > 1.0 {
            "rising"
        } else if diff < -1.0 {
            "falling"
        } else {
            "stable"
        }
    }

    fn compute_signal_drivers(&self, bus: &SignalBus) -> serde_json::Value {
        let mut drivers = serde_json::Map::new();
        for (&sig_id, signal) in bus.all_signals() {
            let dev = signal.value - signal.baseline;
            if dev.abs() >= SIGNAL_DRIVER_THRESHOLD {
                let key = signal_name(sig_id.0);
                let val = if dev >= 0.0 {
                    format!("+{:.3}", dev)
                } else {
                    format!("{:.3}", dev)
                };
                drivers.insert(key, serde_json::Value::String(val));
            }
        }
        serde_json::Value::Object(drivers)
    }

    fn compute_identity_alignment(&self, frame: &SymbolActivationFrame, self_model: &SelfModel) -> f64 {
        if self_model.identity_signature.is_empty() || frame.active.is_empty() {
            return 0.0;
        }
        let identity_indices: std::collections::HashSet<u32> =
            self_model.identity_signature.iter().map(|(idx, _)| *idx).collect();
        let active_count = frame.active.iter()
            .filter(|(idx, _, _)| identity_indices.contains(idx))
            .count();
        active_count as f64 / self_model.identity_signature.len() as f64
    }

    fn append(&mut self, record: serde_json::Value) {
        // Append to disk
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .append(true)
            .create(true)
            .open(&self.log_path)
        {
            if let Ok(line) = serde_json::to_string(&record) {
                let _ = writeln!(file, "{}", line);
            }
        }

        // Maintain rolling buffer; rewrite file when limit hit
        self.rolling.push_back(record);
        if self.rolling.len() > MAX_ROLLING {
            self.rolling.pop_front();
            self.rewrite_log();
        }
    }

    /// Rewrites expression.log from the in-memory rolling buffer.
    /// Called only when the buffer exceeds MAX_ROLLING (once per 10K emits).
    fn rewrite_log(&self) {
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .write(true)
            .truncate(true)
            .create(true)
            .open(&self.log_path)
        {
            for record in &self.rolling {
                if let Ok(line) = serde_json::to_string(record) {
                    let _ = writeln!(file, "{}", line);
                }
            }
        }
    }
}
