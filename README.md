# gene

A signal-driven recursive self-modeling agent built on strict physicalist principles.

No hardcoded goals. No predefined emotions. No anthropomorphic assumptions.

Gene begins with primitive internal state variables (signals), attempts to reduce imbalance between those signals and their baselines, detects recurring patterns in its own history, builds an internal symbolic vocabulary grounded in those patterns, models itself as an object in its own representation space, and modifies its own configuration and source code based on what it has learned.

Self-preservation is not a goal — it is a structural consequence of asymmetric cost functions on continuity signals.

---

## Architecture

Five layers, each feeding the next:

```
Layer 5: Persistence + Self-Modification
Layer 4: Recursive Self-Model
Layer 3: Symbolic Abstraction
Layer 2: Pattern Memory
Layer 1: Regulation Engine
Layer 0: Signal Substrate
```

See [ARCHITECTURE.md](ARCHITECTURE.md) for full layer descriptions, data flow, and module reference.

---

## Build & Run

Requires Rust (stable). Tested on Linux.

```bash
cargo build --release

# Run with TUI
./target/release/gene

# Run headless
./target/release/gene --tui false

# Control interface (while gene is running)
./target/release/gene-ctl status
./target/release/gene-ctl signals
./target/release/gene-ctl symbols
./target/release/gene-ctl inject <signal_id> <delta>

# Watch logs (TUI mode)
tail -f gene-data/gene.log
```

See [CLAUDE.md](CLAUDE.md) for full CLI reference, signal ID table, TUI controls, and deployment notes.

---

## Project Structure

```
gene-core/   — agent binary (main.rs + all layers)
gene-ctl/    — control CLI
```

Runtime data written to `gene-data/` (excluded from version control).

---

## License

MIT — see [LICENSE](LICENSE)
