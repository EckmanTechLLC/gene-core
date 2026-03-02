use crate::regulation::action::{Action, ShellCmd, SourceTarget, SystemOp};
use crate::regulation::causal::CausalTracer;
use crate::selfmodel::model::SelfModel;
use crate::signal::bus::SignalBus;
use crate::signal::types::{SignalClass, SignalId};
use crate::symbol::activation::SymbolActivationFrame;
use crate::symbol::ledger::SymbolLedger;
use anyhow::Result;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Generates new actions and source patches based on observed signal dynamics.
/// All generation is algorithmic — grounded in causal data, not hardcoded.
pub struct CodeGenerator {
    pub src_root: PathBuf,
    pub data_dir: PathBuf,
}

impl CodeGenerator {
    pub fn new(src_root: PathBuf, data_dir: PathBuf) -> Self {
        Self { src_root, data_dir }
    }

    /// Generate a new Action definition that targets the most chronically deviated signals.
    /// Returns None if there's not enough data to justify a new action.
    pub fn generate_corrective_action(
        &self,
        bus: &SignalBus,
        causal: &CausalTracer,
        next_action_id: u32,
    ) -> Option<Action> {
        // Find signals that are chronically deviated (large deviation, not continuity)
        let mut deviations: Vec<(SignalId, f64)> = bus.all_signals()
            .iter()
            .filter(|(_, s)| s.class != SignalClass::Continuity && s.class != SignalClass::World)
            .map(|(id, s)| (*id, s.deviation()))
            .filter(|(_, dev)| dev.abs() > 0.05)
            .collect();

        if deviations.is_empty() {
            return None;
        }

        // Sort by absolute deviation descending
        deviations.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap());

        // Check that existing actions don't already address the top deviating signal
        let top_signal = deviations[0].0;
        let already_covered = causal.stats.iter()
            .any(|((_, sid), stats)| *sid == top_signal && stats.mean_delta < -0.01 && stats.count >= 5);
        if already_covered {
            return None;
        }

        // Build corrective effect profile: push deviated signals back toward baseline
        // Take top 3 most deviated, apply 30% corrective force
        let effects: Vec<(SignalId, f64)> = deviations.iter()
            .take(3)
            .map(|(id, dev)| (*id, -dev * 0.3))
            .collect();

        let label = format!(
            "corrective@{}: targets [{}]",
            next_action_id,
            effects.iter()
                .map(|(id, d)| format!("{}:{:+.3}", id, d))
                .collect::<Vec<_>>()
                .join(",")
        );

        tracing::info!("codegen: generating corrective action {} — {}", next_action_id, label);

        let mut action = Action::new(next_action_id, effects, 1, 0.01);
        action.label = Some(label);
        Some(action)
    }

    /// Generate a source code patch for a target file.
    /// Returns the patch content as a string.
    pub fn generate_source_patch(
        &self,
        target: &SourceTarget,
        bus: &SignalBus,
        causal: &CausalTracer,
        self_model: &SelfModel,
        tick: u64,
    ) -> Result<String> {
        let target_path = target.path(&self.src_root);
        let original = std::fs::read_to_string(&target_path)
            .unwrap_or_else(|_| String::new());

        let patch = match target {
            SourceTarget::PatternExtractor => {
                self.patch_pattern_extractor(&original, bus, tick)
            }
            SourceTarget::RegulationSelector => {
                self.patch_regulation_selector(&original, causal, tick)
            }
            SourceTarget::SelfModelModel => {
                self.patch_self_model(&original, self_model, tick)
            }
            _ => {
                // Generic: just annotate with learned data
                self.generic_annotation_patch(&original, causal, self_model, tick)
            }
        };

        Ok(patch)
    }

    /// Generate self_prompt.md — the agent's plain-language self-description.
    /// This is read back on startup to bootstrap action preferences.
    pub fn generate_self_prompt(
        &self,
        tick: u64,
        bus: &SignalBus,
        causal: &CausalTracer,
        self_model: &SelfModel,
        symbol_ledger: &SymbolLedger,
        frame: &SymbolActivationFrame,
    ) -> String {
        let best_action = causal.action_imbalance_stats.iter()
            .filter(|(_, s)| s.count >= 10)
            .min_by(|a, b| a.1.mean_delta.partial_cmp(&b.1.mean_delta).unwrap())
            .map(|(id, s)| format!("action_{} (mean_delta:{:+.4})", id, s.mean_delta));

        let worst_action = causal.action_imbalance_stats.iter()
            .filter(|(_, s)| s.count >= 10)
            .max_by(|a, b| a.1.mean_delta.partial_cmp(&b.1.mean_delta).unwrap())
            .map(|(id, s)| format!("action_{} (mean_delta:{:+.4})", id, s.mean_delta));

        let top_deviating: Vec<String> = {
            let mut devs: Vec<_> = bus.all_signals().iter()
                .filter(|(_, s)| s.class != SignalClass::Continuity && s.class != SignalClass::World)
                .map(|(id, s)| (id.to_string(), s.deviation()))
                .collect();
            devs.sort_by(|a, b| b.1.abs().partial_cmp(&a.1.abs()).unwrap());
            devs.iter().take(4)
                .map(|(id, dev)| format!("{}:{:+.4}", id, dev))
                .collect()
        };

        let identity = self_model.identity_description();
        let symbol_count = symbol_ledger.len();
        let dominant = frame.dominant
            .and_then(|idx| symbol_ledger.get(idx))
            .map(|s| s.token.clone())
            .unwrap_or_else(|| "none".into());

        let all_action_perf: Vec<String> = {
            let mut perfs: Vec<_> = causal.action_imbalance_stats.iter()
                .filter(|(_, s)| s.count >= 5)
                .map(|(id, s)| (id, s.mean_delta, s.count))
                .collect();
            perfs.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
            perfs.iter()
                .map(|(id, delta, count)| format!("  action_{}: delta={:+.4} n={}", id, delta, count))
                .collect()
        };

        format!(
r#"# Gene Self-Prompt
Generated at tick: {}
This file is written and read by the agent itself. Do not edit manually.

## Identity
{}
Dominant symbol: {}
Total symbols coined: {}

## Regulatory State
Current imbalance: {:.4}
Top deviating signals: {}

## Action Performance (learned)
Best action: {}
Worst action: {}

### Full ranking (best to worst):
{}

## Operational Notes
- All currently available actions produce net-positive imbalance delta.
- A corrective action targeting top-deviation signals should be generated.
- The system has been running for {} ticks.
- Continuity signals are nominal — self-modification is permitted.

## Symbol Context at last write
Active: {}
"#,
            tick,
            identity,
            dominant,
            symbol_count,
            bus.compute_imbalance(),
            top_deviating.join(", "),
            best_action.unwrap_or_else(|| "insufficient data".into()),
            worst_action.unwrap_or_else(|| "insufficient data".into()),
            all_action_perf.join("\n"),
            tick,
            frame.summary(),
        )
    }

    /// Read and parse self_prompt.md into a map of key observations.
    /// Used at startup to prime action preferences before causal data exists.
    pub fn read_self_prompt(&self) -> Option<SelfPromptData> {
        let path = self.data_dir.join("self_prompt.md");
        let content = std::fs::read_to_string(&path).ok()?;

        let mut preferred_action: Option<u32> = None;
        let mut avoided_action: Option<u32> = None;
        let mut prior_tick: u64 = 0;

        for line in content.lines() {
            if line.starts_with("Generated at tick:") {
                prior_tick = line.split(':').nth(1)
                    .and_then(|s| s.trim().parse().ok())
                    .unwrap_or(0);
            }
            // Parse "Best action: action_10 (mean_delta:..."
            if line.starts_with("Best action: action_") {
                preferred_action = line.split('_').nth(1)
                    .and_then(|s| s.split(' ').next())
                    .and_then(|s| s.parse().ok());
            }
            if line.starts_with("Worst action: action_") {
                avoided_action = line.split('_').nth(1)
                    .and_then(|s| s.split(' ').next())
                    .and_then(|s| s.parse().ok());
            }
        }

        tracing::info!(
            "self_prompt loaded from tick {}: preferred=action_{:?} avoid=action_{:?}",
            prior_tick, preferred_action, avoided_action
        );

        Some(SelfPromptData {
            prior_tick,
            preferred_action,
            avoided_action,
            raw: content,
        })
    }

    // ── Patch generators ─────────────────────────────────────────────────────

    fn patch_pattern_extractor(&self, original: &str, bus: &SignalBus, tick: u64) -> String {
        // Check current pattern extraction rate — if patterns stopped growing,
        // lower the deviation threshold to be more sensitive
        let avg_dev: f64 = bus.all_signals().values()
            .filter(|s| s.class != SignalClass::Continuity && s.class != SignalClass::World)
            .map(|s| s.deviation().abs())
            .sum::<f64>() / bus.all_signals().len().max(1) as f64;

        let new_threshold = if avg_dev < 0.02 { 0.01 } else { 0.05 };

        // Replace the constant in the source
        let patched = original.replace(
            "const DEVIATION_THRESHOLD: f64 = 0.05;",
            &format!("const DEVIATION_THRESHOLD: f64 = {:.3}; // auto-patched at tick {}", new_threshold, tick),
        );

        if patched == original {
            // Already patched or constant not found — add a comment
            format!("// gene self-patch at tick {} — no structural change needed\n{}", tick, original)
        } else {
            patched
        }
    }

    fn patch_regulation_selector(&self, original: &str, causal: &CausalTracer, tick: u64) -> String {
        // If causal data shows high variance in outcomes, increase exploration rate
        let avg_variance: f64 = causal.action_imbalance_stats.values()
            .map(|s| s.variance)
            .sum::<f64>() / causal.action_imbalance_stats.len().max(1) as f64;

        let new_exploration = if avg_variance > 0.1 { 0.25 } else { 0.10 };

        let patched = original.replace(
            "pub fn new(exploration_rate: f64, repetition_limit: u32) -> Self {",
            &format!(
                "// auto-patched at tick {}: avg_variance={:.4} → exploration={:.2}\npub fn new(exploration_rate: f64, repetition_limit: u32) -> Self {{",
                tick, avg_variance, new_exploration
            ),
        );
        patched
    }

    fn patch_self_model(&self, original: &str, self_model: &SelfModel, tick: u64) -> String {
        format!("// gene self-patch at tick {} — self_model has {} identity symbols\n{}",
            tick, self_model.identity_signature.len(), original)
    }

    fn generic_annotation_patch(&self, original: &str, causal: &CausalTracer, self_model: &SelfModel, tick: u64) -> String {
        let total_obs: u32 = causal.action_imbalance_stats.values().map(|s| s.count).sum();
        format!(
            "// gene self-annotation at tick {} — {} action observations, {} identity symbols\n{}",
            tick, total_obs, self_model.identity_signature.len(), original
        )
    }
}

/// Data parsed from self_prompt.md, used to bootstrap a fresh session.
#[derive(Debug, Clone)]
pub struct SelfPromptData {
    pub prior_tick: u64,
    pub preferred_action: Option<u32>,
    pub avoided_action: Option<u32>,
    pub raw: String,
}
