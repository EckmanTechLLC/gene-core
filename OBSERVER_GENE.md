# observer-gene — Planning Document

## What It Is

A third binary built on gene-core, configured purely for world observation.

No controlled signals. No trading system. No portfolio state. No external actuation.

Gene's only goal is self-preservation (continuity signals, IDs 0–2). Everything else is
observation-only (weight=0). The regulation engine still runs — gene still selects actions —
but those actions are purely self-maintenance: calibrating its own perception, coining derived
signals, adjusting decay rates.

The output is the symbol ledger: a vocabulary of cross-domain co-activation patterns that
emerged from observing many different kinds of empirical data simultaneously. Flux is both
the data source and the publication target — gene reads from Flux feeds and publishes its
state back to Flux for downstream consumers.

---

## Core Principle: Empirical Feeds Only

Gene's signal bus requires normalized scalar values. This constrains what can feed it —
but not what *domain* it comes from. Flux can carry anything; the constraint is on the
form of the data, not the subject matter.

**Include:**
- Direct measurements: price, temperature, speed, count, pressure, frequency, rate
- Derived transforms of measurements: EMA, rolling stddev, delta, z-score
- Pre-processed scores: if upstream processing has already reduced text/events to a
  normalized scalar (sentiment, anomaly score, severity index), that scalar qualifies

**Exclude:**
- Raw text, headlines, article content — not a scalar
- Any value that requires domain knowledge to interpret before it can be normalized
- Composite indices that encode assumptions or strategy

The rule: **if it's a number that normalizes cleanly, it belongs on the bus.** If it
requires interpretation to become a number, that interpretation happens upstream in Flux
before gene sees it.

---

## Entity Aggregation

Many Flux feeds carry entity-level data — individual vessels, individual flights, individual
trades. Gene does not receive entity-level data. The `flux_multi.rs` subscriber aggregates
before registering signals on the bus.

**General aggregation strategies:**
- **Count** — total active entities in a region or globally
- **Density** — entities per unit area in a defined geographic zone
- **Statistical summary** — mean, stddev, percentile of a property across the entity set
- **Rate of change** — delta of any aggregate over time
- **Threshold crossings** — fraction of entities above/below a value

This applies to any entity-heavy feed regardless of domain. The goal is to reduce
thousands of entities to dozens of meaningful scalar signals. The aggregation zones and
thresholds are configuration, not code — defined per feed in the startup config.

---

## Current Flux Feeds

| Domain | Raw data | Aggregation approach |
|--------|----------|---------------------|
| Weather | Temperature, pressure, humidity, wind, precipitation | Per-region aggregates, global averages |
| Crypto | OHLCV per asset | Per-asset normalized price/volume, derived indicators |
| Stocks | OHLCV per symbol/index | Per-index normalized, derived indicators |
| Airplanes | Per-flight position, altitude, speed | Regional density, global count, fleet avg speed |
| Ships | Per-vessel position, speed, heading | Regional density by key zone, global count, avg speed |

Flux can carry anything. The table above reflects current feeds — not a ceiling.

---

## Signal Taxonomy

All signals are `SignalClass::World`, weight=0. No signal contributes to imbalance.

### Raw Empirical
Normalized aggregate measurements. Naming convention: `s_{domain}_{property}_{region}`

Examples across current feeds:

| Signal pattern | Source | Notes |
|----------------|--------|-------|
| s_weather_temp_* | Flux weather | Regional temperature aggregate |
| s_weather_pressure_* | Flux weather | Barometric pressure |
| s_weather_wind_* | Flux weather | Wind speed |
| s_crypto_price_* | Flux crypto | Per-asset close, normalized |
| s_crypto_volume_* | Flux crypto | Relative volume |
| s_stock_price_* | Flux stocks | Per-index close |
| s_stock_volume_* | Flux stocks | Relative volume |
| s_air_count_global | Flux airplanes | Total active flights |
| s_air_density_* | Flux airplanes | Regional flight density |
| s_air_speed_avg | Flux airplanes | Fleet average speed |
| s_ship_count_global | Flux ships | Total active vessels |
| s_ship_density_* | Flux ships | Regional vessel density |
| s_ship_speed_avg | Flux ships | Fleet average speed |

### Derived Transforms
Applied per-asset and per-domain where useful. Same list as financial-gene — see
`FINANCIAL_GENE.md` for rationale and constraints.

EMA (9, 21, 50) — ATR(14) — rolling stddev(20) — rolling z-score(20) — delta

### Continuity (IDs 0–2, unchanged)
Gene's only real goals. Self-preservation only.

---

## What Becomes Interesting

The symbol ledger is the output. With diverse empirical feeds, the pattern/symbol machinery
finds co-activation clusters *across* domains — patterns that no individual feed would
surface alone.

A symbol might emerge that co-activates across:
- A commodity price deviation
- Suppressed traffic aggregate in a specific geographic zone
- A weather anomaly in a relevant region
- A correlated equity sector move

No human assigned that relationship. It emerged from co-occurrence statistics. Whether it
reflects genuine causal structure or coincidence is a question for the observer.

The composite symbol graph (Φ_C_NNNN) is particularly worth watching. Composites that
span multiple domains suggest higher-order structure. A composite that activates reliably
across three unrelated feeds is either a genuine cross-domain pattern or a strong
coincidence — both are worth knowing about.

The longer gene runs and the more feeds it observes, the richer and more specific the
vocabulary becomes. This is measured in days and weeks, not hours.

---

## Output Stack

Gene's state is published back to Flux. Downstream consumers can be anything.

### Layer 1 — Raw machine output
`gene-data/expression.log` JSONL: tick, dominant symbol, active symbol cluster,
imbalance, signal deviations. Queryable via `gene-ctl expressions`.

### Layer 2 — Flux publication
Gene publishes its state snapshot to a Flux topic (e.g. `gene.observer.state`) every N
ticks. Any subscriber — dashboard, alerting system, another gene instance — can consume it.
This is the existing optional Flux bridge in `expression/engine.rs`, enabled by default
in observer-gene.

### Layer 3 — Human-facing translation
**flux-universe.com** is the natural surface for this. Options range from simple to rich:

- **Minimal**: show active symbol activations with their contributing signal context.
  No LLM required — just render which signals are deviating and which symbol is dominant.
- **Intermediate**: symbol activation timeline, composite graph visualization, pattern
  frequency heatmap across domains. Still no LLM.
- **Full**: LLM-assisted translation — take a symbol activation event + the signal states
  when it was coined + historical co-occurrence stats → produce a human-readable
  interpretation. Gene builds the vocabulary; the LLM reads it back in natural language.

The LLM translation layer is a companion tool, not part of gene. Gene produces grounded
structured output; translation is a separate concern. The Flux publication (Layer 2) is
the interface between them.

---

## What Is New vs OS Gene

Almost nothing in the core. The only structural addition:

- `signal/flux_multi.rs` — generic Flux subscriber accepting N configurable topic
  namespaces at startup. Same WS pattern as `signal/flux.rs`, parameterized by config.

Everything else — regulation, pattern, symbol, self-model, persistence — inherited unchanged.

---

## What gene-core Reuses Unchanged

All of it. No modifications to gene-core required.

---

## Build Path

1. Create `observer-gene/` repository, add `gene-core` as dependency
2. Implement `signal/flux_multi.rs` — configurable multi-topic Flux subscriber
3. Wire `main.rs` with signal IDs per feed, all weight=0, aggregation config
4. Enable Flux publisher in expression engine (Layer 2 output)
5. Run against available Flux feeds — monitor symbol ledger via `gene-ctl symbols`
6. Evaluate composite symbol graph after sufficient runtime (days, not hours)
7. Build flux-universe.com integration once symbol vocabulary has stabilized

---

## Future Feeds

Flux can carry anything. Signal candidates worth adding as feeds become available:

| Domain | Signal candidates |
|--------|-----------------|
| Energy | Grid frequency, electricity load, natural gas / oil price |
| Commodities | Metals, agricultural futures — price and volume |
| Economic | PMI, freight indices, port throughput, trade volumes |
| Satellite | Nighttime light emission, deforestation rate, ice coverage (processed scalar) |
| Social | Pre-processed sentiment score per topic — empirical if upstream-reduced |
| Infrastructure | Power outage counts, internet traffic aggregates, cellular load |

---

## References

- `gene-core/src/signal/flux.rs` — WS subscription pattern to adapt for multi-topic
- `gene-core/src/signal/world.rs` — WorldSignalPoller aggregation pattern
- `gene-core/src/signal/bus.rs` — `register_with_id()`, weight=0 registration
- `gene-core/src/expression/engine.rs` — Flux publication bridge
- `FINANCIAL_GENE.md` — derived indicator list and "what to avoid" constraints
- `ARCHITECTURE.md` — full layer descriptions
