# financial-gene — Planning Document

## What It Is

A second binary built on gene-core's architecture, adapted for financial market signal processing.
Gene is **not** the trading system — it is a regime detector and signal generator.
A separate paper trading system acts on gene's outputs and feeds results back.

Gene's value in this domain is the emergent symbolic vocabulary it builds over market data,
and its self-calibrating perception of market regimes via meta-actions.

---

## Project Structure

Separate repository: `financial-gene/`
Reuses `gene-core` as a library dependency (all five layers: signal, regulation, pattern, symbol, selfmodel, persistence).
Has its own `main.rs` wiring, market signal pollers, and action space.
Does **not** share the OS gene binary, gene-data/, or checkpoint.

---

## Two-System Architecture

```
financial-gene   ←→   Flux   ←→   paper-trading-system
```

**financial-gene:**
- Ingests market data from Flux (OHLCV, vol, sentiment, macro)
- Tracks portfolio state signals pushed back from trading system
- Publishes current signal state, symbol activations, imbalance to Flux

**paper-trading-system (separate, out of scope here):**
- Subscribes to financial-gene state via Flux
- Executes paper trades based on gene's regime signals
- Pushes position state, PnL, drawdown back to financial-gene via Flux

Coordination layer: Flux WebSocket pub/sub (same pattern as `signal/flux.rs`).

---

## Signal Taxonomy

### Observed / Observation-Only (weight=0)
Feed the pattern/symbol machinery. Gene cannot affect these.

| Signal | Source | Notes |
|--------|--------|-------|
| s_price_* | Flux OHLCV | Per-asset close, normalized |
| s_volume_* | Flux OHLCV | Relative volume normalized |
| s_volatility | Derived from OHLCV | Realized vol, normalized |
| s_momentum | Derived from OHLCV | Rate of change, normalized |
| s_sentiment | News/NLP feed | Normalized 0–1 |
| s_vix | Macro feed | Normalized |
| s_spread | Order book | Bid/ask spread normalized |

### Controlled (weight > 0)
Gene tracks these in the imbalance function. Trading system pushes updates.

| Signal | Source | Notes |
|--------|--------|-------|
| s_pnl_deviation | Trading system | PnL delta from target, normalized |
| s_drawdown | Trading system | Current drawdown / max drawdown limit |
| s_position_concentration | Trading system | Largest single position / portfolio |
| s_cash_ratio | Trading system | Cash as fraction of portfolio |

### Continuity Signals (IDs 0–2)
Map to capital preservation constraints — same exponential cost structure as OS gene.

| Signal | Semantic Meaning |
|--------|-----------------|
| s_continuity | Account solvency — never margin call, never total drawdown > threshold |
| s_integrity | Risk limit compliance — position concentration, exposure bounds |
| s_coherence | Signal integrity — not acting on stale or corrupt market data |

---

## Action Space

Two tiers. No direct market actions — gene does not execute trades.

### Tier 1 — Perception Meta-Actions

Actions that modify how gene tracks signals. Feedback loop: better calibration →
better symbol activation → better portfolio signals from trading system.

- `AdjustDecay(signal_id, delta)` — track a signal faster/slower
- `AdjustBaseline(signal_id, delta)` — recalibrate "normal" for a signal
- `AdjustWeight(signal_id, delta)` — change imbalance contribution
- `CoinDerivedSignal(op, signal_a, signal_b)` — register a ratio/difference/product signal on the bus

All continuity-gated. Implemented as new `SystemOp` variants in `persistence/executor.rs`.
`SignalBus` needs `set_decay_rate()` and `register_derived()` additions.

### Tier 2 — Output Signals (read by trading system)

Not actions — these are what the trading system subscribes to via Flux:
- Current symbol activations and strengths
- Active pattern IDs and historical co-occurring action correlations
- Per-signal deviation magnitudes and directions
- Imbalance trajectory (rising/stable/falling)
- Identity signature (dominant regime symbols)

---

## What gene-core Reuses Unchanged

- `signal/bus.rs` — SignalBus, decay, imbalance, snapshot
- `regulation/` — ActionSelector, CausalTracer, RegulationDrive, ImbalanceScorer
- `pattern/` — PatternExtractor, PatternIndex, PatternRecord
- `symbol/` — SymbolLedger, SymbolGrounder, CompositionEngine
- `selfmodel/` — SelfModel, ActionEvaluator, MetaSignal
- `persistence/` — SessionStore, Directives, SelfModifier, checkpoint

---

## What Is New in financial-gene

- `main.rs` — financial signal IDs, action registration, Flux wiring
- `signal/market.rs` — MarketSignalPoller (reads OHLCV/sentiment from Flux)
- `signal/portfolio.rs` — PortfolioSignalPoller (reads positions/PnL from Flux)
- `signal/bus.rs` additions — `set_decay_rate()`, `register_derived()`
- `persistence/executor.rs` additions — `AdjustDecay`, `AdjustBaseline`, `CoinDerivedSignal` ops
- Flux publisher — writes gene's state snapshot to Flux every N ticks

---

## Key Design Decisions

1. **Gene is a signal generator, not a trading system.** It has no exchange connection, no order API. The trading system consumes gene's state.

2. **Market data is observation-only (weight=0).** Gene cannot reduce imbalance by changing BTC price. Portfolio state signals (drawdown, PnL deviation) are in the imbalance function because the trading system can affect them.

3. **Self-calibration is the action space.** Since external signals are uncontrollable, gene's agency is in adjusting its own perception — decay rates, baselines, derived signal coinage. This is the financial analog of OS gene's system actions.

4. **Derived signal coinage is emergent feature engineering.** Analogous to `GenAction` (action 100) coining corrective actions, a `GenSignal` op coins derived signals from observed co-activation statistics. Not prescribed — emerges from the pattern layer.

5. **Continuity signals map to capital preservation constraints.** The exponential cost structure is appropriate — partial loss is painful, total loss is catastrophic. The structural invariant holds across domains.

6. **Flux is the coordination bus.** Same pattern as `signal/flux.rs`. financial-gene subscribes to market data and publishes state; trading system subscribes to state and publishes portfolio updates.

7. **Do not abstract gene-core yet.** Build financial-gene as a second concrete binary first. Extract `GeneConfig`, `SignalPoller` trait, and `GeneEngine::new(config)` only after both domain instances are running and the real variation points are known.

---

## Self-Modification Scope

OS gene's self-modification (GenSourcePatch, CargoBuild, ApplyAndRestart) modifies its own source and binary. In financial-gene, self-modification operates primarily at the signal layer (meta-actions above) rather than source level. The source-level ops can remain available but are lower priority — signal calibration is the more meaningful form of self-adaptation in this domain.

---

## Build Path

1. Create `financial-gene/` repository, add `gene-core` as path or crate dependency
2. Wire `main.rs` with financial signal IDs and Flux pollers
3. Add `set_decay_rate()` to `SignalBus`
4. Add perception meta-actions to `SystemOpExecutor`
5. Implement Flux publisher for gene state output
6. Run against Flux market data in observation mode (all portfolio signals mocked/static)
7. Introduce live paper trading system once gene's symbol vocabulary has had time to develop

---

## References

- `gene-core/src/signal/flux.rs` — Flux WS subscription pattern to follow for market/portfolio pollers
- `gene-core/src/signal/world.rs` — WorldSignalPoller pattern for periodic polling
- `gene-core/src/signal/bus.rs` — SignalBus, `register_with_id()`, `set_weight()`
- `gene-core/src/persistence/executor.rs` — SystemOpExecutor, SystemOp enum, continuity gating
- `gene-core/src/regulation/action.rs` — Action, effect profiles, ActionSpace
- `ARCHITECTURE.md` — full layer descriptions and data flow
- `CLAUDE.md` — signal ID table, design invariants, deployment pattern
