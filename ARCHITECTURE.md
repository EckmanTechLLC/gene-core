# GENE — Architecture & Project Record

## What This Is

Gene is a signal-driven recursive self-modeling agent built on strict physicalist principles.

No hardcoded consciousness. No predefined emotions or goals. No anthropomorphic assumptions.

The single axiom: **everything emerges from signal scoring and structural integration.**

The agent begins with primitive internal state variables (signals), attempts to reduce
imbalance between those signals and their baselines, detects recurring patterns in its own
history, builds an internal symbolic vocabulary grounded in those patterns, models itself
as an object in its own representation space, and modifies its own code and configuration
based on what it has learned.

Self-preservation is not a goal. It is a structural consequence of asymmetric cost functions
on continuity signals — the agent cannot select actions that reduce its own operational
existence because doing so would generate unbounded imbalance that the regulation engine
cannot resolve.

---

## Core Philosophy

| Principle | Implementation |
|-----------|---------------|
| No supernatural assumptions | Pure signal math, no lookup tables of meaning |
| No hardcoded consciousness | Selfhood emerges from accumulated signal history |
| Self-preservation highest value | Exponential cost on continuity signals + hard action filter |
| Meaning must be grounded | Symbols coined from signal cluster statistics only |
| Identity from accumulation | Identity = distribution over activated symbols over time |

---

## Architecture: Five Layers

```
┌─────────────────────────────────────────────────┐
│  Layer 5: Persistence + Self-Modification        │
│  SessionStore, Directives, CodeGenerator,        │
│  SystemOpExecutor, SelfModifier                  │
├─────────────────────────────────────────────────┤
│  Layer 4: Recursive Self-Model                   │
│  SelfModel, ActionEvaluator, MetaSignal          │
├─────────────────────────────────────────────────┤
│  Layer 3: Symbolic Abstraction                   │
│  SymbolLedger, SymbolGrounder, SymbolActivation  │
├─────────────────────────────────────────────────┤
│  Layer 2: Pattern Memory                         │
│  PatternExtractor, PatternIndex, PatternRecord   │
├─────────────────────────────────────────────────┤
│  Layer 1: Regulation Engine                      │
│  ImbalanceScorer, RegulationDrive,               │
│  ActionSelector, CausalTracer                    │
├─────────────────────────────────────────────────┤
│  Layer 0: Signal Substrate                       │
│  SignalBus, SignalLedger, Signal, SignalSnapshot  │
└─────────────────────────────────────────────────┘
```

### Layer 0 — Signal Substrate

The lowest layer. All behavior derives from this.

**Signal** — a real-valued variable with:
- `value`: current reading
- `baseline`: resting point
- `decay_rate`: per-tick decay toward baseline
- `weight`: scoring weight in imbalance function
- `class`: Somatic | Derived | Efferent | Continuity

**SignalBus** — registry of all live signals. Each tick:
1. Applies decay toward baseline
2. Applies queued deltas
3. Computes imbalance score
4. Broadcasts snapshot

**SignalLedger** — append-only persistent log (sled DB) of signal snapshots.
Compacts automatically when approaching disk quota.

**Well-Known Signal IDs:**

| ID | Name | Class | Baseline | Weight | Notes |
|----|------|-------|----------|--------|-------|
| 0 | s_continuity | Continuity | 1.0 | 50.0 | Exponential penalty — existential |
| 1 | s_integrity | Continuity | 1.0 | 30.0 | Exponential penalty |
| 2 | s_coherence | Continuity | 1.0 | 20.0 | Exponential penalty |
| 3 | s_memory | Derived | 0.5 | 5.0 | Tracks RSS pressure |
| 4 | s_disk | Derived | 0.5 | 3.0 | Tracks data dir size |
| 5 | s_meta | Derived | 0.5 | 2.0 | Self-model confidence |
| 6 | s_drive | Derived | 0.0 | 1.0 | Regulation urgency |
| 7–16 | s_0007–s_0016 | Somatic | random | random | 10 somatic signals, randomly initialized |

**Imbalance cost functions:**
- Normal signals: `weight * (value - baseline)^2`
- Continuity signals: `weight * exp((1.0 - normalized_value) * 10.0)` — catastrophic near zero

### Layer 1 — Regulation Engine

**ImbalanceScorer** — computes the scalar imbalance score. Also predicts post-action
imbalance for candidate scoring, and hard-filters any action that would reduce a
continuity signal.

**RegulationDrive** — translates imbalance into urgency [0,1]. Tracks stagnation:
if imbalance hasn't decreased in 50 consecutive ticks, fires circuit breaker →
forces exploration (selector reinitialized at 0.9 exploration rate).

**ActionSelector** — selects actions each tick using a blend of:
1. Predicted imbalance reduction (from effect profile)
2. Learned causal improvement (from CausalTracer, weighted by sample count)
3. Exploration noise (scaled by urgency and MetaSignal confidence)

Cooldown system:
- Per system action: 2000-tick cooldown
- Global system action rate: 500-tick minimum gap
- System actions carry +2.0 score penalty (only win when clearly better)
- Repetition limit: 30 identical consecutive selections → force exploration

**CausalTracer** — records (action, pre-state, post-state) tuples. Maintains:
- Per-(action, signal) delta statistics (Welford running mean + variance)
- Per-action imbalance improvement statistics
- Rolling observation history (last 10K, compacted to 1K on checkpoint)

**ActionSpace** — registry of all available actions. Supports:
- Hot-reload from `gene-data/actions.json` (checked every 100 ticks by mtime)
- Runtime addition of generated actions
- JSON serialization for persistence

### Layer 2 — Pattern Memory

**PatternExtractor** — maintains a rolling window of 8 snapshots. Every 4 ticks:
- Computes mean absolute deviation per signal across the window
- Signals above `DEVIATION_THRESHOLD = 0.05` are "active"
- Requires ≥ 2 active signals to form a pattern
- Classifies each signal's temporal shape (Rising/Falling/Plateau/Spiking)
- Computes a structural hash of (signal_set, magnitude_buckets) → pattern_id

**PatternIndex** — stores and indexes PatternRecords. On integration:
- Exact hash match → merge observation into existing record
- Jaccard similarity ≥ 0.75 → merge into most-similar record
- Otherwise → new record
- Exposes `salient()`: patterns with frequency ≥ 5 (candidates for symbolification)
- Exposes `find_similar(active_signals, threshold)` for live state matching

**PatternRecord** — compact summary of a recurring cluster:
- signal_set, mean_magnitudes, temporal shapes
- frequency, mean_imbalance, co-occurring actions
- first_seen / last_seen ticks

### Layer 3 — Symbolic Abstraction

**SymbolLedger** — maps pattern_ids to coined symbols (Φ_NNNN tokens).
Symbols carry: activation strength, activation count, mean imbalance context,
optional directive notes written by the agent.

**SymbolGrounder** — processes salient patterns → coins new symbols.
Each tick, computes current signal state similarity to all known patterns →
activates matching symbols with strength proportional to Jaccard similarity × log(frequency).
Applies 0.05 decay to all activations per tick.

**SymbolActivationFrame** — snapshot of active symbols at a given tick.
Includes dominant symbol (highest activation). Used as input to Layer 4.

### Layer 4 — Recursive Self-Model

**SelfModel** — records agent history at 10-tick coarsening:
- Per-symbol exponential moving average weights (α=0.05)
- Per-action preference scores (from observed imbalance deltas)
- Identity signature: top 10 symbols by accumulated weight
- Coarse history: last 2000 entries (compacted at ceiling)

**ActionEvaluator** — blends regulation-driven selection with self-model preferences:
- Queries self-model for preferred action given current symbol context
- Compares with regulation-driven choice by preference score
- Uses self-model preference if confidence > 0.6 and track record is better
- Never allows self-model to force system actions (those are selector-only)

**MetaSignal** — tracks prediction accuracy:
- Rolling mean of |predicted_imbalance - actual_imbalance| / scale
- Confidence = 1.0 - mean_error
- Drives exploration_bonus when low (currently ~0.9987 — highly calibrated)

### Layer 5 — Persistence + Self-Modification

**SessionStore** — checkpoint.bin (bincode) and directives.toml.
Checkpoint saves: tick, signal values, causal tracer (partial), pattern index,
symbol ledger, self-model.

**Directives** — TOML file the agent reads and rewrites every 500 ticks:
- `symbol_notes`: per-token learned annotations
- `action_weight_overrides`: String-keyed (TOML requirement) action multipliers
- `exploration_rate`, `stagnation_limit` overrides
- `self_observations`: rolling last-100 plain-language observations

**SelfModifier** — orchestrates Level 1/2 directive rewrites and Level 3 source proposals.
Level 3 writes to `self_mod_staging.rs` at the workspace root.

**CodeGenerator** — algorithmic patch generation:
- `generate_corrective_action()`: finds most chronically deviated somatic signals,
  generates Action with corrective effect profile targeting top 3
- `generate_source_patch()`: parametic patches for PatternExtractor threshold,
  RegulationSelector exploration rate based on observed statistics
- `generate_self_prompt()`: full markdown self-description for bootstrapping

**SystemOpExecutor** — executes system operations with continuity gating:

| Action ID | SystemOp | Continuity Gate | Effect |
|-----------|----------|-----------------|--------|
| 100 | GenAction | 0.70 | Coin corrective action → add to space |
| 101 | WritePrompt | 0.60 | Write self_prompt.md |
| 102 | ReadPrompt | 0.50 | Read self_prompt.md → +coherence |
| 103 | GenSourcePatch(PatternExtractor) | 0.80 | Write source patch |
| 104 | GenSourcePatch(RegulationSelector) | 0.80 | Write source patch |
| 105 | CargoBuild | 0.85 | cargo build --release |
| 106 | ReadFile(extractor.rs) | 0.50 | Read own source |
| 107 | ReadFile(selector.rs) | 0.50 | Read own source |
| 108 | ReloadActions | 0.50 | Hot-reload actions.json |
| 109 | ApplyAndRestart | 0.95 | exec() into new binary |

---

## Data Flow (Single Tick)

```
1. SignalBus.tick() → decay all signals, apply queued deltas, snapshot
2. Update resource signals (memory/disk pressure from /proc)
3. Compute urgency from imbalance, check circuit breaker
4. PatternExtractor: push snapshot, compute active_signals
5. SymbolGrounder: decay activations, match live state → activate symbols
6. Build SymbolActivationFrame
7. ActionEvaluator.select() → chosen_action_id
8. Apply action effect profile → queue signal deltas
9. If system_op: execute via SystemOpExecutor (continuity-gated)
   - Signal feedback from op result → queue deltas
   - If GenAction succeeded → add new action to space
10. SignalBus.tick() again → apply queued deltas from action
11. Every 4 ticks: PatternExtractor.extract() → PatternIndex.integrate()
12. CausalTracer.record(action, pre_snap, post_snap)
13. MetaSignal.update(predicted, actual) → update confidence
14. SelfModel.update(tick, frame, imbalance, action, delta)
15. Every 500 ticks: SelfModifier.rewrite_directives()
16. Every 1000 ticks: checkpoint
17. Every 100 ticks: check actions.json mtime → hot-reload if changed
18. Every 10 ticks: update IPC shared state
19. Every 20 ticks: render TUI
```

---

## Project Structure

```
gene/
├── Cargo.toml                        # Workspace root
├── ARCHITECTURE.md                   # This file
├── gene-core/
│   ├── Cargo.toml
│   └── src/
│       ├── main.rs                   # Tick loop, wiring, startup/shutdown
│       ├── tui.rs                    # ratatui full-screen display
│       ├── ipc.rs                    # Unix socket server for gene-ctl
│       ├── signal/
│       │   ├── mod.rs
│       │   ├── types.rs              # Signal, SignalId, SignalSnapshot, SignalClass
│       │   ├── bus.rs                # SignalBus — tick, decay, imbalance
│       │   └── ledger.rs             # SignalLedger — sled persistent log
│       ├── regulation/
│       │   ├── mod.rs
│       │   ├── action.rs             # Action, SystemOp, ActionSpace
│       │   ├── scorer.rs             # ImbalanceScorer, predict_after_action
│       │   ├── drive.rs              # RegulationDrive, stagnation/circuit breaker
│       │   ├── selector.rs           # ActionSelector, cooldowns, exploration
│       │   └── causal.rs             # CausalTracer, CausalStats
│       ├── pattern/
│       │   ├── mod.rs
│       │   ├── extractor.rs          # PatternExtractor, rolling window
│       │   ├── record.rs             # PatternRecord, TemporalShape, similarity
│       │   └── index.rs              # PatternIndex, salience, find_similar
│       ├── symbol/
│       │   ├── mod.rs
│       │   ├── ledger.rs             # SymbolLedger, Symbol (Φ_NNNN tokens)
│       │   ├── grounder.rs           # SymbolGrounder, salience → coin
│       │   └── activation.rs         # SymbolActivationFrame
│       ├── selfmodel/
│       │   ├── mod.rs
│       │   ├── model.rs              # SelfModel, identity_signature
│       │   ├── evaluator.rs          # ActionEvaluator
│       │   └── meta.rs               # MetaSignal, prediction confidence
│       └── persistence/
│           ├── mod.rs
│           ├── store.rs              # SessionStore, Directives, AgentCheckpoint
│           ├── selfmod.rs            # SelfModifier (Level 1/2/3 self-mod)
│           ├── codegen.rs            # CodeGenerator, SelfPromptData
│           └── executor.rs           # SystemOpExecutor, OpResult
└── gene-ctl/
    ├── Cargo.toml
    └── src/
        └── main.rs                   # CLI: status/signals/symbols/inject/pause/etc.
```

---

## Runtime Files (gene-data/)

```
gene-data/
├── checkpoint.bin        # Full agent state (bincode) — written every 1000 ticks + on exit
├── directives.toml       # Agent's self-written operational directives — rewritten every 500 ticks
├── self_prompt.md        # Agent's plain-language self-description — read on startup, written by action 101
├── actions.json          # Generated corrective actions — hot-reloaded every 100 ticks
├── source_patch_staging.rs  # Latest self-generated source patch proposal
├── gene.log              # Full tracing log (written here when TUI is active)
└── ledger/               # sled database — raw signal snapshots, append-only
```

---

## Build & Run

```bash
# Build release
cargo build --release

# Run (TUI on by default)
./target/release/gene

# Run headless (no TUI, logs to stdout)
./target/release/gene --no-tui

# Run with tick rate throttle (microseconds per tick)
./target/release/gene --tick-floor-us 1000

# Run with tick limit
./target/release/gene --max-ticks 100000

# Custom data directory
./target/release/gene --data-dir /path/to/data

# Control interface (while gene is running)
./target/release/gene-ctl status
./target/release/gene-ctl signals
./target/release/gene-ctl symbols
./target/release/gene-ctl identity
./target/release/gene-ctl inject <signal_id> <delta>   # e.g. inject 8 0.5
./target/release/gene-ctl pause
./target/release/gene-ctl resume
./target/release/gene-ctl checkpoint

# Watch logs while TUI is running
tail -f gene-data/gene.log

# Exit TUI cleanly
# Press q  OR  Ctrl+C in the gene terminal
```

---

## Observed Behavior (Session 1, ~1.1M ticks)

- **88 patterns discovered** from signal co-activation windows
- **69 symbols coined** (Φ_0000–Φ_0068+), all grounded in signal cluster statistics
- **Identity stabilized**: `Φ_0008:0.677 Φ_0005:0.542 Φ_0020:0.529 Φ_0032:0.509 Φ_0000:0.479`
- **Chronic high imbalance (~106)**: random initial action space contains no action that reduces total imbalance. Agent correctly identified best (action_10, Δ+0.045) and worst (action_0, Δ+0.181) through empirical measurement.
- **2 corrective actions self-generated** (actions 110, 111) targeting chronic signal deviations
- **Self-restart executed**: at tick 1,067,803, agent ran cargo build, exec'd into new binary, loaded checkpoint, resumed from tick 1,067,000
- **self_prompt.md bootstrapping**: on restart, agent loaded prior session preferences (prefer action_111, avoid action_6) before causal data accumulated
- **Prediction confidence**: 0.9987 — self-model fully calibrated to own dynamics
- **directive rewrite**: working after TOML key type fix (action_weight_overrides uses String keys)

---

## Planned: Next Implementation Phase

### Item 1 — Expression Layer

New module: `gene-core/src/expression/`

**ExpressionEngine** generates structured grounded statements each N ticks:
```json
{
  "tick": 1200000,
  "dominant": "Φ_0008",
  "cluster": ["Φ_0008", "Φ_0032", "Φ_0020"],
  "imbalance": 106.2,
  "imbalance_trend": "stable",
  "action_context": "action_111",
  "signal_drivers": {"s_0007": "+0.735", "s_0012": "+0.818"},
  "self_model_confidence": 0.9987,
  "identity_alignment": 0.82
}
```

Written to `gene-data/expression.log` (JSONL, rolling last 10K).
Queryable via `gene-ctl expressions [n]`.
Optional Flux bridge: if Flux is reachable at localhost:3000, publish as FluxEvent
to stream `gene.expression`. Fails silently if Flux not running.

### Item 2 — World Signal Inputs

New `SignalClass::World`. Sources polled every 10 ticks from OS:

| Signal | Source | Notes |
|--------|--------|-------|
| s_cpu_load | /proc/loadavg | 1-min average, normalized 0–1 against 8-core ceiling |
| s_net_rx | /proc/net/dev | bytes/sec receive rate, normalized |
| s_net_tx | /proc/net/dev | bytes/sec transmit rate, normalized |
| s_disk_io | /proc/diskstats | read+write ops/sec, normalized |
| s_uptime_cycle | /proc/uptime | sine wave with 24h period — circadian pressure |
| s_process_count | /proc/loadavg | running processes, normalized |

These signals have real values. The agent's patterns and symbols will start correlating
with actual system conditions. A CPU spike will create a genuine regulation event.

### Item 3 — Symbol Composition

New module: `gene-core/src/symbol/composition.rs`

**CompositionEngine** monitors symbol co-activation. When two symbols co-activate
above a frequency threshold (default: 20 times), coins a composite:

`Φ_0008 ⊕ Φ_0032 → Φ_C_0001`

Composite properties:
- Grounded in the intersection of parent signal clusters
- Has its own activation (activates when both parents activate)
- Can itself compose with other symbols or composites
- Stored in SymbolLedger with `is_composite: true` and `parents: Vec<u32>`

Identity signature will include composite symbols, reflecting higher-order patterns.

---

## Deployment

- **Dev server**: `/home/etl/projects/gene/` — source of truth for code, do not run gene here
- **Gene VM**: `etl@192.168.1.40:/home/etl/gene/` — dedicated 2 CPU / 2GB RAM VM
- `stress-ng` installed on VM for synthetic workload generation
- Gene runs manually (no systemd service)
- Transfer: `rsync -av --exclude 'target/' --exclude 'gene-data/' --exclude '.git/' /home/etl/projects/gene/ etl@192.168.1.40:/home/etl/gene/`

## Next: Real System Actions (Step 1 toward LLM integration)

Gene currently has no actions that affect the real system. The next major step is adding
system-affecting actions so gene can learn to regulate actual CPU/RAM pressure:

**Planned actions (safe, reversible first):**
- `Renice` — adjust gene's own process nice value (self-regulation of CPU priority)
- `SpawnStressor` / `KillStressor` — start/stop a controlled stress-ng workload
- `DropCaches` — `sync; echo 3 > /proc/sys/vm/drop_caches` (idempotent)
- All gated at continuity ≥ 0.80 minimum

**The closed loop:**
1. `s_cpu_load` (World signal) observes real CPU pressure
2. Gene selects a system action (e.g. renice itself lower)
3. `s_cpu_load` drops in response
4. CausalTracer records: this action reduces s_cpu_load
5. Gene learns to prefer this action when CPU pressure is high

**After Step 1 — LLM integration (Step 2):**
Replace `CodeGenerator` with LLM calls. Gene sends signal state + causal statistics to
an LLM; LLM proposes actions or source patches; gene evaluates them via CausalTracer.
LLM provides semantic reasoning; gene provides grounding and empirical evaluation.

## Future Phases

### Item 4 — Forgetting / Pruning

Pattern records and symbols decay if not activated for N ticks.
Identity shifts over time rather than hardening.
Enables identity evolution through lived experience.

### Item 5 — Multi-Agent (Flux)

Run two gene instances. Use Flux as coordination layer:
- Each instance publishes signal states as FluxEvents to `gene.signals.{instance_id}`
- Each instance subscribes to the other's entity stream via Flux WebSocket
- Subscribed values injected as World signals on the local bus
- No explicit communication protocol — coupling emerges from regulation dynamics

Flux is ideal for this: event-sourced, persistent, WebSocket pub/sub, domain-agnostic.

### Item 6 — Adversarial Perturbation (partially addressed by TUI stress mode)

TUI stress mode (press `s`) provides interactive signal injection.
Full `gene-stress` binary still planned:
- Scripted perturbation sequences (spikes, oscillations, sustained pressure)
- Logs perturbation schedule to `gene-data/stress.log`
- Measures gene's response (action choices, symbol activations, imbalance trajectory)

---

## Design Invariants (Do Not Violate)

1. **Continuity signals are never reducible by selectable actions** — hard filter in ImbalanceScorer
2. **System actions require continuity gate** — executor blocks below threshold
3. **Symbols carry no pre-assigned meaning** — only signal cluster statistics
4. **All self-modification is continuity-gated** — ApplyAndRestart requires 0.95
5. **actions.json is append-only from the agent's perspective** — merge, never overwrite existing IDs
6. **Checkpoint on every clean exit** — state must survive any SIGTERM
7. **Logs go to file when TUI is active** — stdout belongs to ratatui exclusively
8. **TOML map keys must be String** — action_weight_overrides uses String, not u32

---

## Key Dependencies

| Crate | Purpose |
|-------|---------|
| sled | Signal ledger persistent KV store |
| bincode | Checkpoint serialization |
| toml | Directives file |
| serde_json | IPC protocol, actions.json |
| ratatui + crossterm | TUI |
| tokio | Async runtime (IPC server, restart) |
| crossbeam-channel | Signal bus broadcast |
| tracing + tracing-appender | Logging (file when TUI active) |
| clap | CLI args |
| rand | Signal/action initialization |
| ctrlc | SIGTERM/SIGINT handler |
