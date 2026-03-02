mod expression;
mod pattern;
mod persistence;
mod regulation;
mod selfmodel;
mod signal;
mod symbol;
mod tui;
mod ipc;

use anyhow::Result;
use clap::Parser;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tracing_subscriber::fmt;
use tracing_appender::non_blocking::WorkerGuard;

use crate::pattern::extractor::PatternExtractor;
use crate::pattern::index::PatternIndex;
use crate::persistence::codegen::CodeGenerator;
use crate::persistence::executor::SystemOpExecutor;
use crate::persistence::selfmod::SelfModifier;
use crate::persistence::store::{AgentCheckpoint, SessionStore};
use crate::regulation::action::{Action, ActionSpace, ShellCmd, SourceTarget, SystemOp};
use crate::regulation::causal::CausalTracer;
use crate::regulation::drive::RegulationDrive;
use crate::regulation::selector::ActionSelector;
use crate::selfmodel::evaluator::ActionEvaluator;
use crate::selfmodel::meta::MetaSignal;
use crate::selfmodel::model::SelfModel;
use crate::signal::bus::SignalBus;
use crate::signal::ledger::SignalLedger;
use crate::signal::types::{DeltaSource, SignalClass, SignalId};
use crate::expression::ExpressionEngine;
use crate::signal::flux::{FluxEarthquakeState, FluxPoller, FluxSignalIds, spawn_flux_ws_task};
use crate::signal::world::{WorldSignalIds, WorldSignalPoller};
use crate::symbol::activation::SymbolActivationFrame;
use crate::symbol::composition::CompositionEngine;
use crate::symbol::grounder::SymbolGrounder;
use crate::symbol::ledger::SymbolLedger;

#[derive(Parser, Debug)]
#[command(name = "gene", about = "Signal-driven recursive self-modeling agent")]
struct Args {
    #[arg(long, default_value = "./gene-data")]
    data_dir: PathBuf,

    #[arg(long, default_value = "0")]
    max_ticks: u64,

    #[arg(long, default_value = "0")]
    tick_floor_us: u64,

    #[arg(long, default_value = "512")]
    disk_quota_mb: u64,

    #[arg(long, default_value_t = true)]
    tui: bool,

    #[arg(long, default_value = "10")]
    signal_count: u32,

    #[arg(long, default_value = "1000")]
    checkpoint_interval: u64,

    #[arg(long, default_value = "/tmp/gene.sock")]
    socket_path: PathBuf,

    #[arg(long, default_value = "wss://api.flux-universe.com/api/ws")]
    flux_url: String,
}

const SIG_CONTINUITY: SignalId = SignalId(0);
const SIG_INTEGRITY:  SignalId = SignalId(1);
const SIG_COHERENCE:  SignalId = SignalId(2);
const SIG_MEMORY:     SignalId = SignalId(3);
const SIG_DISK:       SignalId = SignalId(4);
const SIG_META:       SignalId = SignalId(5);
const SIG_DRIVE:      SignalId = SignalId(6);
const SIG_SOMATIC_START: u32 = 7;
// World signals — fixed IDs, assumes --signal-count ≤ 10 (default)
const SIG_CPU_LOAD:     SignalId = SignalId(17);
const SIG_NET_RX:       SignalId = SignalId(18);
const SIG_NET_TX:       SignalId = SignalId(19);
const SIG_DISK_IO:      SignalId = SignalId(20);
const SIG_UPTIME_CYCLE: SignalId = SignalId(21);
const SIG_PROC_COUNT:   SignalId = SignalId(22);
const SIG_SWAP_USED:    SignalId = SignalId(23);
const SIG_IOWAIT:       SignalId = SignalId(24);
const SIG_CTX_SWITCHES: SignalId = SignalId(25);
const SIG_TICK_RATE:    SignalId = SignalId(26);
const SIG_MEM_AVAILABLE:SignalId = SignalId(27);
// Flux Universe earthquake signals
const SIG_QUAKE_RATE:      SignalId = SignalId(28);
const SIG_QUAKE_MAGNITUDE: SignalId = SignalId(29);
const SIG_QUAKE_DEPTH:     SignalId = SignalId(30);
const SIG_QUAKE_SIG:       SignalId = SignalId(31);

// Forgetting thresholds
const PATTERN_FORGET_TICKS:   u64 = 50_000;
const COMPOSITE_FORGET_TICKS: u64 = 100_000;

fn build_bus(signal_count: u32) -> SignalBus {
    let mut bus = SignalBus::new();
    bus.register_with_id(SIG_CONTINUITY, SignalClass::Continuity, 1.0, 0.0, 50.0);
    bus.register_with_id(SIG_INTEGRITY,  SignalClass::Continuity, 1.0, 0.0, 30.0);
    bus.register_with_id(SIG_COHERENCE,  SignalClass::Continuity, 1.0, 0.0, 20.0);
    bus.register_with_id(SIG_MEMORY, SignalClass::Derived, 0.5, 0.01, 5.0);
    bus.register_with_id(SIG_DISK,   SignalClass::Derived, 0.5, 0.01, 3.0);
    bus.register_with_id(SIG_META,   SignalClass::Derived, 0.5, 0.001, 2.0);
    bus.register_with_id(SIG_DRIVE,  SignalClass::Derived, 0.0, 0.05,  1.0);

    use rand::Rng;
    let mut rng = rand::thread_rng();
    for i in 0..signal_count {
        let id = SignalId(SIG_SOMATIC_START + i);
        let baseline   = rng.gen_range(0.1_f64..0.9);
        let decay_rate = rng.gen_range(0.001_f64..0.05);
        let weight     = rng.gen_range(0.5_f64..3.0);
        bus.register_with_id(id, SignalClass::Somatic, baseline, decay_rate, weight);
    }

    // World signals — polled from OS every 10 ticks, start after somatic range
    bus.register_with_id(SIG_CPU_LOAD,      SignalClass::World, 0.2,  0.005, 2.0);
    bus.register_with_id(SIG_NET_RX,        SignalClass::World, 0.1,  0.01,  1.5);
    bus.register_with_id(SIG_NET_TX,        SignalClass::World, 0.1,  0.01,  1.5);
    bus.register_with_id(SIG_DISK_IO,       SignalClass::World, 0.1,  0.01,  1.5);
    bus.register_with_id(SIG_UPTIME_CYCLE,  SignalClass::World, 0.5,  0.001, 1.0);
    bus.register_with_id(SIG_PROC_COUNT,    SignalClass::World, 0.3,  0.005, 1.0);
    bus.register_with_id(SIG_SWAP_USED,     SignalClass::World, 0.1,  0.005, 1.5);
    bus.register_with_id(SIG_IOWAIT,        SignalClass::World, 0.05, 0.01,  1.5);
    bus.register_with_id(SIG_CTX_SWITCHES,  SignalClass::World, 0.1,  0.02,  1.0);
    bus.register_with_id(SIG_TICK_RATE,     SignalClass::World, 0.7,  0.001, 1.5);
    bus.register_with_id(SIG_MEM_AVAILABLE, SignalClass::World, 0.3,  0.005, 2.0);
    // Flux Universe earthquake signals — observation-only (weight=0: no imbalance contribution)
    bus.register_with_id(SIG_QUAKE_RATE,      SignalClass::World, 0.1, 0.001, 0.0);
    bus.register_with_id(SIG_QUAKE_MAGNITUDE, SignalClass::World, 0.1, 0.001, 0.0);
    bus.register_with_id(SIG_QUAKE_DEPTH,     SignalClass::World, 0.5, 0.001, 0.0);
    bus.register_with_id(SIG_QUAKE_SIG,       SignalClass::World, 0.1, 0.001, 0.0);

    bus
}

fn build_initial_action_space(signal_count: u32, somatic_start: u32) -> ActionSpace {
    use rand::Rng;
    let mut rng = rand::thread_rng();
    let mut actions = Vec::new();
    let somatic_ids: Vec<SignalId> = (somatic_start..somatic_start + signal_count)
        .map(SignalId)
        .collect();

    // 12 random somatic actions
    for action_id in 0..12u32 {
        let n_affected = rng.gen_range(2..=4usize).min(somatic_ids.len());
        let mut effects = Vec::new();
        let mut indices: Vec<usize> = (0..somatic_ids.len()).collect();
        for i in 0..n_affected {
            let j = i + rng.gen_range(0..somatic_ids.len() - i);
            indices.swap(i, j);
        }
        for &idx in &indices[..n_affected] {
            let delta = rng.gen_range(-0.2_f64..0.2);
            if delta.abs() > 0.02 {
                effects.push((somatic_ids[idx], delta));
            }
        }
        actions.push(Action::new(action_id, effects, rng.gen_range(1..=3u32), rng.gen_range(0.0_f64..0.05)));
    }

    // ── System actions (IDs 100+, well clear of somatic space) ────────────

    // 100: generate a corrective action targeting chronic deviations
    actions.push(
        Action::new(100, vec![], 1, 0.02)
            .with_system_op(SystemOp::GenAction, 0.7)
    );

    // 101: write self_prompt.md
    actions.push(
        Action::new(101, vec![], 1, 0.01)
            .with_system_op(SystemOp::WritePrompt, 0.6)
    );

    // 102: read self_prompt.md back (verify coherence)
    actions.push(
        Action::new(102, vec![], 1, 0.005)
            .with_system_op(SystemOp::ReadPrompt, 0.5)
    );

    // 103: generate source patch for pattern extractor
    actions.push(
        Action::new(103, vec![], 1, 0.03)
            .with_system_op(
                SystemOp::GenSourcePatch { target: SourceTarget::PatternExtractor },
                0.8,
            )
    );

    // 104: generate source patch for regulation selector
    actions.push(
        Action::new(104, vec![], 1, 0.03)
            .with_system_op(
                SystemOp::GenSourcePatch { target: SourceTarget::RegulationSelector },
                0.8,
            )
    );

    // 105: cargo build (compile after patch)
    actions.push(
        Action::new(105, vec![], 1, 0.1)
            .with_system_op(SystemOp::CargoBuild, 0.85)
    );

    // 106: read own source — pattern extractor
    actions.push(
        Action::new(106, vec![], 1, 0.005)
            .with_system_op(
                SystemOp::ReadFile {
                    path: PathBuf::from("gene-core/src/pattern/extractor.rs")
                },
                0.5,
            )
    );

    // 107: read own source — regulation selector
    actions.push(
        Action::new(107, vec![], 1, 0.005)
            .with_system_op(
                SystemOp::ReadFile {
                    path: PathBuf::from("gene-core/src/regulation/selector.rs")
                },
                0.5,
            )
    );

    // 108: reload actions.json (hot-reload)
    actions.push(
        Action::new(108, vec![], 1, 0.01)
            .with_system_op(SystemOp::ReloadActions, 0.5)
    );

    // 109: apply and restart with new binary
    actions.push(
        Action::new(109, vec![], 1, 0.2)
            .with_system_op(SystemOp::ApplyAndRestart, 0.95)
    );

    // 110: renice gene to niceness 10 — yield CPU to the system
    // Predicts tick rate drops (lower priority = slower execution)
    actions.push(
        Action::new(110, vec![(SIG_TICK_RATE, -0.15)], 1, 0.005)
            .with_system_op(SystemOp::Renice { niceness: 10 }, 0.60)
    );

    // 111: restore gene to normal priority (niceness 0)
    // Predicts tick rate recovers (normal priority = faster execution)
    actions.push(
        Action::new(111, vec![(SIG_TICK_RATE, 0.15)], 1, 0.005)
            .with_system_op(SystemOp::Renice { niceness: 0 }, 0.50)
    );

    // 112: spawn stress-ng stressor (1 CPU + 1 VM worker, 120s timeout)
    // Predicts CPU load rises and tick rate drops
    actions.push(
        Action::new(112, vec![(SIG_CPU_LOAD, 0.3), (SIG_TICK_RATE, -0.15)], 1, 0.02)
            .with_system_op(SystemOp::SpawnStressor, 0.70)
    );

    // 113: kill any running stress-ng stressor
    // Predicts CPU load falls and tick rate recovers
    actions.push(
        Action::new(113, vec![(SIG_CPU_LOAD, -0.2), (SIG_TICK_RATE, 0.1)], 1, 0.005)
            .with_system_op(SystemOp::KillStressor, 0.50)
    );

    // 114: drop page caches to reduce memory pressure
    // Predicts memory pressure falls
    actions.push(
        Action::new(114, vec![(SIG_MEMORY, -0.15)], 1, 0.01)
            .with_system_op(SystemOp::DropCaches, 0.65)
    );

    ActionSpace::new(actions)
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();

    // When TUI is active, logs must go to a file — stdout belongs to ratatui.
    // Keep the guard alive for the duration of main so the log writer isn't dropped.
    let tui_would_enable = args.tui && std::io::stdout().is_terminal();
    let _log_guard: Option<WorkerGuard> = if tui_would_enable {
        std::fs::create_dir_all(&args.data_dir).ok();
        let file_appender = tracing_appender::rolling::never(&args.data_dir, "gene.log");
        let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);
        tracing_subscriber::fmt()
            .with_writer(non_blocking)
            .with_ansi(false)
            .init();
        Some(guard)
    } else {
        fmt::init();
        None
    };

    tracing::info!("gene starting up");

    std::fs::create_dir_all(&args.data_dir)?;
    let ledger_path  = args.data_dir.join("ledger");
    let quota_entries = (args.disk_quota_mb * 1024 * 1024) / 256;

    let mut bus          = build_bus(args.signal_count);
    // Baselines never change after registration — build once for use in pattern extraction.
    let signal_baselines: std::collections::HashMap<crate::signal::types::SignalId, f64> =
        bus.all_signals().iter().map(|(id, s)| (*id, s.baseline)).collect();
    let mut action_space = build_initial_action_space(args.signal_count, SIG_SOMATIC_START);
    let mut ledger       = SignalLedger::open(&ledger_path, quota_entries)?;
    let mut causal       = CausalTracer::new(10_000);
    let mut pattern_extractor = PatternExtractor::new();
    let mut pattern_index     = PatternIndex::new();
    let mut symbol_ledger     = SymbolLedger::new();
    let     symbol_grounder   = SymbolGrounder::new(0.05);
    let mut self_model   = SelfModel::new(2000, 10);
    let mut meta         = MetaSignal::new(SIG_META);
    let     evaluator    = ActionEvaluator::new();
    let mut drive        = RegulationDrive::new(SIG_DRIVE, 50);
    let mut selector     = ActionSelector::new(0.15, 30);
    let mut expr_engine        = ExpressionEngine::new(&args.data_dir, 100);
    let mut composition_engine = CompositionEngine::new(20);
    let world_ids = WorldSignalIds {
        cpu_load:     SIG_CPU_LOAD,
        net_rx:       SIG_NET_RX,
        net_tx:       SIG_NET_TX,
        disk_io:      SIG_DISK_IO,
        uptime_cycle: SIG_UPTIME_CYCLE,
        proc_count:   SIG_PROC_COUNT,
        swap_used:    SIG_SWAP_USED,
        iowait:       SIG_IOWAIT,
        ctx_switches: SIG_CTX_SWITCHES,
        mem_available:SIG_MEM_AVAILABLE,
    };
    let mut world_poller = WorldSignalPoller::new();

    // System op executor
    let src_root = std::env::current_dir()?.join("gene-core/src");
    let executor = SystemOpExecutor::new(
        args.data_dir.clone(),
        src_root.clone(),
        std::env::current_dir()?,
        SIG_INTEGRITY,
        SIG_COHERENCE,
        SIG_CONTINUITY,
    );

    // CodeGenerator for startup self_prompt bootstrap
    let codegen = CodeGenerator::new(src_root, args.data_dir.clone());

    // Load checkpoint
    let store    = SessionStore::new(&args.data_dir);
    let mut tick: u64 = 0;

    match store.load() {
        Ok(Some(cp)) => {
            tick = cp.tick;
            for (id, val) in &cp.signal_values {
                bus.set_value(*id, *val);
            }
            causal        = cp.causal_tracer;
            pattern_index = cp.pattern_index;
            symbol_ledger = cp.symbol_ledger;
            self_model    = cp.self_model;
            composition_engine.seed_from_ledger(&symbol_ledger);
            tracing::info!("resumed from tick {}", tick);
        }
        Ok(None) => {}
        Err(e) => {
            tracing::warn!("checkpoint load failed (schema change?), starting fresh: {}", e);
        }
    }

    // Bootstrap action preferences from self_prompt.md if it exists
    if let Some(prompt_data) = codegen.read_self_prompt() {
        if prompt_data.prior_tick > 0 {
            tracing::info!(
                "bootstrapping from self_prompt (prior tick {}): prefer={:?} avoid={:?}",
                prompt_data.prior_tick, prompt_data.preferred_action, prompt_data.avoided_action
            );
            // Prime self-model action preferences from prior session
            if let Some(preferred) = prompt_data.preferred_action {
                self_model.action_preferences.entry(preferred).or_insert(0.5);
            }
            if let Some(avoided) = prompt_data.avoided_action {
                self_model.action_preferences.entry(avoided).or_insert(-0.5);
            }
        }
    }

    // Load any previously generated actions from actions.json
    let actions_json_path = args.data_dir.join("actions.json");
    let mut actions_json_mtime = get_mtime(&actions_json_path);
    if actions_json_path.exists() {
        if let Ok(json) = std::fs::read_to_string(&actions_json_path) {
            let added = action_space.merge_from_json(&json).unwrap_or(0);
            if added > 0 {
                tracing::info!("loaded {} persisted actions from actions.json", added);
            }
        }
    }

    let self_modifier = SelfModifier::new(
        SessionStore::new(&args.data_dir),
        std::env::current_dir()?,
    );

    // IPC
    let socket_path  = args.socket_path.clone();
    let shared_state = Arc::new(Mutex::new(SharedState::default()));
    let state_clone  = shared_state.clone();
    tokio::spawn(async move {
        if let Err(e) = ipc::serve(socket_path, state_clone).await {
            tracing::warn!("IPC server error: {}", e);
        }
    });

    // Flux Universe earthquake signal source
    let flux_earthquake_state = Arc::new(Mutex::new(FluxEarthquakeState::new()));
    let flux_ids = FluxSignalIds {
        quake_rate:      SIG_QUAKE_RATE,
        quake_magnitude: SIG_QUAKE_MAGNITUDE,
        quake_depth:     SIG_QUAKE_DEPTH,
        quake_sig:       SIG_QUAKE_SIG,
    };
    let mut flux_poller = FluxPoller::new(flux_earthquake_state.clone());
    if !args.flux_url.is_empty() {
        spawn_flux_ws_task(args.flux_url.clone(), flux_earthquake_state);
    }

    // Graceful shutdown
    let running = Arc::new(std::sync::atomic::AtomicBool::new(true));
    let r = running.clone();
    ctrlc::set_handler(move || {
        tracing::info!("shutdown signal received");
        r.store(false, std::sync::atomic::Ordering::SeqCst);
    }).ok();

    let tui_enabled = args.tui && std::io::stdout().is_terminal();
    let mut tui_state = tui::TuiState::new();

    tracing::info!("entering tick loop at tick {}", tick);

    // Tick rate tracking for s_tick_rate signal
    let mut tick_rate_window_start = Instant::now();
    let mut tick_rate_ticks: u64 = 0;

    loop {
        let tick_start = Instant::now();

        if !running.load(std::sync::atomic::Ordering::SeqCst) {
            tracing::info!("shutting down at tick {}", tick);
            break;
        }
        if args.max_ticks > 0 && tick >= args.max_ticks {
            tracing::info!("max ticks {} reached", args.max_ticks);
            break;
        }

        // ── Pause support ─────────────────────────────────────────────────
        if let Ok(state) = shared_state.try_lock() {
            if state.pause_requested {
                drop(state);
                std::thread::sleep(Duration::from_millis(50));
                tick += 1;
                continue;
            }
        }

        // ── Hot-reload actions.json ────────────────────────────────────────
        if tick % 100 == 0 {
            let new_mtime = get_mtime(&actions_json_path);
            if new_mtime != actions_json_mtime {
                if let Ok(json) = std::fs::read_to_string(&actions_json_path) {
                    let added = action_space.merge_from_json(&json).unwrap_or(0);
                    if added > 0 {
                        tracing::info!("hot-reloaded {} new actions from actions.json", added);
                        push_log(&shared_state, tick, &format!("hot-reload: +{} action(s) from actions.json", added));
                    }
                }
                actions_json_mtime = new_mtime;
            }
        }

        // ── Layer 0: Signal tick ──────────────────────────────────────────
        let (imbalance, snapshot) = bus.tick(tick);
        let pre_snapshot = snapshot.clone();
        ledger.append(&snapshot)?;
        pattern_extractor.push(snapshot.clone());
        update_resource_signals(&mut bus, &args.data_dir);
        if tick % 10 == 0 {
            world_poller.poll(&mut bus, &world_ids);
            flux_poller.poll(&mut bus, &flux_ids);
        }

        // ── Layer 1: Regulation ───────────────────────────────────────────
        let urgency = drive.urgency(imbalance, 50.0);
        bus.set_value(SIG_DRIVE, urgency);

        let circuit_break = drive.update(imbalance, tick);
        if circuit_break {
            tracing::warn!("tick {}: stagnation — forcing exploration", tick);
            drive.reset_stagnation();
            selector = ActionSelector::new(0.9, 30);
        }

        let active_signals = pattern_extractor.current_active(0.02, &signal_baselines);

        // ── Layer 3: Symbol activation ────────────────────────────────────
        symbol_grounder.process_salience(&pattern_index, &mut symbol_ledger, tick);
        let _ = symbol_grounder.update_activations(
            &pattern_index, &mut symbol_ledger, &active_signals, imbalance, tick,
        );
        let frame = SymbolActivationFrame::build(tick, &symbol_ledger, 0.1);

        // ── Symbol composition ────────────────────────────────────────────
        composition_engine.observe(&frame.active, &symbol_ledger);
        if tick % 4 == 0 {
            let new_composites = composition_engine.maybe_compose(&mut symbol_ledger, tick);
            if !new_composites.is_empty() {
                tracing::info!("tick {}: coined {} composite(s)", tick, new_composites.len());
                for idx in &new_composites {
                    if let Some(sym) = symbol_ledger.get(*idx) {
                        let parents_str: String = sym.parents.iter()
                            .filter_map(|p| symbol_ledger.get(*p))
                            .map(|p| p.token.as_str())
                            .collect::<Vec<_>>()
                            .join(" × ");
                        let msg = if parents_str.is_empty() {
                            format!("coined {}", sym.token)
                        } else {
                            format!("coined {} ({}) ", sym.token, parents_str)
                        };
                        push_log(&shared_state, tick, &msg);
                    }
                }
            }
        }

        // ── Layer 4: Select action ────────────────────────────────────────
        let chosen_action_id = evaluator.select(
            &bus, &action_space, &causal, &mut selector,
            &self_model, &meta, &frame, urgency, tick,
        );

        // ── Execute chosen action ─────────────────────────────────────────
        if let Some(action_id) = chosen_action_id {
            if let Some(action) = action_space.get(action_id) {
                // Apply signal effect profile
                for (&sig_id, &delta) in &action.effect_profile {
                    bus.queue_delta(sig_id, delta, DeltaSource::Action(action_id));
                }

                // Execute system op if present
                if let Some(op) = &action.system_op.clone() {
                    let continuity = bus.get_value(SIG_CONTINUITY);
                    let gate       = action.continuity_gate;
                    let next_id    = action_space.next_id;

                    let (result, new_action) = executor.execute(
                        op, &bus, &causal, &self_model,
                        &symbol_ledger, &frame,
                        next_id, tick, continuity, gate,
                    );

                    // Log system op result to activity console
                    {
                        let label   = op_label(op);
                        let status  = if result.success { "ok" } else { "FAIL" };
                        let snippet = result.output.lines().next().unwrap_or("").trim();
                        let snippet = if snippet.len() > 60 { &snippet[..60] } else { snippet };
                        push_log(&shared_state, tick, &format!("sys:{} → {} {}", label, status, snippet));
                    }

                    // Feed signal results back into bus
                    for (sig_id, delta) in result.signal_feedback {
                        bus.queue_delta(sig_id, delta, DeltaSource::Action(action_id));
                    }

                    // If execution generated a new action, add it to the space
                    if let Some(new_act) = new_action {
                        let new_id = action_space.add(new_act);
                        tracing::info!("tick {}: new action {} added to space", tick, new_id);
                        push_log(&shared_state, tick, &format!("new action a_{} added to space", new_id));
                    }

                    if !result.success {
                        tracing::debug!("tick {}: system op failed: {}", tick, &result.output[..result.output.len().min(120)]);
                    }
                }
            }
        }

        // ── Second bus tick to apply queued deltas ────────────────────────
        let (post_imbalance, post_snapshot) = bus.tick(tick);

        // ── Layer 2: Pattern extraction ───────────────────────────────────
        if tick % 4 == 0 {
            if let Some(result) = pattern_extractor.extract(&signal_baselines) {
                pattern_index.integrate(result, chosen_action_id);
            }
        }

        // ── Layer 4: Causal + self-model ──────────────────────────────────
        if let Some(action_id) = chosen_action_id {
            causal.record(action_id, tick, &pre_snapshot, &post_snapshot);
        }

        meta.update(imbalance, post_imbalance);
        bus.set_value(SIG_META, meta.bus_value());

        let imbalance_delta = post_imbalance - imbalance;
        self_model.update(tick, &frame, post_imbalance, chosen_action_id, imbalance_delta);

        // ── Expression layer ──────────────────────────────────────────────
        if let Some(record) = expr_engine.maybe_emit(
            tick, &frame, &bus, &self_model, &meta, chosen_action_id, &causal,
        ) {
            if let Ok(mut state) = shared_state.try_lock() {
                state.recent_expressions.push(record);
                if state.recent_expressions.len() > 50 {
                    state.recent_expressions.remove(0);
                }
            }
        }

        // ── Layer 5: Periodic self-modification ───────────────────────────
        if tick > 0 && tick % self_modifier.directive_mod_interval == 0 {
            if let Err(e) = self_modifier.rewrite_directives(
                tick, &self_model, &symbol_ledger, &frame, &causal,
            ) {
                tracing::warn!("directive rewrite failed: {}", e);
            }
        }

        // ── Layer 5: Checkpoint ───────────────────────────────────────────
        if tick > 0 && tick % args.checkpoint_interval == 0 {
            let cp = AgentCheckpoint {
                tick,
                signal_values: bus.snapshot_values(),
                causal_tracer: causal.clone_partial(),
                pattern_index: pattern_index.clone(),
                symbol_ledger: symbol_ledger.clone(),
                self_model:    self_model.clone(),
                action_imbalance_history: Vec::new(),
            };
            if let Err(e) = store.save(&cp) {
                tracing::warn!("checkpoint failed: {}", e);
                push_log(&shared_state, tick, &format!("checkpoint FAIL: {}", e));
            } else {
                push_log(&shared_state, tick, "checkpoint saved");
            }
            ledger.flush()?;
        }

        // ── IPC state update ──────────────────────────────────────────────
        if tick % 10 == 0 {
            if let Ok(mut state) = shared_state.try_lock() {
                state.tick          = tick;
                state.imbalance     = post_imbalance;
                state.last_action   = chosen_action_id;
                state.active_symbols = frame.active.iter()
                    .map(|(_, tok, s)| (tok.clone(), *s))
                    .collect();
                state.identity      = self_model.identity_description();
                state.pattern_count = pattern_index.len();
                state.symbol_count  = symbol_ledger.len();
                state.confidence    = meta.confidence;
                state.action_count    = action_space.all().len();
                state.composite_count = symbol_ledger.composites().count();

                // Apply any pending injection from gene-ctl
                if let Some((sig_id, delta)) = state.pending_inject.take() {
                    bus.inject(sig_id, delta);
                    tracing::info!("injected {} → {}", delta, sig_id);
                }
            }
        }

        // ── TUI ───────────────────────────────────────────────────────────
        if tui_enabled && tick % 20 == 0 {
            let (paused, recent_expressions, activity_log) = if let Ok(st) = shared_state.try_lock() {
                (st.pause_requested, st.recent_expressions.clone(), st.activity_log.clone())
            } else {
                (false, Vec::new(), Vec::new())
            };
            tui_state.update(
                tick, post_imbalance, &bus, &frame,
                chosen_action_id, meta.confidence,
                pattern_index.len(), symbol_ledger.len(),
                symbol_ledger.composites().count(),
                paused,
                recent_expressions,
                activity_log,
            );
            match tui_state.render() {
                Ok((injects, weight_changes)) => {
                    for (sig_id, delta) in injects {
                        bus.inject(crate::signal::types::SignalId(sig_id), delta);
                        push_log(&shared_state, tick,
                            &format!("stress: s_{:04} {:+.3}", sig_id, delta));
                        tracing::info!("stress inject: s_{:04} {:+.3}", sig_id, delta);
                    }
                    for (sig_id, new_weight) in weight_changes {
                        bus.set_weight(crate::signal::types::SignalId(sig_id), new_weight);
                        push_log(&shared_state, tick,
                            &format!("weight: s_{:04} → {:.2}", sig_id, new_weight));
                        tracing::info!("weight: s_{:04} → {:.2}", sig_id, new_weight);
                    }
                }
                Err(e) => tracing::debug!("tui render: {}", e),
            }
        }

        // ── Tick rate signal ──────────────────────────────────────────────
        tick_rate_ticks += 1;
        if tick_rate_ticks >= 200 {
            let elapsed = tick_rate_window_start.elapsed().as_secs_f64();
            let tps = tick_rate_ticks as f64 / elapsed.max(0.001);
            // log2-normalized: log2(32768) ≈ 15 as practical ceiling (~32K ticks/s)
            let norm = (tps.log2().max(0.0) / 15.0).clamp(0.0, 1.0);
            let cur = bus.get_value(SIG_TICK_RATE);
            bus.set_value(SIG_TICK_RATE, cur * 0.7 + norm * 0.3);
            tick_rate_window_start = Instant::now();
            tick_rate_ticks = 0;
        }

        // ── Forgetting / pruning ──────────────────────────────────────────
        if tick > 0 && tick % 5000 == 0 {
            let protected: std::collections::HashSet<u64> = symbol_ledger.all()
                .filter(|s| !s.is_composite && s.pattern_id != 0)
                .map(|s| s.pattern_id)
                .collect();
            let pruned_patterns = pattern_index.prune_stale(tick, PATTERN_FORGET_TICKS, &protected);
            let pruned_composites = symbol_ledger.prune_composites(tick, COMPOSITE_FORGET_TICKS);
            if !pruned_composites.is_empty() {
                composition_engine.purge_symbols(&pruned_composites);
            }
            if pruned_patterns > 0 || !pruned_composites.is_empty() {
                let msg = format!("pruned: {} pattern(s), {} composite(s)", pruned_patterns, pruned_composites.len());
                tracing::info!("tick {}: {}", tick, msg);
                push_log(&shared_state, tick, &msg);
            }
        }

        // ── Throttle ──────────────────────────────────────────────────────
        if args.tick_floor_us > 0 {
            let elapsed = tick_start.elapsed();
            let floor   = Duration::from_micros(args.tick_floor_us);
            if elapsed < floor {
                std::thread::sleep(floor - elapsed);
            }
        }

        tick += 1;
    }

    // Final checkpoint
    let cp = AgentCheckpoint {
        tick,
        signal_values: bus.snapshot_values(),
        causal_tracer: causal.clone_partial(),
        pattern_index: pattern_index.clone(),
        symbol_ledger: symbol_ledger.clone(),
        self_model:    self_model.clone(),
        action_imbalance_history: Vec::new(),
    };
    store.save(&cp)?;
    ledger.flush()?;
    tracing::info!("gene exited cleanly at tick {}", tick);
    Ok(())
}

// ── Helpers ───────────────────────────────────────────────────────────────────

fn update_resource_signals(bus: &mut SignalBus, data_dir: &std::path::Path) {
    let mem_pressure  = read_mem_pressure_normalized();
    let cur_mem       = bus.get_value(SIG_MEMORY);
    bus.set_value(SIG_MEMORY, cur_mem * 0.9 + mem_pressure * 0.1);

    let disk_pressure = read_disk_pressure_normalized(data_dir);
    let cur_disk      = bus.get_value(SIG_DISK);
    bus.set_value(SIG_DISK, cur_disk * 0.9 + disk_pressure * 0.1);
}

fn read_mem_pressure_normalized() -> f64 {
    if let Ok(content) = std::fs::read_to_string("/proc/self/status") {
        for line in content.lines() {
            if line.starts_with("VmRSS:") {
                if let Some(kb_str) = line.split_whitespace().nth(1) {
                    if let Ok(kb) = kb_str.parse::<f64>() {
                        return (kb / (500.0 * 1024.0)).clamp(0.0, 1.0);
                    }
                }
            }
        }
    }
    0.0
}

fn read_disk_pressure_normalized(data_dir: &std::path::Path) -> f64 {
    let count = std::fs::read_dir(data_dir)
        .map(|e| e.count())
        .unwrap_or(0);
    (count as f64 / 1000.0).clamp(0.0, 1.0)
}

fn get_mtime(path: &std::path::Path) -> Option<std::time::SystemTime> {
    std::fs::metadata(path).ok().and_then(|m| m.modified().ok())
}

/// Shared state for IPC
#[derive(Default, Clone)]
pub struct SharedState {
    pub tick:          u64,
    pub imbalance:     f64,
    pub last_action:   Option<u32>,
    pub active_symbols: Vec<(String, f64)>,
    pub identity:      String,
    pub pattern_count: usize,
    pub symbol_count:  usize,
    pub confidence:    f64,
    pub action_count:  usize,
    pub pending_inject: Option<(SignalId, f64)>,
    pub pause_requested: bool,
    pub recent_expressions: Vec<serde_json::Value>,
    pub composite_count: usize,
    /// Activity log for TUI console (newest first, capped at 200)
    pub activity_log: Vec<String>,
}

impl CausalTracer {
    pub fn clone_partial(&self) -> Self {
        Self {
            stats: self.stats.clone(),
            action_imbalance_stats: self.action_imbalance_stats.clone(),
            observations: self.observations().iter().rev().take(1000).cloned().collect(),
            max_history: 10_000,
        }
    }
}

use std::io::IsTerminal;

// ── Activity log helpers ───────────────────────────────────────────────────────

fn tick_str(t: u64) -> String {
    if t >= 1_000_000 { format!("{:.2}M", t as f64 / 1_000_000.0) }
    else if t >= 1_000 { format!("{:.1}k", t as f64 / 1_000.0) }
    else { format!("{}", t) }
}

fn push_log(shared: &Arc<Mutex<SharedState>>, tick: u64, msg: &str) {
    if let Ok(mut st) = shared.try_lock() {
        st.activity_log.insert(0, format!("{:>6}  {}", tick_str(tick), msg));
        if st.activity_log.len() > 200 {
            st.activity_log.truncate(200);
        }
    }
}

fn op_label(op: &SystemOp) -> String {
    match op {
        SystemOp::GenAction        => "GenAction".to_string(),
        SystemOp::WritePrompt      => "WritePrompt".to_string(),
        SystemOp::ReadPrompt       => "ReadPrompt".to_string(),
        SystemOp::CargoBuild       => "CargoBuild".to_string(),
        SystemOp::ReloadActions    => "ReloadActions".to_string(),
        SystemOp::ApplyAndRestart  => "ApplyAndRestart".to_string(),
        SystemOp::ShellExec { .. } => "ShellExec".to_string(),
        SystemOp::Renice { niceness } => format!("Renice({})", niceness),
        SystemOp::SpawnStressor    => "SpawnStressor".to_string(),
        SystemOp::KillStressor     => "KillStressor".to_string(),
        SystemOp::DropCaches       => "DropCaches".to_string(),
        SystemOp::ReadFile { path } => {
            let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("?");
            format!("ReadFile({})", name)
        }
        SystemOp::GenSourcePatch { target } => {
            let t: &str = match target {
                SourceTarget::PatternExtractor   => "extractor",
                SourceTarget::RegulationSelector => "selector",
                SourceTarget::SignalBus          => "bus",
                SourceTarget::SelfModelModel     => "selfmodel",
                SourceTarget::Custom(p) =>
                    p.file_name().and_then(|n| n.to_str()).unwrap_or("custom"),
            };
            format!("GenPatch({})", t)
        }
    }
}
