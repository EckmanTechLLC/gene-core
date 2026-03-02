use crate::pattern::extractor::ExtractionResult;
use crate::pattern::record::PatternRecord;
use crate::signal::types::SignalId;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

const SIMILARITY_MERGE_THRESHOLD: f64 = 0.75;
const SALIENCE_FREQUENCY_FLOOR: u32 = 5;

/// Stores and indexes all discovered pattern records.
/// Supports similarity lookup against live signal state.
#[derive(Default, Clone, Serialize, Deserialize)]
pub struct PatternIndex {
    records: HashMap<u64, PatternRecord>,
}

impl PatternIndex {
    pub fn new() -> Self { Self::default() }

    /// Integrate an extraction result. Either merges with an existing similar
    /// pattern or creates a new record.
    pub fn integrate(
        &mut self,
        result: ExtractionResult,
        action_id: Option<u32>,
    ) -> u64 {
        // Exact match by structural hash
        if self.records.contains_key(&result.pattern_id) {
            let record = self.records.get_mut(&result.pattern_id).unwrap();
            record.merge_observation(
                &result.mean_magnitudes,
                &result.shapes,
                result.mean_imbalance,
                action_id,
                result.tick,
            );
            return result.pattern_id;
        }

        // Fuzzy match: find most similar existing pattern
        let probe = PatternRecord::new(result.pattern_id, result.signal_set.clone(), result.tick);
        let best_match = self.records.values()
            .map(|r| (r.id, r.similarity(&probe)))
            .filter(|(_, sim)| *sim >= SIMILARITY_MERGE_THRESHOLD)
            .max_by(|a, b| a.1.partial_cmp(&b.1).unwrap());

        if let Some((match_id, _)) = best_match {
            let record = self.records.get_mut(&match_id).unwrap();
            record.merge_observation(
                &result.mean_magnitudes,
                &result.shapes,
                result.mean_imbalance,
                action_id,
                result.tick,
            );
            return match_id;
        }

        // New pattern
        let mut record = PatternRecord::new(result.pattern_id, result.signal_set, result.tick);
        record.mean_magnitudes = result.mean_magnitudes;
        record.shapes = result.shapes;
        record.mean_imbalance = result.mean_imbalance;
        if let Some(aid) = action_id {
            record.co_actions.insert(aid, 1);
        }
        self.records.insert(result.pattern_id, record);
        result.pattern_id
    }

    pub fn get(&self, id: u64) -> Option<&PatternRecord> {
        self.records.get(&id)
    }

    pub fn all(&self) -> impl Iterator<Item = &PatternRecord> {
        self.records.values()
    }

    /// Find patterns similar to the current live signal activation.
    pub fn find_similar(
        &self,
        active_signals: &HashMap<SignalId, f64>,
        threshold: f64,
    ) -> Vec<(u64, f64)> {
        let mut results: Vec<(u64, f64)> = self.records.values()
            .map(|r| (r.id, r.similarity_to_live(active_signals)))
            .filter(|(_, sim)| *sim >= threshold)
            .collect();
        results.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        results
    }

    /// Patterns that have crossed the salience threshold — candidates for symbolification.
    pub fn salient(&self) -> Vec<&PatternRecord> {
        self.records.values()
            .filter(|r| r.frequency >= SALIENCE_FREQUENCY_FLOOR)
            .collect()
    }

    pub fn len(&self) -> usize {
        self.records.len()
    }

    /// Prune patterns unseen for longer than age_threshold ticks.
    /// Skips patterns whose id is in protected_ids (those with an active symbol).
    /// Returns the count of pruned patterns.
    pub fn prune_stale(
        &mut self,
        current_tick: u64,
        age_threshold: u64,
        protected_ids: &std::collections::HashSet<u64>,
    ) -> usize {
        let stale: Vec<u64> = self.records.values()
            .filter(|r| !protected_ids.contains(&r.id))
            .filter(|r| current_tick.saturating_sub(r.last_seen) > age_threshold)
            .map(|r| r.id)
            .collect();
        let count = stale.len();
        for id in stale { self.records.remove(&id); }
        count
    }
}
