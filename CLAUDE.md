# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Deployment

- **Dev server**: `/home/etl/projects/gene/` — source of truth, do not run gene here
- **Gene VM**: `etl@192.168.50.40:/home/etl/gene/` — dedicated VM, gene runs here
- **Transfer**: `rsync -av --exclude 'target/' --exclude 'gene-data/' --exclude '.git/' /home/etl/projects/gene/ etl@192.168.50.40:/home/etl/gene/`
- **VM build**: `ssh etl@192.168.50.40 "source ~/.cargo/env && cd /home/etl/gene && cargo build --release"`
- **Note**: After wiping gene-data/ or target/, run the VM build before starting gene
- Gene runs manually (no systemd service) — start in a terminal with TUI
- Synthetic workload available via `stress-ng` (installed on VM)

## Build & Run

```bash
# Build
cargo build --release

# Run with TUI (default)
./target/release/gene

# Run headless (logs to stdout)
./target/release/gene --tui false

# Run with options
./target/release/gene --data-dir ./gene-data --max-ticks 10000 --tick-floor-us 100

# Control CLI (while gene is running)
./target/release/gene-ctl status
./target/release/gene-ctl signals
./target/release/gene-ctl symbols
./target/release/gene-ctl identity
./target/release/gene-ctl inject <signal_id> <delta>   # e.g. inject 8 0.5
./target/release/gene-ctl pause
./target/release/gene-ctl resume
./target/release/gene-ctl checkpoint

# Watch logs (when TUI active, stdout belongs to ratatui)
tail -f gene-data/gene.log

# Stop gene
# Press q or Ctrl+C in the gene terminal. Use pkill gene if unresponsive.
```

There are no automated tests. Build verification: `cargo build --release` (or `cargo check` for faster feedback).

## Architecture

Gene is a **5-layer signal-driven agent**. Each layer feeds the next; no layer has hardcoded goals or semantics.

```
Layer 0: Signal Substrate     — SignalBus, decay, imbalance cost functions
Layer 1: Regulation Engine    — ImbalanceScorer, RegulationDrive, ActionSelector, CausalTracer
Layer 2: Pattern Memory       — PatternExtractor (rolling window), PatternIndex
Layer 3: Symbolic Abstraction — SymbolGrounder, SymbolLedger (Φ_NNNN tokens)
Layer 4: Self-Model           — SelfModel, ActionEvaluator, MetaSignal
Layer 5: Persistence          — SessionStore, SelfModifier, CodeGenerator, SystemOpExecutor
```

The **tick loop** in `gene-core/src/main.rs` is the single execution path. Per tick (in order):
1. `bus.tick()` — apply decay, emit snapshot
2. Append to sled ledger, push to pattern extractor
3. Update resource signals (memory/disk pressure from /proc)
4. Compute urgency via `RegulationDrive`
5. Activate symbols via `SymbolGrounder`
6. Select action via `ActionEvaluator` (wraps `ActionSelector` + self-model bias)
7. Apply action effect profile to bus (queue deltas)
8. Execute system op if action has one (gated by `s_continuity` value ≥ gate threshold)
9. Second `bus.tick()` to apply queued deltas
10. Extract patterns every 4 ticks
11. Record causal stats, update `MetaSignal`, update `SelfModel`
12. Rewrite directives every 500 ticks (`SelfModifier`)
13. Checkpoint every 1000 ticks (bincode to `gene-data/checkpoint.bin`)
14. Update IPC shared state every 10 ticks
15. Render TUI every 20 ticks

### Signal IDs (well-known, never change)

| ID | Name | Class | Notes |
|----|------|-------|-------|
| 0 | s_continuity | Continuity | Weight 50, exp penalty — self-preservation core |
| 1 | s_integrity | Continuity | Weight 30, exp penalty |
| 2 | s_coherence | Continuity | Weight 20, exp penalty |
| 3 | s_memory | Derived | /proc/self/status VmRSS |
| 4 | s_disk | Derived | data_dir file count |
| 5 | s_meta | Derived | prediction confidence from MetaSignal |
| 6 | s_drive | Derived | urgency from RegulationDrive |
| 7–16 | s_0007–s_0016 | Somatic | Random init at startup |
| 17 | s_cpu_load | World | /proc/loadavg, normalized 0–1 (ceiling 8 cores) |
| 18 | s_net_rx | World | /proc/net/dev bytes received, delta-normalized |
| 19 | s_net_tx | World | /proc/net/dev bytes sent, delta-normalized |
| 20 | s_disk_io | World | /proc/diskstats sectors, delta-normalized |
| 21 | s_uptime_cycle | World | sine wave over 24h period (circadian) |
| 22 | s_proc_count | World | /proc/loadavg total processes, normalized |
| 23 | s_swap_used | World | /proc/meminfo SwapUsed/SwapTotal |
| 24 | s_iowait | World | /proc/stat CPU iowait% delta |
| 25 | s_ctx_switches | World | /proc/self/status voluntary+nonvoluntary ctx switches delta |
| 26 | s_tick_rate | World | gene's own ticks/sec, log2-normalized (ceiling log2(32768)≈15) |
| 27 | s_mem_available | World | 1 - (MemAvailable/MemTotal) from /proc/meminfo |
| 28 | s_quake_rate | World | Flux earthquakes: events/hr in 1-hr window / 20 — **weight=0 (observation-only)** |
| 29 | s_quake_magnitude | World | Flux earthquakes: max magnitude in 1-hr window / 9 — **weight=0 (observation-only)** |
| 30 | s_quake_depth | World | Flux earthquakes: 1 - (mean depth_km / 700), shallow=high — **weight=0 (observation-only)** |
| 31 | s_quake_sig | World | Flux earthquakes: max USGS significance in 1-hr window / 2000 — **weight=0 (observation-only)** |

Continuity signals use **exponential** cost: `weight * exp((1 - normalized) * 10)`.
All other signals use **quadratic** cost: `weight * (value - baseline)²`.
This asymmetry is the structural source of self-preservation — no goal is hardcoded.

### System Actions (IDs 100+)

All require `s_continuity ≥ gate` to execute (checked in `SystemOpExecutor`).
Cooldowns: 2000 ticks per-action, 500 ticks global system action rate limit.
Score penalty: +2.0 applied in `ActionSelector` so system actions only win when clearly better.

| ID | Op | Gate |
|----|-----|------|
| 100 | GenAction — coin corrective action targeting chronic deviations | 0.70 |
| 101 | WritePrompt — write self_prompt.md | 0.60 |
| 102 | ReadPrompt — read self_prompt.md | 0.50 |
| 103 | GenSourcePatch(PatternExtractor) | 0.80 |
| 104 | GenSourcePatch(RegulationSelector) | 0.80 |
| 105 | CargoBuild — cargo build --release | 0.85 |
| 106 | ReadFile(extractor.rs) | 0.50 |
| 107 | ReadFile(selector.rs) | 0.50 |
| 108 | ReloadActions — hot-reload actions.json | 0.50 |
| 109 | ApplyAndRestart — exec() into new binary | 0.95 |

Self-generated actions (ID ≥ 110) land in `gene-data/actions.json` and are hot-reloaded every 100 ticks.

### Key Modules

- `gene-core/src/signal/bus.rs` — `SignalBus`: register, tick, decay, queue_delta, inject, snapshot
- `gene-core/src/signal/world.rs` — `WorldSignalPoller`: polls /proc every 10 ticks for 6 OS signals
- `gene-core/src/regulation/action.rs` — `Action`, `ActionSpace`, `SystemOp` enum definitions
- `gene-core/src/regulation/selector.rs` — `ActionSelector`: blends predicted vs. causal-learned improvement, enforces cooldowns
- `gene-core/src/regulation/causal.rs` — `CausalTracer`: Welford online stats per (action, signal) pair
- `gene-core/src/pattern/extractor.rs` — sliding window (size 8), baseline-deviation detection (not series MAD)
- `gene-core/src/symbol/ledger.rs` — `SymbolLedger`: Φ_NNNN token registry, activation EMA; composites Φ_C_NNNN
- `gene-core/src/symbol/composition.rs` — `CompositionEngine`: coins composites when two symbols co-activate ≥20 times
- `gene-core/src/selfmodel/model.rs` — `SelfModel`: action preference EMA, identity signature (top-10 symbols), coarsened history
- `gene-core/src/persistence/executor.rs` — `SystemOpExecutor`: continuity gate check + dispatch for all system ops
- `gene-core/src/persistence/codegen.rs` — `CodeGenerator`: generates corrective action JSON and self_prompt.md
- `gene-core/src/expression/engine.rs` — `ExpressionEngine`: JSONL output every 100 ticks to gene-data/expression.log
- `gene-ctl/src/main.rs` — Unix socket client sending JSON commands to `/tmp/gene.sock`

### Runtime Data (`gene-data/`)

| File | Purpose |
|------|---------|
| `checkpoint.bin` | bincode-serialized `AgentCheckpoint` (tick, signals, causal, patterns, symbols, self_model) |
| `directives.toml` | self-modifiable config (symbol_notes, action_weight_overrides, exploration_rate) |
| `actions.json` | self-generated actions, merged (never overwritten) into ActionSpace on hot-reload |
| `self_prompt.md` | agent's plain-language self-description, read on restart to bootstrap action preferences |
| `expression.log` | JSONL rolling log of structured expressions (last 10K), queryable via gene-ctl expressions |
| `source_patch_staging.rs` | latest self-generated source patch proposal |
| `gene.log` | tracing output when TUI active (stdout belongs to ratatui) |
| `ledger/` | sled database — raw SignalSnapshot stream |

### TUI Modes

Press `s` for stress mode (inject signals), `w` for weight edit mode.

**Stress mode** (`s`):

| Key | Action |
|-----|--------|
| `↑`/`↓` or `j`/`k` | Move signal selection |
| `+`/`-` | Nudge selected signal ±0.3 |
| `1` | Spike +1.0 on selected |
| `2` | Drop -1.0 on selected |
| `3` | Normalize — push toward baseline |
| `4` | Pressure — +0.5 on all somatic signals |
| `5` | Oscillate — 20 pulses ±0.5 on selected |
| `s`/`Esc` | Exit stress mode |

**Weight edit mode** (`w`): Adjust imbalance contribution weights live. Continuity signals (IDs 0-2) are protected.

| Key | Action |
|-----|--------|
| `↑`/`↓` or `j`/`k` | Move signal selection |
| `[` | Weight −0.5 (min 0.0) |
| `]` | Weight +0.5 (max 20.0) |
| `w`/`Esc` | Exit weight mode |

## Design Invariants

These must never be violated:

1. **Continuity signals are never reducible by selectable actions** — `ImbalanceScorer::harms_continuity()` is a hard filter applied before any action is considered.
2. **System actions require continuity gate** — `SystemOpExecutor` checks `s_continuity ≥ action.continuity_gate` before executing.
3. **Symbols carry no pre-assigned meaning** — tokens are Φ_NNNN; semantics emerge only from signal cluster statistics.
4. **actions.json is append/merge only** — `ActionSpace::merge_from_json()` skips existing IDs; never overwrite.
5. **TOML map keys must be String** — `action_weight_overrides: HashMap<String, f64>` (not u32); TOML doesn't support integer keys.
6. **Logs go to file when TUI is active** — `tracing-appender` to `gene.log`; stdout belongs to ratatui.
7. **Checkpoint on every clean exit** — final checkpoint written in the shutdown path unconditionally.
8. **Never force system actions through the self-model path** — `ActionEvaluator` defers to `reg_choice` if `sm_id` is a system action.
