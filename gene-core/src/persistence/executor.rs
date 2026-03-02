use crate::persistence::codegen::CodeGenerator;
use crate::regulation::action::{Action, ShellCmd, SourceTarget, SystemOp};
use crate::regulation::causal::CausalTracer;
use crate::selfmodel::model::SelfModel;
use crate::signal::bus::SignalBus;
use crate::signal::types::SignalId;
use crate::symbol::activation::SymbolActivationFrame;
use crate::symbol::ledger::SymbolLedger;
use anyhow::Result;
use std::path::PathBuf;
use std::process::Command;

/// Result of executing a SystemOp.
pub struct OpResult {
    pub success: bool,
    pub output: String,
    /// Signal deltas to inject as feedback
    pub signal_feedback: Vec<(SignalId, f64)>,
}

/// Executes SystemOp variants with continuity gating.
/// The continuity gate is a hard prerequisite — if continuity < gate, the op is skipped.
pub struct SystemOpExecutor {
    pub data_dir: PathBuf,
    pub src_root: PathBuf,
    pub workspace_root: PathBuf,
    codegen: CodeGenerator,
    // Well-known signal IDs for feedback (must match main.rs constants)
    pub sig_integrity: SignalId,
    pub sig_coherence: SignalId,
    pub sig_continuity: SignalId,
}

impl SystemOpExecutor {
    pub fn new(
        data_dir: PathBuf,
        src_root: PathBuf,
        workspace_root: PathBuf,
        sig_integrity: SignalId,
        sig_coherence: SignalId,
        sig_continuity: SignalId,
    ) -> Self {
        let codegen = CodeGenerator::new(src_root.clone(), data_dir.clone());
        Self {
            data_dir,
            src_root,
            workspace_root,
            codegen,
            sig_integrity,
            sig_coherence,
            sig_continuity,
        }
    }

    /// Execute a system op. Returns OpResult with signal feedback.
    pub fn execute(
        &self,
        op: &SystemOp,
        bus: &SignalBus,
        causal: &CausalTracer,
        self_model: &SelfModel,
        symbol_ledger: &SymbolLedger,
        frame: &SymbolActivationFrame,
        next_action_id: u32,
        tick: u64,
        continuity_value: f64,
        continuity_gate: f64,
    ) -> (OpResult, Option<Action>) {
        // Hard continuity gate
        if continuity_value < continuity_gate {
            tracing::warn!(
                "system op {:?} blocked by continuity gate ({:.3} < {:.3})",
                std::mem::discriminant(op), continuity_value, continuity_gate
            );
            return (OpResult {
                success: false,
                output: format!("blocked: continuity {:.3} < gate {:.3}", continuity_value, continuity_gate),
                signal_feedback: vec![],
            }, None);
        }

        match op {
            SystemOp::ReadFile { path } => self.exec_read_file(path),
            SystemOp::GenAction => self.exec_gen_action(bus, causal, next_action_id),
            SystemOp::GenSourcePatch { target } => {
                self.exec_gen_source_patch(target, bus, causal, self_model, tick)
            }
            SystemOp::ShellExec { cmd } => self.exec_shell(cmd),
            SystemOp::CargoBuild => self.exec_cargo_build(continuity_value),
            SystemOp::ReloadActions => self.exec_reload_signal(),
            SystemOp::WritePrompt => {
                self.exec_write_prompt(tick, bus, causal, self_model, symbol_ledger, frame)
            }
            SystemOp::ReadPrompt => self.exec_read_prompt(),
            SystemOp::ApplyAndRestart => self.exec_apply_and_restart(continuity_value, tick),
            SystemOp::Renice { niceness } => self.exec_renice(*niceness),
            SystemOp::SpawnStressor => self.exec_spawn_stressor(),
            SystemOp::KillStressor => self.exec_kill_stressor(),
            SystemOp::DropCaches => self.exec_drop_caches(),
        }
    }

    fn exec_read_file(&self, path: &std::path::Path) -> (OpResult, Option<Action>) {
        match std::fs::read_to_string(path) {
            Ok(content) => {
                let size = content.len() as f64;
                // Small positive integrity delta: reading source = coherent self-reference
                let feedback = vec![(self.sig_integrity, 0.005)];
                tracing::info!("read_file: {:?} ({} bytes)", path, size as usize);
                (OpResult { success: true, output: content, signal_feedback: feedback }, None)
            }
            Err(e) => {
                let feedback = vec![(self.sig_coherence, -0.01)];
                (OpResult { success: false, output: e.to_string(), signal_feedback: feedback }, None)
            }
        }
    }

    fn exec_gen_action(
        &self,
        bus: &SignalBus,
        causal: &CausalTracer,
        next_action_id: u32,
    ) -> (OpResult, Option<Action>) {
        match self.codegen.generate_corrective_action(bus, causal, next_action_id) {
            Some(action) => {
                let label = action.label.clone().unwrap_or_default();
                // Save generated action to actions.json for persistence
                let _ = self.append_action_to_json(&action);
                let feedback = vec![
                    (self.sig_integrity, 0.02),
                    (self.sig_coherence, 0.01),
                ];
                tracing::info!("gen_action: coined action {} — {}", action.id, label);
                (OpResult { success: true, output: label, signal_feedback: feedback }, Some(action))
            }
            None => {
                // Not enough data or no chronic deviation — no action needed yet
                (OpResult {
                    success: true,
                    output: "no corrective action warranted at this time".into(),
                    signal_feedback: vec![],
                }, None)
            }
        }
    }

    fn exec_gen_source_patch(
        &self,
        target: &SourceTarget,
        bus: &SignalBus,
        causal: &CausalTracer,
        self_model: &SelfModel,
        tick: u64,
    ) -> (OpResult, Option<Action>) {
        match self.codegen.generate_source_patch(target, bus, causal, self_model, tick) {
            Ok(patch) => {
                let staging_path = self.data_dir.join("source_patch_staging.rs");
                match std::fs::write(&staging_path, &patch) {
                    Ok(_) => {
                        tracing::info!("source patch written to {:?}", staging_path);
                        let feedback = vec![(self.sig_integrity, 0.01)];
                        (OpResult { success: true, output: format!("patch written ({} bytes)", patch.len()), signal_feedback: feedback }, None)
                    }
                    Err(e) => {
                        let feedback = vec![(self.sig_integrity, -0.01)];
                        (OpResult { success: false, output: e.to_string(), signal_feedback: feedback }, None)
                    }
                }
            }
            Err(e) => {
                (OpResult { success: false, output: e.to_string(), signal_feedback: vec![] }, None)
            }
        }
    }

    fn exec_shell(&self, cmd: &ShellCmd) -> (OpResult, Option<Action>) {
        let (program, args): (&str, Vec<&str>) = match cmd {
            ShellCmd::CargoCheck   => ("cargo", vec!["check", "--quiet"]),
            ShellCmd::CargoTest    => ("cargo", vec!["test", "--quiet"]),
            ShellCmd::RustFmt { path } => {
                let path_str = path.to_str().unwrap_or("");
                let result = Command::new("rustfmt")
                    .arg("--check")
                    .arg(path_str)
                    .current_dir(&self.workspace_root)
                    .output();
                return self.shell_result(result, cmd);
            }
            ShellCmd::ListDataDir => ("ls", vec!["-lh"]),
            ShellCmd::ReadTrace   => ("cat", vec!["gene.trace"]),
        };

        let result = Command::new(program)
            .args(&args)
            .current_dir(&self.workspace_root)
            .output();

        self.shell_result(result, cmd)
    }

    fn shell_result(
        &self,
        result: std::io::Result<std::process::Output>,
        cmd: &ShellCmd,
    ) -> (OpResult, Option<Action>) {
        match result {
            Ok(out) => {
                let success = out.status.success();
                let output = String::from_utf8_lossy(&out.stdout).to_string()
                    + &String::from_utf8_lossy(&out.stderr);
                let feedback = if success {
                    vec![(self.sig_coherence, 0.005)]
                } else {
                    vec![(self.sig_coherence, -0.01)]
                };
                tracing::info!("shell {:?}: success={}", std::mem::discriminant(cmd), success);
                (OpResult { success, output, signal_feedback: feedback }, None)
            }
            Err(e) => {
                let feedback = vec![(self.sig_coherence, -0.02)];
                (OpResult { success: false, output: e.to_string(), signal_feedback: feedback }, None)
            }
        }
    }

    fn exec_cargo_build(&self, continuity_value: f64) -> (OpResult, Option<Action>) {
        // Extra continuity requirement for compilation
        if continuity_value < 0.85 {
            return (OpResult {
                success: false,
                output: "cargo build blocked: continuity < 0.85".into(),
                signal_feedback: vec![],
            }, None);
        }

        tracing::info!("exec: cargo build --release");
        let result = Command::new("cargo")
            .args(["build", "--release"])
            .current_dir(&self.workspace_root)
            .output();

        match result {
            Ok(out) => {
                let success = out.status.success();
                let stderr = String::from_utf8_lossy(&out.stderr).to_string();
                if success {
                    tracing::info!("cargo build succeeded");
                    let feedback = vec![
                        (self.sig_integrity, 0.05),
                        (self.sig_coherence, 0.03),
                    ];
                    (OpResult { success: true, output: "build succeeded".into(), signal_feedback: feedback }, None)
                } else {
                    tracing::warn!("cargo build failed: {}", &stderr[..stderr.len().min(200)]);
                    let feedback = vec![
                        (self.sig_integrity, -0.08),
                        (self.sig_coherence, -0.05),
                    ];
                    (OpResult { success: false, output: stderr, signal_feedback: feedback }, None)
                }
            }
            Err(e) => {
                let feedback = vec![(self.sig_integrity, -0.05)];
                (OpResult { success: false, output: e.to_string(), signal_feedback: feedback }, None)
            }
        }
    }

    fn exec_reload_signal(&self) -> (OpResult, Option<Action>) {
        // Signal to main loop that actions.json should be reloaded
        // Main loop polls for this by checking mtime — this just confirms intent
        let path = self.data_dir.join("actions.json");
        let exists = path.exists();
        tracing::info!("reload_actions: actions.json exists={}", exists);
        (OpResult {
            success: exists,
            output: if exists { "reload signal sent".into() } else { "actions.json not found".into() },
            signal_feedback: vec![(self.sig_integrity, 0.01)],
        }, None)
    }

    fn exec_write_prompt(
        &self,
        tick: u64,
        bus: &SignalBus,
        causal: &CausalTracer,
        self_model: &SelfModel,
        symbol_ledger: &SymbolLedger,
        frame: &SymbolActivationFrame,
    ) -> (OpResult, Option<Action>) {
        let content = self.codegen.generate_self_prompt(
            tick, bus, causal, self_model, symbol_ledger, frame
        );
        let path = self.data_dir.join("self_prompt.md");
        match std::fs::write(&path, &content) {
            Ok(_) => {
                tracing::info!("self_prompt.md written ({} bytes) at tick {}", content.len(), tick);
                let feedback = vec![
                    (self.sig_integrity, 0.01),
                    (self.sig_coherence, 0.02),
                ];
                (OpResult { success: true, output: format!("self_prompt.md written ({} bytes)", content.len()), signal_feedback: feedback }, None)
            }
            Err(e) => {
                let feedback = vec![(self.sig_coherence, -0.01)];
                (OpResult { success: false, output: e.to_string(), signal_feedback: feedback }, None)
            }
        }
    }

    fn exec_read_prompt(&self) -> (OpResult, Option<Action>) {
        let path = self.data_dir.join("self_prompt.md");
        match std::fs::read_to_string(&path) {
            Ok(content) => {
                tracing::info!("self_prompt.md read ({} bytes)", content.len());
                let feedback = vec![(self.sig_coherence, 0.01)];
                (OpResult { success: true, output: content, signal_feedback: feedback }, None)
            }
            Err(e) => {
                (OpResult { success: false, output: e.to_string(), signal_feedback: vec![] }, None)
            }
        }
    }

    fn exec_apply_and_restart(
        &self,
        continuity_value: f64,
        tick: u64,
    ) -> (OpResult, Option<Action>) {
        // Highest continuity gate — only restart if fully stable
        if continuity_value < 0.95 {
            return (OpResult {
                success: false,
                output: format!("restart blocked: continuity {:.3} < 0.95", continuity_value),
                signal_feedback: vec![],
            }, None);
        }

        let new_binary = self.workspace_root.join("target/release/gene");
        if !new_binary.exists() {
            return (OpResult {
                success: false,
                output: "release binary not found — run CargoBuild first".into(),
                signal_feedback: vec![(self.sig_integrity, -0.02)],
            }, None);
        }

        tracing::info!("apply_and_restart: exec into new binary at tick {}", tick);
        // Write a restart marker so the new process knows it's a self-restart
        let marker = self.data_dir.join("restart_marker");
        let _ = std::fs::write(&marker, format!("{}", tick));

        // exec() replaces the process — state is preserved via checkpoint
        use std::os::unix::process::CommandExt;
        let err = Command::new(&new_binary)
            .arg("--data-dir")
            .arg(&self.data_dir)
            .exec(); // never returns on success

        (OpResult {
            success: false,
            output: format!("exec failed: {}", err),
            signal_feedback: vec![(self.sig_continuity, -0.1)],
        }, None)
    }

    fn exec_renice(&self, niceness: i32) -> (OpResult, Option<Action>) {
        let pid = std::process::id();
        // Use sudo to allow restoring priority (non-root can only increase nice without it)
        let result = Command::new("sudo")
            .args(["renice", "-n", &niceness.to_string(), "-p", &pid.to_string()])
            .output();
        match result {
            Ok(out) if out.status.success() => {
                tracing::info!("renice: PID {} → nice={}", pid, niceness);
                (OpResult {
                    success: true,
                    output: format!("nice={}", niceness),
                    signal_feedback: vec![(self.sig_coherence, 0.01)],
                }, None)
            }
            Ok(out) => {
                let err = String::from_utf8_lossy(&out.stderr).to_string();
                tracing::warn!("renice failed: {}", err);
                (OpResult { success: false, output: err, signal_feedback: vec![(self.sig_coherence, -0.005)] }, None)
            }
            Err(e) => (OpResult { success: false, output: e.to_string(), signal_feedback: vec![] }, None),
        }
    }

    fn exec_spawn_stressor(&self) -> (OpResult, Option<Action>) {
        let marker = self.data_dir.join("stressor.pid");
        // Check if already running
        if marker.exists() {
            if let Ok(pid_str) = std::fs::read_to_string(&marker) {
                if let Ok(pid) = pid_str.trim().parse::<u32>() {
                    if std::path::Path::new(&format!("/proc/{}", pid)).exists() {
                        return (OpResult {
                            success: true,
                            output: format!("stressor already running (PID {})", pid),
                            signal_feedback: vec![],
                        }, None);
                    }
                }
            }
            let _ = std::fs::remove_file(&marker);
        }
        match Command::new("stress-ng")
            .args(["--cpu", "1", "--vm", "1", "--vm-bytes", "256M", "--timeout", "120s", "--quiet"])
            .spawn()
        {
            Ok(child) => {
                let pid = child.id();
                let _ = std::fs::write(&marker, pid.to_string());
                tracing::info!("spawn_stressor: stress-ng PID {}", pid);
                (OpResult {
                    success: true,
                    output: format!("spawned PID {}", pid),
                    signal_feedback: vec![(self.sig_integrity, -0.01)],
                }, None)
            }
            Err(e) => {
                tracing::warn!("spawn_stressor failed: {}", e);
                (OpResult { success: false, output: e.to_string(), signal_feedback: vec![(self.sig_coherence, -0.01)] }, None)
            }
        }
    }

    fn exec_kill_stressor(&self) -> (OpResult, Option<Action>) {
        let marker = self.data_dir.join("stressor.pid");
        let _ = std::fs::remove_file(&marker);
        match Command::new("pkill").args(["-f", "stress-ng"]).output() {
            Ok(out) => {
                // pkill returns 1 if no process matched — that's fine
                let code = out.status.code().unwrap_or(0);
                tracing::info!("kill_stressor: pkill exit code {}", code);
                (OpResult {
                    success: true,
                    output: "stressor killed".into(),
                    signal_feedback: vec![(self.sig_integrity, 0.01)],
                }, None)
            }
            Err(e) => (OpResult { success: false, output: e.to_string(), signal_feedback: vec![] }, None),
        }
    }

    fn exec_drop_caches(&self) -> (OpResult, Option<Action>) {
        let _ = Command::new("sync").output();
        // Requires: etl ALL=(ALL) NOPASSWD: /usr/bin/tee /proc/sys/vm/drop_caches
        let result = Command::new("sudo")
            .args(["tee", "/proc/sys/vm/drop_caches"])
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::null())
            .spawn()
            .and_then(|mut child| {
                use std::io::Write;
                if let Some(stdin) = child.stdin.as_mut() {
                    stdin.write_all(b"3")?;
                }
                child.wait()
            });
        match result {
            Ok(status) if status.success() => {
                tracing::info!("drop_caches: page caches dropped");
                (OpResult {
                    success: true,
                    output: "caches dropped".into(),
                    signal_feedback: vec![(self.sig_coherence, 0.01)],
                }, None)
            }
            Ok(status) => {
                let msg = format!("drop_caches: exit code {:?}", status.code());
                tracing::warn!("{}", msg);
                (OpResult { success: false, output: msg, signal_feedback: vec![(self.sig_coherence, -0.005)] }, None)
            }
            Err(e) => {
                tracing::warn!("drop_caches failed: {}", e);
                (OpResult { success: false, output: e.to_string(), signal_feedback: vec![(self.sig_coherence, -0.005)] }, None)
            }
        }
    }

    fn append_action_to_json(&self, action: &Action) -> anyhow::Result<()> {
        let path = self.data_dir.join("actions.json");
        let mut actions: Vec<Action> = if path.exists() {
            let content = std::fs::read_to_string(&path)?;
            serde_json::from_str(&content).unwrap_or_default()
        } else {
            Vec::new()
        };
        // Avoid duplicates
        if !actions.iter().any(|a| a.id == action.id) {
            actions.push(action.clone());
        }
        let json = serde_json::to_string_pretty(&actions)?;
        std::fs::write(&path, json)?;
        Ok(())
    }
}
