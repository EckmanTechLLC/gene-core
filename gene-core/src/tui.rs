use crate::signal::bus::SignalBus;
use crate::signal::types::SignalClass;
use crate::symbol::activation::SymbolActivationFrame;
use anyhow::Result;
use ratatui::{
    backend::CrosstermBackend,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Gauge, List, ListItem, Paragraph},
    Terminal,
};
use crossterm::{
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::io;
use std::time::Instant;

pub struct TuiState {
    terminal: Option<Terminal<CrosstermBackend<io::Stdout>>>,
    // Display data
    tick: u64,
    imbalance: f64,
    /// (id, name, class, value, baseline, weight) — sorted by class then id
    signals: Vec<(u32, String, SignalClass, f64, f64, f64)>,
    active_symbols: Vec<(String, f64)>,
    last_action: Option<u32>,
    confidence: f64,
    pattern_count: usize,
    symbol_count: usize,
    composite_count: usize,
    paused: bool,
    recent_expressions: Vec<serde_json::Value>,
    activity_log: Vec<String>,
    last_render: Instant,
    tick_rate: f64,
    // Stress mode
    stress_mode: bool,
    /// Index into self.signals of the currently selected signal
    selected_signal: usize,
    /// Active oscillation: (signal_id, direction +1/-1, remaining pulses)
    oscillate: Option<(u32, i32, u32)>,
    // Weight edit mode
    weight_mode: bool,
}

impl TuiState {
    pub fn new() -> Self {
        let terminal = Self::init_terminal().ok();
        Self {
            terminal,
            tick: 0,
            imbalance: 0.0,
            signals: Vec::new(),
            active_symbols: Vec::new(),
            last_action: None,
            confidence: 0.5,
            pattern_count: 0,
            symbol_count: 0,
            composite_count: 0,
            paused: false,
            recent_expressions: Vec::new(),
            activity_log: Vec::new(),
            last_render: Instant::now(),
            tick_rate: 0.0,
            stress_mode: false,
            selected_signal: 0,
            oscillate: None,
            weight_mode: false,
        }
    }

    fn init_terminal() -> Result<Terminal<CrosstermBackend<io::Stdout>>> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(stdout, EnterAlternateScreen)?;
        let backend = CrosstermBackend::new(io::stdout());
        Ok(Terminal::new(backend)?)
    }

    pub fn update(
        &mut self,
        tick: u64,
        imbalance: f64,
        bus: &SignalBus,
        frame: &SymbolActivationFrame,
        last_action: Option<u32>,
        confidence: f64,
        pattern_count: usize,
        symbol_count: usize,
        composite_count: usize,
        paused: bool,
        recent_expressions: Vec<serde_json::Value>,
        activity_log: Vec<String>,
    ) {
        // Compute tick rate: update() is called every 20 ticks
        let elapsed = self.last_render.elapsed().as_secs_f64();
        if elapsed > 0.01 {
            self.tick_rate = 20.0 / elapsed;
        }
        self.last_render = Instant::now();

        self.tick = tick;
        self.imbalance = imbalance;
        self.last_action = last_action;
        self.confidence = confidence;
        self.pattern_count = pattern_count;
        self.symbol_count = symbol_count;
        self.composite_count = composite_count;
        self.paused = paused;
        self.recent_expressions = recent_expressions;
        self.activity_log = activity_log;

        self.signals = bus.all_signals()
            .iter()
            .map(|(id, sig)| (id.0, signal_name(id.0), sig.class, sig.value, sig.baseline, sig.weight))
            .collect();
        // Sort: Continuity → Derived → Somatic → World → Efferent, then by id
        self.signals.sort_by(|a, b| {
            let co = class_order(&a.2).cmp(&class_order(&b.2));
            if co != std::cmp::Ordering::Equal { co } else { a.0.cmp(&b.0) }
        });

        // Clamp selection to valid range after signal list may have changed
        if !self.signals.is_empty() {
            self.selected_signal = self.selected_signal.min(self.signals.len() - 1);
        }

        self.active_symbols = frame.active.iter()
            .take(8)
            .map(|(_, tok, str)| (tok.clone(), *str))
            .collect();
    }

    /// Render the TUI and return inject requests and weight changes produced by keypresses.
    /// Injects: (signal_id, delta). Weight changes: (signal_id, new_weight).
    pub fn render(&mut self) -> Result<(Vec<(u32, f64)>, Vec<(u32, f64)>)> {
        let term = match &mut self.terminal {
            Some(t) => t,
            None => return Ok((Vec::new(), Vec::new())),
        };

        let tick            = self.tick;
        let imbalance       = self.imbalance;
        let signals         = self.signals.clone();
        let active_symbols  = self.active_symbols.clone();
        let last_action     = self.last_action;
        let confidence      = self.confidence;
        let pattern_count   = self.pattern_count;
        let symbol_count    = self.symbol_count;
        let composite_count = self.composite_count;
        let paused          = self.paused;
        let recent_exprs    = self.recent_expressions.clone();
        let activity_log    = self.activity_log.clone();
        let tick_rate       = self.tick_rate;
        let stress_mode     = self.stress_mode;
        let weight_mode     = self.weight_mode;
        let selected        = self.selected_signal;
        let oscillating     = self.oscillate.is_some();

        term.draw(|f| {
            let area = f.area();

            // Outer: [main area | activity log | footer]
            let footer_h = if stress_mode || weight_mode { 5 } else { 3 };
            let outer = Layout::default()
                .direction(Direction::Vertical)
                .constraints([
                    Constraint::Min(10),
                    Constraint::Length(7),   // activity console (~5 events + borders)
                    Constraint::Length(footer_h),
                ])
                .split(area);

            // Main: [left 58% | right 42%]
            let main = Layout::default()
                .direction(Direction::Horizontal)
                .constraints([Constraint::Percentage(58), Constraint::Percentage(42)])
                .split(outer[0]);

            // Left: [header | imbalance gauge | signals]
            let left = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(3), Constraint::Length(3), Constraint::Min(8)])
                .split(main[0]);

            // Right: [active symbols | expression feed]
            let right = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Length(12), Constraint::Min(6)])
                .split(main[1]);

            render_header(f, left[0], tick, pattern_count, symbol_count,
                composite_count, confidence, last_action, tick_rate, paused);
            render_imbalance(f, left[1], imbalance);
            render_signals(f, left[2], &signals, stress_mode, weight_mode, selected);
            render_symbols(f, right[0], &active_symbols);
            render_expressions(f, right[1], &recent_exprs);
            render_activity(f, outer[1], &activity_log);
            render_footer(f, outer[2], stress_mode, weight_mode, oscillating,
                signals.get(selected).map(|(id, name, _, val, base, weight)| (*id, name.as_str(), *val, *base, *weight)));
        })?;

        let mut injects: Vec<(u32, f64)> = Vec::new();
        let mut weight_changes: Vec<(u32, f64)> = Vec::new();

        // Emit next oscillation pulse if active
        if let Some((sig_id, dir, remaining)) = self.oscillate {
            injects.push((sig_id, dir as f64 * 0.5));
            if remaining <= 1 {
                self.oscillate = None;
            } else {
                self.oscillate = Some((sig_id, -dir, remaining - 1));
            }
        }

        // Non-blocking keypress check
        if crossterm::event::poll(std::time::Duration::from_millis(0))? {
            if let crossterm::event::Event::Key(key) = crossterm::event::read()? {
                use crossterm::event::{KeyCode, KeyModifiers};

                if !self.stress_mode && !self.weight_mode {
                    // Normal mode keys
                    match key.code {
                        KeyCode::Char('q') => {
                            self.cleanup();
                            std::process::exit(0);
                        }
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            self.cleanup();
                            std::process::exit(0);
                        }
                        KeyCode::Char('s') => {
                            self.stress_mode = true;
                        }
                        KeyCode::Char('w') => {
                            self.weight_mode = true;
                        }
                        _ => {}
                    }
                } else if self.stress_mode {
                    // Stress mode keys
                    let n = self.signals.len();
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('s') => {
                            self.stress_mode = false;
                            self.oscillate = None;
                        }
                        KeyCode::Char('q') => {
                            self.cleanup();
                            std::process::exit(0);
                        }
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            self.cleanup();
                            std::process::exit(0);
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if n > 0 {
                                self.selected_signal = self.selected_signal.saturating_sub(1);
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if n > 0 {
                                self.selected_signal = (self.selected_signal + 1).min(n - 1);
                            }
                        }
                        // Nudge: +0.3 / -0.3 on selected signal
                        KeyCode::Char('+') | KeyCode::Char('=') => {
                            if let Some(&(id, _, _, _, _, _)) = self.signals.get(self.selected_signal) {
                                injects.push((id, 0.3));
                            }
                        }
                        KeyCode::Char('-') => {
                            if let Some(&(id, _, _, _, _, _)) = self.signals.get(self.selected_signal) {
                                injects.push((id, -0.3));
                            }
                        }
                        // Preset 1: spike +1.0
                        KeyCode::Char('1') => {
                            if let Some(&(id, _, _, _, _, _)) = self.signals.get(self.selected_signal) {
                                injects.push((id, 1.0));
                            }
                        }
                        // Preset 2: drop -1.0
                        KeyCode::Char('2') => {
                            if let Some(&(id, _, _, _, _, _)) = self.signals.get(self.selected_signal) {
                                injects.push((id, -1.0));
                            }
                        }
                        // Preset 3: normalize — push toward baseline
                        KeyCode::Char('3') => {
                            if let Some(&(id, _, _, val, base, _)) = self.signals.get(self.selected_signal) {
                                let delta = (base - val).clamp(-2.0, 2.0);
                                injects.push((id, delta));
                            }
                        }
                        // Preset 4: pressure — +0.5 on all somatic signals
                        KeyCode::Char('4') => {
                            for &(id, _, class, _, _, _) in &self.signals {
                                if class == SignalClass::Somatic {
                                    injects.push((id, 0.5));
                                }
                            }
                        }
                        // Preset 5: oscillate selected signal (20 pulses, ±0.5, one per render)
                        KeyCode::Char('5') => {
                            if let Some(&(id, _, _, _, _, _)) = self.signals.get(self.selected_signal) {
                                self.oscillate = Some((id, 1, 20));
                            }
                        }
                        _ => {}
                    }
                } else {
                    // Weight edit mode keys
                    let n = self.signals.len();
                    match key.code {
                        KeyCode::Esc | KeyCode::Char('w') => {
                            self.weight_mode = false;
                        }
                        KeyCode::Char('q') => {
                            self.cleanup();
                            std::process::exit(0);
                        }
                        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => {
                            self.cleanup();
                            std::process::exit(0);
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            if n > 0 {
                                self.selected_signal = self.selected_signal.saturating_sub(1);
                            }
                        }
                        KeyCode::Down | KeyCode::Char('j') => {
                            if n > 0 {
                                self.selected_signal = (self.selected_signal + 1).min(n - 1);
                            }
                        }
                        // [ decrease weight by 0.5 (Continuity signals protected)
                        KeyCode::Char('[') => {
                            if let Some(entry) = self.signals.get_mut(self.selected_signal) {
                                if entry.2 != SignalClass::Continuity {
                                    let new_weight = (entry.5 - 0.5).max(0.0);
                                    entry.5 = new_weight;
                                    weight_changes.push((entry.0, new_weight));
                                }
                            }
                        }
                        // ] increase weight by 0.5
                        KeyCode::Char(']') => {
                            if let Some(entry) = self.signals.get_mut(self.selected_signal) {
                                if entry.2 != SignalClass::Continuity {
                                    let new_weight = (entry.5 + 0.5).min(20.0);
                                    entry.5 = new_weight;
                                    weight_changes.push((entry.0, new_weight));
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        Ok((injects, weight_changes))
    }

    fn cleanup(&mut self) {
        if self.terminal.is_some() {
            disable_raw_mode().ok();
            execute!(io::stdout(), LeaveAlternateScreen).ok();
        }
    }
}

impl Drop for TuiState {
    fn drop(&mut self) {
        self.cleanup();
    }
}

// ── Panel renderers ────────────────────────────────────────────────────────────

fn render_header(
    f: &mut ratatui::Frame,
    area: Rect,
    tick: u64,
    pattern_count: usize,
    symbol_count: usize,
    composite_count: usize,
    confidence: f64,
    last_action: Option<u32>,
    tick_rate: f64,
    paused: bool,
) {
    let action_str = last_action
        .map(|a| format!("a_{}", a))
        .unwrap_or_else(|| "none".into());

    let mut spans = vec![
        Span::styled("GENE ", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        Span::raw(format!(
            "tick:{} pat:{} sym:{} comp:{} conf:{:.3} {} {:.1}t/s",
            abbrev_tick(tick), pattern_count, symbol_count,
            composite_count, confidence, action_str, tick_rate,
        )),
    ];
    if paused {
        spans.push(Span::styled(
            "  [PAUSED]",
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        ));
    }

    let header = Paragraph::new(Line::from(spans))
        .block(Block::default().borders(Borders::ALL));
    f.render_widget(header, area);
}

fn render_imbalance(f: &mut ratatui::Frame, area: Rect, imbalance: f64) {
    // Scale: 0–200 covers the typical operating range (chronic baseline ~100)
    let ratio = (imbalance / 200.0).clamp(0.0, 1.0);
    let color = if ratio < 0.35 { Color::Green } else if ratio < 0.65 { Color::Yellow } else { Color::Red };
    let gauge = Gauge::default()
        .block(Block::default().title(format!("Imbalance  {:.4}", imbalance)).borders(Borders::ALL))
        .gauge_style(Style::default().fg(color))
        .ratio(ratio)
        .label("");
    f.render_widget(gauge, area);
}

fn render_signals(
    f: &mut ratatui::Frame,
    area: Rect,
    signals: &[(u32, String, SignalClass, f64, f64, f64)],
    stress_mode: bool,
    weight_mode: bool,
    selected: usize,
) {
    let mut items: Vec<ListItem> = Vec::new();
    let mut current_class: Option<SignalClass> = None;
    let mut signal_idx = 0usize;

    for (_, name, class, val, base, weight) in signals {
        // Insert class section header on class boundary
        if current_class.map_or(true, |c| c != *class) {
            let label = match class {
                SignalClass::Continuity => "── Continuity ──",
                SignalClass::Derived    => "── Derived ──",
                SignalClass::Somatic    => "── Somatic ──",
                SignalClass::World      => "── World ──",
                SignalClass::Efferent   => "── Efferent ──",
            };
            items.push(ListItem::new(Line::from(
                Span::styled(label, Style::default().fg(Color::DarkGray))
            )));
            current_class = Some(*class);
        }

        let is_selected = (stress_mode || weight_mode) && signal_idx == selected;
        let row_style = if is_selected {
            Style::default().add_modifier(Modifier::REVERSED)
        } else {
            Style::default()
        };

        if weight_mode {
            let protected = *class == SignalClass::Continuity;
            let cursor_color = if protected { Color::DarkGray } else { Color::Green };
            let cursor = if is_selected { "▶ " } else { "  " };
            let bar_len = ((*weight / 10.0 * 20.0) as usize).min(20);
            let bar = "█".repeat(bar_len);
            let w_color = if protected { Color::DarkGray } else { Color::Green };
            items.push(ListItem::new(Line::from(vec![
                Span::styled(cursor, Style::default().fg(cursor_color)),
                Span::styled(format!("{:12} ", name), row_style.fg(Color::Gray)),
                Span::styled(format!("w:{:5.2}  ", weight), row_style.fg(w_color)),
                Span::styled(bar, row_style.fg(w_color)),
            ])));
        } else {
            let dev = val - base;
            let bar_len = ((dev.abs() * 20.0) as usize).min(20);
            let bar = "█".repeat(bar_len);
            let dev_color = if dev.abs() < 0.05 {
                Color::DarkGray
            } else if dev > 0.0 {
                Color::Yellow
            } else {
                Color::Blue
            };
            let cursor = if is_selected { "▶ " } else { "  " };
            items.push(ListItem::new(Line::from(vec![
                Span::styled(cursor, Style::default().fg(Color::Red)),
                Span::styled(format!("{:12} ", name), row_style.fg(Color::Gray)),
                Span::styled(format!("{:.4} ", val),  row_style),
                Span::styled(format!("[{:>+.3}] ", dev), row_style.fg(dev_color)),
                Span::styled(bar, row_style.fg(dev_color)),
            ])));
        }

        signal_idx += 1;
    }

    if items.is_empty() {
        items.push(ListItem::new("(no signals)"));
    }

    let title = if stress_mode {
        "Signals  [STRESS]"
    } else if weight_mode {
        "Signals  [WEIGHT]"
    } else {
        "Signals"
    };
    f.render_widget(
        List::new(items).block(Block::default().title(title).borders(Borders::ALL)),
        area,
    );
}

fn render_symbols(f: &mut ratatui::Frame, area: Rect, symbols: &[(String, f64)]) {
    let items: Vec<ListItem> = if symbols.is_empty() {
        vec![ListItem::new(Span::styled(
            "(no active symbols yet)",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        symbols.iter().map(|(tok, str)| {
            let bar_len = (*str * 20.0) as usize;
            let bar = "▪".repeat(bar_len.min(20));
            // Composite symbols (Φ_C_NNNN) rendered in Cyan; primitives in Magenta
            let color = if tok.starts_with("Φ_C_") { Color::Cyan } else { Color::Magenta };
            ListItem::new(Line::from(vec![
                Span::styled(format!("{:10} ", tok),  Style::default().fg(color)),
                Span::styled(format!("{:.3} ", str),  Style::default()),
                Span::styled(bar, Style::default().fg(color)),
            ]))
        }).collect()
    };

    f.render_widget(
        List::new(items).block(Block::default().title("Active Symbols").borders(Borders::ALL)),
        area,
    );
}

fn render_expressions(f: &mut ratatui::Frame, area: Rect, exprs: &[serde_json::Value]) {
    let items: Vec<ListItem> = if exprs.is_empty() {
        vec![ListItem::new(Span::styled(
            "(no expressions yet)",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        // Show most recent first (newest at top of panel)
        exprs.iter().rev().take(20).map(|rec| {
            let tick     = rec.get("tick").and_then(|v| v.as_u64()).unwrap_or(0);
            let dominant = rec.get("dominant").and_then(|v| v.as_str()).unwrap_or("?");
            let trend    = rec.get("imbalance_trend").and_then(|v| v.as_str()).unwrap_or("?");
            let action   = rec.get("action_context").and_then(|v| v.as_str()).unwrap_or("?");
            let align    = rec.get("identity_alignment").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let imb      = rec.get("imbalance").and_then(|v| v.as_f64()).unwrap_or(0.0);

            let (trend_ch, trend_color) = match trend {
                "rising"  => ("↑", Color::Red),
                "falling" => ("↓", Color::Green),
                _         => ("─", Color::DarkGray),
            };
            let action_short = action.strip_prefix("action_")
                .map(|s| format!("a{}", s))
                .unwrap_or_else(|| action.to_string());
            let dom_color = if dominant.starts_with("Φ_C_") { Color::Cyan } else { Color::Magenta };

            ListItem::new(Line::from(vec![
                Span::styled(format!("{} ", abbrev_tick(tick)), Style::default().fg(Color::DarkGray)),
                Span::styled(trend_ch, Style::default().fg(trend_color)),
                Span::raw(" "),
                Span::styled(format!("{:10} ", dominant), Style::default().fg(dom_color)),
                Span::styled(format!("{:5} ", action_short), Style::default().fg(Color::Gray)),
                Span::styled(format!("imb:{:.1} ", imb), Style::default().fg(Color::DarkGray)),
                Span::styled(format!("al:{:.2}", align), Style::default().fg(Color::DarkGray)),
            ]))
        }).collect()
    };

    f.render_widget(
        List::new(items).block(Block::default().title("Expressions").borders(Borders::ALL)),
        area,
    );
}

fn render_activity(f: &mut ratatui::Frame, area: Rect, log: &[String]) {
    let capacity = area.height.saturating_sub(2) as usize;

    let items: Vec<ListItem> = if log.is_empty() {
        vec![ListItem::new(Span::styled(
            "(no activity yet)",
            Style::default().fg(Color::DarkGray),
        ))]
    } else {
        log.iter().take(capacity).map(|line| {
            // Colour-code by event type
            let style = if line.contains("FAIL") {
                Style::default().fg(Color::Red)
            } else if line.contains("sys:CargoBuild") || line.contains("sys:ApplyAndRestart") {
                Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)
            } else if line.contains("sys:") {
                Style::default().fg(Color::White)
            } else if line.contains("coined") {
                Style::default().fg(Color::Cyan)
            } else if line.contains("stress:") {
                Style::default().fg(Color::Red)
            } else if line.contains("new action") || line.contains("hot-reload") {
                Style::default().fg(Color::Yellow)
            } else {
                Style::default().fg(Color::DarkGray)
            };

            // Split tick prefix from message for distinct colouring
            if let Some(sep) = line.find("  ") {
                let (prefix, rest) = line.split_at(sep);
                ListItem::new(Line::from(vec![
                    Span::styled(prefix.to_string(), Style::default().fg(Color::DarkGray)),
                    Span::styled(rest.to_string(), style),
                ]))
            } else {
                ListItem::new(Span::styled(line.clone(), style))
            }
        }).collect()
    };

    f.render_widget(
        List::new(items).block(Block::default().title("Activity").borders(Borders::ALL)),
        area,
    );
}

fn render_footer(
    f: &mut ratatui::Frame,
    area: Rect,
    stress_mode: bool,
    weight_mode: bool,
    oscillating: bool,
    selected: Option<(u32, &str, f64, f64, f64)>,
) {
    if stress_mode {
        let sel_line = if let Some((id, name, val, base, _)) = selected {
            format!("selected: {} (id:{})  val:{:.4}  dev:{:+.4}  base:{:.4}",
                name, id, val, val - base, base)
        } else {
            "no signal selected".to_string()
        };
        let osc_str = if oscillating { "  [OSCILLATING]" } else { "" };
        let lines = vec![
            Line::from(vec![
                Span::styled("STRESS  ", Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
                Span::styled("↑↓/jk:select  ", Style::default().fg(Color::White)),
                Span::styled("+/-:nudge±0.3  ", Style::default().fg(Color::Yellow)),
                Span::styled("1:spike+1  2:drop-1  3:normalize  4:pressure(all somatic)  5:oscillate",
                    Style::default().fg(Color::Gray)),
                Span::styled(osc_str, Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            ]),
            Line::from(Span::styled(sel_line, Style::default().fg(Color::Cyan))),
            Line::from(Span::styled(
                "s/Esc:exit stress  q:quit",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        let footer = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Red)));
        f.render_widget(footer, area);
    } else if weight_mode {
        let sel_line = if let Some((id, name, _, _, weight)) = selected {
            let protected = name.contains("continuity") || name.contains("integrity") || name.contains("coherence");
            if protected {
                format!("selected: {} (id:{})  weight:{:.2}  [protected — Continuity signals cannot be edited]", name, id, weight)
            } else {
                format!("selected: {} (id:{})  weight:{:.2}", name, id, weight)
            }
        } else {
            "no signal selected".to_string()
        };
        let lines = vec![
            Line::from(vec![
                Span::styled("WEIGHT  ", Style::default().fg(Color::Green).add_modifier(Modifier::BOLD)),
                Span::styled("↑↓/jk:select  ", Style::default().fg(Color::White)),
                Span::styled("[:weight-0.5  ]:weight+0.5  ", Style::default().fg(Color::Yellow)),
                Span::styled("(Continuity signals protected)",
                    Style::default().fg(Color::DarkGray)),
            ]),
            Line::from(Span::styled(sel_line, Style::default().fg(Color::Green))),
            Line::from(Span::styled(
                "w/Esc:exit weight  q:quit",
                Style::default().fg(Color::DarkGray),
            )),
        ];
        let footer = Paragraph::new(lines)
            .block(Block::default().borders(Borders::ALL)
                .border_style(Style::default().fg(Color::Green)));
        f.render_widget(footer, area);
    } else {
        let footer = Paragraph::new(
            "q:quit  Ctrl-C:shutdown  s:stress  w:weights  [gene-ctl inject <id> <delta> | signals | expressions | pause | resume]"
        )
        .block(Block::default().borders(Borders::ALL))
        .style(Style::default().fg(Color::DarkGray));
        f.render_widget(footer, area);
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────────

fn signal_name(id: u32) -> String {
    match id {
        0  => "s_continuity".to_string(),
        1  => "s_integrity".to_string(),
        2  => "s_coherence".to_string(),
        3  => "s_memory".to_string(),
        4  => "s_disk".to_string(),
        5  => "s_meta".to_string(),
        6  => "s_drive".to_string(),
        17 => "s_cpu_load".to_string(),
        18 => "s_net_rx".to_string(),
        19 => "s_net_tx".to_string(),
        20 => "s_disk_io".to_string(),
        21 => "s_uptime".to_string(),
        22 => "s_proc_cnt".to_string(),
        28 => "s_quake_rate".to_string(),
        29 => "s_quake_mag".to_string(),
        30 => "s_quake_depth".to_string(),
        31 => "s_quake_sig".to_string(),
        n  => format!("s_{:04}", n),
    }
}

fn abbrev_tick(t: u64) -> String {
    if t >= 1_000_000 {
        format!("{:.2}M", t as f64 / 1_000_000.0)
    } else if t >= 1_000 {
        format!("{:.1}k", t as f64 / 1_000.0)
    } else {
        format!("{}", t)
    }
}

fn class_order(c: &SignalClass) -> u8 {
    match c {
        SignalClass::Continuity => 0,
        SignalClass::Derived    => 1,
        SignalClass::Somatic    => 2,
        SignalClass::World      => 3,
        SignalClass::Efferent   => 4,
    }
}
