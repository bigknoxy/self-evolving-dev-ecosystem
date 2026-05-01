pub mod apply;
pub mod context_detector;
pub mod error_classifier;
pub mod pattern_engine;
pub mod suggest;

pub use context_detector::*;
pub use error_classifier::{classify, ErrorSignature};
pub use pattern_engine::*;
pub use suggest::{suggest_for_error, LlmClient};

#[cfg(test)]
mod tests {
    use super::*;
    use pattern_engine::{calculate_confidence, detect_patterns, EventRecord};

    fn make_event(project: &str, etype: &str, desc: &str) -> EventRecord {
        EventRecord {
            project_id: project.to_string(),
            event_type: etype.to_string(),
            description: desc.to_string(),
        }
    }

    #[test]
    fn test_detect_patterns_frequency_threshold() {
        let events = vec![
            make_event("p1", "build_error", "try cargo fix"),
            make_event("p1", "build_error", "try cargo fix"),
            make_event("p1", "build_error", "try cargo fix"),
            make_event("p1", "lint_warning", "run ruff fix"),
        ];
        let patterns = detect_patterns(&events, 2);
        // The "build_error → try cargo fix" pair appears 2 times in windows
        assert!(!patterns.is_empty());
        let found = patterns
            .iter()
            .any(|p| p.trigger == "build_error" && p.action == "try cargo fix");
        assert!(found, "Expected build_error pattern");
    }

    #[test]
    fn test_detect_patterns_below_threshold_excluded() {
        let events = vec![
            make_event("p1", "rare_event", "rare_action"),
            make_event("p1", "other_event", "other_action"),
        ];
        // With min_frequency=3, nothing should be returned
        let patterns = detect_patterns(&events, 3);
        assert!(patterns.is_empty());
    }

    #[test]
    fn test_calculate_confidence_no_noise() {
        assert_eq!(calculate_confidence(5.0, 0.0), 1.0);
        assert_eq!(calculate_confidence(0.0, 0.0), 0.0);
    }

    #[test]
    fn test_calculate_confidence_clamped() {
        let c = calculate_confidence(100.0, 1.0);
        assert!(c <= 1.0);
        let c2 = calculate_confidence(0.0, 5.0);
        assert!(c2 >= 0.0);
    }

    #[test]
    fn test_context_detector_nonexistent_path() {
        let meta = context_detector::detect_project("/this/does/not/exist");
        assert!(meta.is_none());
    }
}
