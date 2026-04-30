pub mod store;
pub mod types;

pub use store::KnowledgeStore;
pub use types::*;

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use tempfile::TempDir;

    fn make_store() -> (KnowledgeStore, TempDir) {
        let tmp = TempDir::new().unwrap();
        let store = KnowledgeStore::open(tmp.path()).unwrap();
        (store, tmp)
    }

    #[test]
    fn test_put_and_get_fix() {
        let (mut store, _tmp) = make_store();
        let fix = FixRecord {
            id: "fix1".to_string(),
            signature_hash: "abc123".to_string(),
            patch: "- old\n+ new".to_string(),
            confidence: 0.9,
            applied_count: 1,
            last_applied: Utc::now(),
            source: "learned".to_string(),
        };
        store.put_fix(&fix).unwrap();
        let retrieved = store.get_fix("abc123").unwrap().unwrap();
        assert_eq!(retrieved.id, "fix1");
        assert!((retrieved.confidence - 0.9).abs() < 0.001);
    }

    #[test]
    fn test_get_nonexistent_returns_none() {
        let (mut store, _tmp) = make_store();
        let result: Option<FixRecord> = store.get("nonexistent_key").unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn test_put_and_get_pattern() {
        let (mut store, _tmp) = make_store();
        let pattern = PatternRecord {
            id: "pat1".to_string(),
            trigger: "optimizing bundle_size".to_string(),
            action: "enable tree-shaking".to_string(),
            frequency: 3,
            confidence: 0.75,
            first_seen: Utc::now(),
            last_seen: Utc::now(),
            examples: vec!["project-a".to_string()],
        };
        store.put_pattern(&pattern).unwrap();
        let retrieved = store.get_pattern("pat1").unwrap().unwrap();
        assert_eq!(retrieved.trigger, "optimizing bundle_size");
        assert_eq!(retrieved.frequency, 3);
    }

    #[test]
    fn test_put_and_get_project() {
        let (mut store, _tmp) = make_store();
        let meta = ProjectMeta {
            id: "proj1".to_string(),
            path: "/home/dev/myapp".to_string(),
            name: "myapp".to_string(),
            detected_stack: vec!["React".to_string(), "TypeScript".to_string()],
            primary_language: Some("TypeScript".to_string()),
            last_accessed: Utc::now(),
            session_count: 5,
        };
        store.put_project(&meta).unwrap();
        let retrieved = store.get_project("proj1").unwrap().unwrap();
        assert_eq!(retrieved.name, "myapp");
        assert_eq!(retrieved.detected_stack.len(), 2);
    }

    #[test]
    fn test_put_get_list_error() {
        let (mut store, _tmp) = make_store();
        let now = Utc::now();
        let rec = ErrorRecord {
            tool: "rustc".to_string(),
            kind: "E0599".to_string(),
            hash: "deadbeef".to_string(),
            raw_excerpt: "error[E0599]".to_string(),
            first_seen: now,
            last_seen: now,
            occurrences: 1,
            last_command: "cargo build".to_string(),
        };
        store.put_error(&rec).unwrap();
        let got = store.get_error("deadbeef").unwrap().unwrap();
        assert_eq!(got.kind, "E0599");
        assert_eq!(got.tool, "rustc");
        assert_eq!(got.occurrences, 1);

        // list should include it
        let listed = store.list_errors().unwrap();
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].hash, "deadbeef");
    }

    #[test]
    fn test_delete_removes_entry() {
        let (mut store, _tmp) = make_store();
        let fix = FixRecord {
            id: "f2".to_string(),
            signature_hash: "del123".to_string(),
            patch: "patch".to_string(),
            confidence: 0.5,
            applied_count: 0,
            last_applied: Utc::now(),
            source: "manual".to_string(),
        };
        store.put_fix(&fix).unwrap();
        store
            .delete(&format!("{}del123", keys::FIX_PREFIX))
            .unwrap();
        let result = store.get_fix("del123").unwrap();
        assert!(result.is_none());
    }
}
