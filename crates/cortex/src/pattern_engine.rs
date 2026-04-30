//! Lightweight pattern detection from event streams.
//! Uses simple frequency counting — no external ML dependencies.

use chrono::Utc;
use organism_knowledge::PatternRecord;
use std::collections::HashMap;
use uuid::Uuid;

/// A simplified event record for pattern mining
#[derive(Debug, Clone)]
pub struct EventRecord {
    pub project_id: String,
    pub event_type: String,
    pub description: String,
}

/// Detect patterns: when event_type X, action Y is always taken next.
/// Returns patterns with frequency >= min_frequency.
pub fn detect_patterns(events: &[EventRecord], min_frequency: u32) -> Vec<PatternRecord> {
    // Count (event_type, next_description) pairs
    let mut pair_counts: HashMap<(String, String), u32> = HashMap::new();

    for window in events.windows(2) {
        let trigger = window[0].event_type.clone();
        let action = window[1].description.clone();
        *pair_counts.entry((trigger, action)).or_insert(0) += 1;
    }

    let now = Utc::now();
    pair_counts
        .into_iter()
        .filter(|(_, count)| *count >= min_frequency)
        .map(|((trigger, action), count)| PatternRecord {
            id: Uuid::new_v4().to_string(),
            trigger,
            action,
            frequency: count,
            confidence: (count as f64 / 10.0).min(1.0),
            first_seen: now,
            last_seen: now,
            examples: Vec::new(),
        })
        .collect()
}

/// Calculate confidence from improvement vs noise floor
pub fn calculate_confidence(improvement: f64, noise_floor: f64) -> f64 {
    if noise_floor == 0.0 {
        return if improvement > 0.0 { 1.0 } else { 0.0 };
    }
    (improvement / noise_floor / 3.0).clamp(0.0, 1.0)
}
