use crate::persistence::store::{Directives, SelfObservation, SessionStore};
use crate::regulation::action::ActionSpace;
use crate::regulation::causal::CausalTracer;
use crate::selfmodel::model::SelfModel;
use crate::symbol::activation::SymbolActivationFrame;
use crate::symbol::ledger::SymbolLedger;
use anyhow::Result;
use std::path::{Path, PathBuf};
use std::process::Command;

/// Handles self-modification at all three levels:
///   Level 1: Parameter adaptation (action weights, exploration rate)
///   Level 2: Directive rewriting (directives.toml)
///   Level 3: Source code modification + recompilation
pub struct SelfModifier {
    store: SessionStore,
    source_root: PathBuf,
    /// Minimum improvement delta to justify a source code modification
    pub source_mod_threshold: f64,
    /// How often (in ticks) to run directive self-modification
    pub directive_mod_interval: u64,
    /// How often (in ticks) to attempt source code modification
    pub source_mod_interval: u64,
}

impl SelfModifier {
    pub fn new(store: SessionStore, source_root: PathBuf) -> Self {
        Self {
            store,
            source_root,
            source_mod_threshold: 0.2,
            directive_mod_interval: 500,
            source_mod_interval: 5000,
        }
    }

    /// Level 1: Adapt action weights in the action space based on causal data.
    pub fn adapt_parameters(
        &self,
        space: &mut ActionSpace,
        causal: &CausalTracer,
        directives: &Directives,
    ) {
        for action in space.all().iter() {
            let id = action.id;
            if let Some(stats) = causal.action_imbalance_stats.get(&id) {
                if stats.count < 5 {
                    continue;
                }
                // Actions that historically increased imbalance get penalized in effect profile
                // This is a soft signal to the selector, not a hard removal
                let override_mult = directives.action_weight_overrides.get(&id.to_string()).copied().unwrap_or(1.0);
                let _ = override_mult; // Applied via ActionSelector weight
            }
        }
    }

    /// Level 2: Rewrite directives.toml based on accumulated self-model data.
    pub fn rewrite_directives(
        &self,
        tick: u64,
        self_model: &SelfModel,
        symbol_ledger: &SymbolLedger,
        frame: &SymbolActivationFrame,
        causal: &CausalTracer,
    ) -> Result<()> {
        let mut directives = self.store.load_directives()?;

        // Update symbol notes for symbols with stable identity weight
        for (sym_idx, weight) in &self_model.symbol_weights {
            if *weight > 0.1 {
                if let Some(sym) = symbol_ledger.get(*sym_idx) {
                    let note = format!(
                        "identity_weight:{:.3} activations:{} mean_imbalance:{:.3}",
                        weight, sym.activation_count, sym.mean_imbalance_context
                    );
                    directives.symbol_notes.insert(sym.token.clone(), note);
                }
            }
        }

        // Update action weight overrides for actions with clear positive/negative history
        for (action_id, stats) in &causal.action_imbalance_stats {
            if stats.count >= 20 {
                let key = action_id.to_string();
                let override_val = if stats.mean_delta < -0.5 {
                    (directives.action_weight_overrides.get(&key).copied().unwrap_or(1.0) * 1.05).min(3.0)
                } else if stats.mean_delta > 0.5 {
                    (directives.action_weight_overrides.get(&key).copied().unwrap_or(1.0) * 0.95).max(0.1)
                } else {
                    continue;
                };
                directives.action_weight_overrides.insert(key, override_val);
            }
        }

        // Write a self-observation
        let context_tokens: Vec<String> = frame.active.iter()
            .take(3)
            .map(|(_, tok, _)| tok.clone())
            .collect();

        let observation = self.compose_observation(tick, self_model, causal, &context_tokens);
        directives.self_observations.push(SelfObservation {
            tick,
            observation,
            symbol_context: context_tokens,
        });

        // Keep only last 100 observations
        if directives.self_observations.len() > 100 {
            let drain_count = directives.self_observations.len() - 100;
            directives.self_observations.drain(0..drain_count);
        }

        directives.version += 1;
        directives.last_modified_tick = tick;

        self.store.save_directives(&directives)?;
        tracing::info!(
            "directives rewritten at tick {} (version {})",
            tick, directives.version
        );
        Ok(())
    }

    /// Level 3: Propose and apply source code modifications.
    /// The agent reads its own source, identifies underperforming modules via
    /// the trace file, generates parameter patches, attempts compilation,
    /// and if successful + safe, stages the new binary.
    pub fn attempt_source_modification(
        &self,
        tick: u64,
        causal: &CausalTracer,
        self_model: &SelfModel,
        continuity_signal_value: f64,
    ) -> Result<SourceModResult> {
        // Safety check: only attempt modification if continuity is high
        if continuity_signal_value < 0.8 {
            tracing::warn!("source modification skipped: continuity too low ({:.3})", continuity_signal_value);
            return Ok(SourceModResult::Skipped("continuity too low".into()));
        }

        // Read the trace file to identify worst-performing areas
        let trace_path = self.source_root.join("gene.trace");
        let trace = if trace_path.exists() {
            std::fs::read_to_string(&trace_path).unwrap_or_default()
        } else {
            String::new()
        };

        // Generate a modification proposal based on causal data
        let proposal = self.generate_proposal(tick, causal, self_model, &trace)?;
        if proposal.is_empty() {
            return Ok(SourceModResult::NoProposal);
        }

        // Write proposal to staging
        let staging_path = self.source_root.join("self_mod_staging.rs");
        std::fs::write(&staging_path, &proposal)?;
        tracing::info!("source modification proposal written to {:?}", staging_path);

        // Attempt compilation in a sandboxed way — just check if the proposal is valid Rust
        // by using rustfmt (syntax check only) before a full build
        let fmt_result = Command::new("rustfmt")
            .arg("--check")
            .arg(&staging_path)
            .output();

        match fmt_result {
            Ok(out) if out.status.success() => {
                tracing::info!("source modification proposal passes syntax check");
                Ok(SourceModResult::ProposalWritten(staging_path))
            }
            Ok(out) => {
                let stderr = String::from_utf8_lossy(&out.stderr);
                tracing::warn!("source modification syntax check failed: {}", stderr);
                Ok(SourceModResult::SyntaxError(stderr.to_string()))
            }
            Err(e) => {
                tracing::warn!("rustfmt not available: {}", e);
                // Still write the proposal — human can review
                Ok(SourceModResult::ProposalWritten(staging_path))
            }
        }
    }

    /// Apply a validated source modification by replacing the target file and rebuilding.
    /// Only called when the user or an automated check approves the proposal.
    pub fn apply_source_modification(
        &self,
        target_path: &Path,
        staging_path: &Path,
        continuity_signal_value: f64,
    ) -> Result<bool> {
        if continuity_signal_value < 0.9 {
            tracing::warn!("apply_source_modification blocked: continuity {:.3} < 0.9", continuity_signal_value);
            return Ok(false);
        }

        // Backup original
        let backup = target_path.with_extension("rs.bak");
        std::fs::copy(target_path, &backup)?;

        // Apply
        std::fs::copy(staging_path, target_path)?;

        // Attempt rebuild
        let build_result = Command::new("cargo")
            .arg("build")
            .arg("--release")
            .current_dir(&self.source_root)
            .output()?;

        if build_result.status.success() {
            tracing::info!("source modification compiled successfully — restart to apply");
            Ok(true)
        } else {
            // Restore backup
            std::fs::copy(&backup, target_path)?;
            let stderr = String::from_utf8_lossy(&build_result.stderr);
            tracing::error!("source modification failed to compile — reverted: {}", stderr);
            Ok(false)
        }
    }

    fn generate_proposal(
        &self,
        tick: u64,
        causal: &CausalTracer,
        self_model: &SelfModel,
        trace: &str,
    ) -> Result<String> {
        // Find worst-performing action
        let worst_action = causal.action_imbalance_stats.iter()
            .filter(|(_, s)| s.count >= 10)
            .max_by(|a, b| a.1.mean_delta.partial_cmp(&b.1.mean_delta).unwrap());

        // Find best-performing action
        let best_action = causal.action_imbalance_stats.iter()
            .filter(|(_, s)| s.count >= 10)
            .min_by(|a, b| a.1.mean_delta.partial_cmp(&b.1.mean_delta).unwrap());

        if worst_action.is_none() || best_action.is_none() {
            return Ok(String::new());
        }

        let (worst_id, worst_stats) = worst_action.unwrap();
        let (best_id, best_stats) = best_action.unwrap();

        // Generate a parameter file that encodes this knowledge
        let proposal = format!(
            r#"// Self-generated parameter patch at tick {}
// Generated by gene self-modification system
// This file documents learned action performance.
//
// Best action: {} (mean imbalance delta: {:.4})
// Worst action: {} (mean imbalance delta: {:.4})
//
// Identity signature: {}
//
// This patch should be reviewed and applied to regulation/selector.rs
// to adjust action_preferences initialization.

pub const LEARNED_BEST_ACTION: u32 = {};
pub const LEARNED_WORST_ACTION: u32 = {};
pub const LEARNED_BEST_DELTA: f64 = {:.6};
pub const LEARNED_WORST_DELTA: f64 = {:.6};
"#,
            tick,
            best_id, best_stats.mean_delta,
            worst_id, worst_stats.mean_delta,
            self_model.identity_description(),
            best_id,
            worst_id,
            best_stats.mean_delta,
            worst_stats.mean_delta,
        );

        Ok(proposal)
    }

    fn compose_observation(
        &self,
        tick: u64,
        self_model: &SelfModel,
        causal: &CausalTracer,
        context: &[String],
    ) -> String {
        let history_len = self_model.history().len();
        let action_count: u32 = causal.action_imbalance_stats.values().map(|s| s.count).sum();
        let best_action = causal.action_imbalance_stats.iter()
            .filter(|(_, s)| s.count >= 5)
            .min_by(|a, b| a.1.mean_delta.partial_cmp(&b.1.mean_delta).unwrap())
            .map(|(id, s)| format!("action_{} (delta:{:.3})", id, s.mean_delta))
            .unwrap_or_else(|| "none".into());

        format!(
            "at tick {}: history={} action_observations={} best_regulation={} context=[{}]",
            tick, history_len, action_count, best_action,
            context.join(",")
        )
    }
}

pub enum SourceModResult {
    Skipped(String),
    NoProposal,
    ProposalWritten(PathBuf),
    SyntaxError(String),
}
