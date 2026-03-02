use crate::pattern::record::{PatternRecord, TemporalShape};
use crate::signal::types::{SignalId, SignalSnapshot};
use std::collections::HashMap;

const DEVIATION_THRESHOLD: f64 = 0.05;
const WINDOW_SIZE: usize = 8;
const MIN_PATTERN_SIGNALS: usize = 2;

/// Scans the signal ledger window and extracts recurring co-activation patterns.
pub struct PatternExtractor {
    /// Rolling window of recent snapshots for shape analysis
    window: Vec<SignalSnapshot>,
}

impl PatternExtractor {
    pub fn new() -> Self {
        Self { window: Vec::with_capacity(WINDOW_SIZE + 2) }
    }

    /// Feed a new snapshot into the rolling window.
    pub fn push(&mut self, snap: SignalSnapshot) {
        self.window.push(snap);
        if self.window.len() > WINDOW_SIZE {
            self.window.remove(0);
        }
    }

    /// Extract a pattern from the current window, if the window is full.
    /// `baselines` provides each signal's resting point for deviation measurement.
    pub fn extract(&self, baselines: &HashMap<SignalId, f64>) -> Option<ExtractionResult> {
        if self.window.len() < WINDOW_SIZE / 2 {
            return None;
        }

        // Identify active signals: those with mean absolute deviation from baseline above threshold
        let mut mean_deviations: HashMap<SignalId, f64> = HashMap::new();
        let count = self.window.len() as f64;

        // Build a map: signal_id -> list of values across the window
        let mut signal_series: HashMap<SignalId, Vec<f64>> = HashMap::new();
        for snap in &self.window {
            for (id, val) in &snap.values {
                signal_series.entry(*id).or_default().push(*val);
            }
        }

        // Compute mean absolute deviation from actual baseline for each signal
        for (id, series) in &signal_series {
            let baseline = baselines.get(id).copied()
                .unwrap_or_else(|| series.iter().sum::<f64>() / series.len() as f64);
            let mad: f64 = series.iter().map(|v| (v - baseline).abs()).sum::<f64>() / count;
            if mad > DEVIATION_THRESHOLD {
                mean_deviations.insert(*id, mad);
            }
        }

        if mean_deviations.len() < MIN_PATTERN_SIGNALS {
            return None;
        }

        // Compute temporal shapes
        let mut shapes: HashMap<SignalId, TemporalShape> = HashMap::new();
        for (id, series) in &signal_series {
            if mean_deviations.contains_key(id) {
                shapes.insert(*id, classify_shape(series));
            }
        }

        // Build signal set (sorted for stable hash)
        let mut signal_set: Vec<SignalId> = mean_deviations.keys().cloned().collect();
        signal_set.sort();

        // Structural hash: hash of sorted signal IDs + magnitude buckets
        let pattern_id = compute_pattern_hash(&signal_set, &mean_deviations);

        // Mean imbalance across window
        let mean_imbalance = self.window.iter().map(|s| s.imbalance).sum::<f64>() / count;

        Some(ExtractionResult {
            pattern_id,
            signal_set,
            mean_magnitudes: mean_deviations,
            shapes,
            mean_imbalance,
            tick: self.window.last().map(|s| s.tick).unwrap_or(0),
        })
    }

    /// Returns signals deviating from their baseline by more than `deviation_threshold`.
    /// Uses actual baselines rather than tick-to-tick change, so stable chronic
    /// deviations register as active even when the signal value isn't currently moving.
    pub fn current_active(&self, deviation_threshold: f64, baselines: &HashMap<SignalId, f64>) -> HashMap<SignalId, f64> {
        if let Some(snap) = self.window.last() {
            snap.values.iter()
                .filter_map(|(id, val)| {
                    let baseline = baselines.get(id).copied().unwrap_or(*val);
                    let dev = (val - baseline).abs();
                    if dev > deviation_threshold { Some((*id, dev)) } else { None }
                })
                .collect()
        } else {
            HashMap::new()
        }
    }
}

pub struct ExtractionResult {
    pub pattern_id: u64,
    pub signal_set: Vec<SignalId>,
    pub mean_magnitudes: HashMap<SignalId, f64>,
    pub shapes: HashMap<SignalId, TemporalShape>,
    pub mean_imbalance: f64,
    pub tick: u64,
}

fn classify_shape(series: &[f64]) -> TemporalShape {
    if series.len() < 3 {
        return TemporalShape::Plateau;
    }
    let first = series[0];
    let last = series[series.len() - 1];
    let mid = series[series.len() / 2];
    let range = series.iter().cloned().fold(f64::NEG_INFINITY, f64::max)
        - series.iter().cloned().fold(f64::INFINITY, f64::min);

    if range < 0.02 {
        TemporalShape::Plateau
    } else if last > first + 0.01 {
        if mid > last + 0.02 {
            TemporalShape::Spiking
        } else {
            TemporalShape::Rising
        }
    } else if last < first - 0.01 {
        TemporalShape::Falling
    } else {
        TemporalShape::Plateau
    }
}

fn compute_pattern_hash(signal_set: &[SignalId], magnitudes: &HashMap<SignalId, f64>) -> u64 {
    use std::collections::hash_map::DefaultHasher;
    use std::hash::{Hash, Hasher};
    let mut hasher = DefaultHasher::new();
    for id in signal_set {
        id.0.hash(&mut hasher);
        // Bucket magnitude to 0.1 granularity for stable hashing
        let bucket = (magnitudes.get(id).copied().unwrap_or(0.0) * 10.0) as u32;
        bucket.hash(&mut hasher);
    }
    hasher.finish()
}

impl Default for PatternExtractor {
    fn default() -> Self { Self::new() }
}
