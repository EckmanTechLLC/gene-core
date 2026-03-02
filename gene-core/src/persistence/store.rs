use crate::pattern::index::PatternIndex;
use crate::regulation::causal::CausalTracer;
use crate::selfmodel::model::SelfModel;
use crate::signal::bus::SignalBus;
use crate::signal::types::SignalId;
use crate::symbol::ledger::SymbolLedger;
use anyhow::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Serializable snapshot of all mutable agent state.
#[derive(Serialize, Deserialize)]
pub struct AgentCheckpoint {
    pub tick: u64,
    pub signal_values: Vec<(SignalId, f64)>,
    pub causal_tracer: CausalTracer,
    pub pattern_index: PatternIndex,
    pub symbol_ledger: SymbolLedger,
    pub self_model: SelfModel,
    pub action_imbalance_history: Vec<f64>,
}

pub struct SessionStore {
    pub checkpoint_path: PathBuf,
    pub directives_path: PathBuf,
}

impl SessionStore {
    pub fn new(data_dir: &Path) -> Self {
        std::fs::create_dir_all(data_dir).ok();
        Self {
            checkpoint_path: data_dir.join("checkpoint.bin"),
            directives_path: data_dir.join("directives.toml"),
        }
    }

    pub fn save(&self, checkpoint: &AgentCheckpoint) -> Result<()> {
        let bytes = bincode::serialize(checkpoint)?;
        std::fs::write(&self.checkpoint_path, bytes)?;
        tracing::debug!("checkpoint saved at tick {}", checkpoint.tick);
        Ok(())
    }

    pub fn load(&self) -> Result<Option<AgentCheckpoint>> {
        if !self.checkpoint_path.exists() {
            return Ok(None);
        }
        let bytes = std::fs::read(&self.checkpoint_path)?;
        let cp: AgentCheckpoint = bincode::deserialize(&bytes)?;
        tracing::info!("checkpoint loaded from tick {}", cp.tick);
        Ok(Some(cp))
    }

    pub fn load_directives(&self) -> Result<Directives> {
        if !self.directives_path.exists() {
            let defaults = Directives::default();
            self.save_directives(&defaults)?;
            return Ok(defaults);
        }
        let content = std::fs::read_to_string(&self.directives_path)?;
        let d: Directives = toml::from_str(&content)?;
        Ok(d)
    }

    pub fn save_directives(&self, directives: &Directives) -> Result<()> {
        let content = toml::to_string_pretty(directives)?;
        std::fs::write(&self.directives_path, content)?;
        Ok(())
    }
}

/// Self-modifiable operational directives.
/// The agent reads and rewrites this at runtime based on learned outcomes.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Directives {
    /// Version — incremented on each self-modification
    pub version: u32,
    /// Tick of last modification
    pub last_modified_tick: u64,
    /// Per-symbol learned notes (token → note)
    pub symbol_notes: HashMap<String, String>,
    /// Action weight overrides (action_id as string → weight multiplier)
    /// String keys required by TOML serialization.
    pub action_weight_overrides: HashMap<String, f64>,
    /// Exploration rate override (None = use default)
    pub exploration_rate: Option<f64>,
    /// Stagnation limit override
    pub stagnation_limit: Option<u32>,
    /// Self-written observations (free text the agent generates about its own state)
    pub self_observations: Vec<SelfObservation>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelfObservation {
    pub tick: u64,
    pub observation: String,
    /// Symbol context at time of observation
    pub symbol_context: Vec<String>,
}

impl Default for Directives {
    fn default() -> Self {
        Self {
            version: 0,
            last_modified_tick: 0,
            symbol_notes: HashMap::new(),
            action_weight_overrides: HashMap::new(),
            exploration_rate: None,
            stagnation_limit: None,
            self_observations: Vec::new(),
        }
    }
}
