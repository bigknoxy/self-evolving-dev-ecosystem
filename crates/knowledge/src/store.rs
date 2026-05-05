//! File-based key-value store backed by JSON files.
//! Simple and portable — no native dependencies.
//! Suitable for Level 0-2 capability. Swap for RocksDB at Level 3+.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::types::{
    keys, ErrorRecord, FeedbackRecord, FixRecord, PatternRecord, ProjectMeta, SuggestionRecord,
};

/// Summary of an error for listing and display
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ErrorSummary {
    pub hash: String,
    pub last_command: String,
    pub occurrences: u32,
    pub last_seen: chrono::DateTime<chrono::Utc>,
    pub has_suggestion: bool,
}

/// Write bytes to a file atomically using write-then-rename.
/// Writes to `path.with_extension("tmp")` first, then renames to `path`.
/// This ensures that if the process crashes mid-write, only a .tmp file is left behind,
/// not a corrupted target file.
fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    let tmp = path.with_extension("tmp");
    fs::write(&tmp, bytes).with_context(|| format!("Writing temporary file {:?}", tmp))?;
    fs::rename(&tmp, path).with_context(|| format!("Renaming {:?} to {:?}", tmp, path))?;
    Ok(())
}

/// File-backed key-value store
pub struct KnowledgeStore {
    data_dir: PathBuf,
    /// In-memory cache: key → JSON string
    cache: HashMap<String, String>,
}

impl KnowledgeStore {
    pub fn open(data_dir: &Path) -> Result<Self> {
        fs::create_dir_all(data_dir)
            .with_context(|| format!("Creating data dir: {:?}", data_dir))?;
        Ok(Self {
            data_dir: data_dir.to_path_buf(),
            cache: HashMap::new(),
        })
    }

    fn key_to_path(&self, key: &str) -> PathBuf {
        // Replace ':' and '/' with safe characters for filenames
        let safe_key = key.replace([':', '/'], "_");
        self.data_dir.join(format!("{}.json", safe_key))
    }

    pub fn get<T: for<'de> Deserialize<'de>>(&mut self, key: &str) -> Result<Option<T>> {
        if let Some(cached) = self.cache.get(key) {
            return Ok(Some(serde_json::from_str(cached)?));
        }
        let path = self.key_to_path(key);
        if !path.exists() {
            return Ok(None);
        }
        let content = fs::read_to_string(&path).with_context(|| format!("Reading {:?}", path))?;
        self.cache.insert(key.to_string(), content.clone());
        Ok(Some(serde_json::from_str(&content)?))
    }

    pub fn put<T: Serialize>(&mut self, key: &str, value: &T) -> Result<()> {
        let content = serde_json::to_string_pretty(value)?;
        let path = self.key_to_path(key);
        atomic_write(&path, content.as_bytes()).with_context(|| format!("Writing {:?}", path))?;
        self.cache.insert(key.to_string(), content);
        Ok(())
    }

    pub fn delete(&mut self, key: &str) -> Result<()> {
        let path = self.key_to_path(key);
        if path.exists() {
            fs::remove_file(&path)?;
        }
        self.cache.remove(key);
        Ok(())
    }

    pub fn list_keys(&self, prefix: &str) -> Result<Vec<String>> {
        let mut keys = Vec::new();
        let safe_prefix = prefix.replace([':', '/'], "_");
        for entry in fs::read_dir(&self.data_dir)? {
            let entry = entry?;
            let fname = entry.file_name().to_string_lossy().into_owned();
            if fname.starts_with(&safe_prefix) && fname.ends_with(".json") {
                let key = fname
                    .trim_end_matches(".json")
                    .replace('_', ":")
                    .to_string();
                keys.push(key);
            }
        }
        Ok(keys)
    }

    // --- Typed accessors ---

    pub fn get_fix(&mut self, sig_hash: &str) -> Result<Option<FixRecord>> {
        self.get(&format!("{}{}", keys::FIX_PREFIX, sig_hash))
    }

    pub fn put_fix(&mut self, record: &FixRecord) -> Result<()> {
        self.put(
            &format!("{}{}", keys::FIX_PREFIX, record.signature_hash),
            record,
        )
    }

    pub fn get_pattern(&mut self, id: &str) -> Result<Option<PatternRecord>> {
        self.get(&format!("{}{}", keys::PATTERN_PREFIX, id))
    }

    pub fn put_pattern(&mut self, record: &PatternRecord) -> Result<()> {
        self.put(&format!("{}{}", keys::PATTERN_PREFIX, record.id), record)
    }

    pub fn list_patterns(&self) -> Result<Vec<String>> {
        self.list_keys(keys::PATTERN_PREFIX)
    }

    pub fn get_project(&mut self, id: &str) -> Result<Option<ProjectMeta>> {
        self.get(&format!("{}{}", keys::PROJECT_PREFIX, id))
    }

    pub fn put_project(&mut self, meta: &ProjectMeta) -> Result<()> {
        self.put(&format!("{}{}", keys::PROJECT_PREFIX, meta.id), meta)
    }

    pub fn get_error(&mut self, hash: &str) -> Result<Option<ErrorRecord>> {
        self.get(&format!("{}{}", keys::ERROR_PREFIX, hash))
    }

    pub fn put_error(&mut self, record: &ErrorRecord) -> Result<()> {
        self.put(&format!("{}{}", keys::ERROR_PREFIX, record.hash), record)
    }

    pub fn list_errors(&mut self) -> Result<Vec<ErrorRecord>> {
        let keys = self.list_keys(keys::ERROR_PREFIX)?;
        let mut out = Vec::with_capacity(keys.len());
        for key in keys {
            if let Some(rec) = self.get::<ErrorRecord>(&key)? {
                out.push(rec);
            }
        }
        Ok(out)
    }

    pub fn list_errors_summary(&self, limit: usize) -> Result<Vec<ErrorSummary>> {
        let mut summaries = Vec::new();

        // Read directory and find all error_*.json files
        for entry in fs::read_dir(&self.data_dir)? {
            let entry = entry?;
            let fname = entry.file_name().to_string_lossy().into_owned();

            // Match error_<hash>.json pattern
            if fname.starts_with("error_") && fname.ends_with(".json") {
                let path = entry.path();

                // Try to parse the error record; skip corrupt files
                match fs::read_to_string(&path) {
                    Ok(content) => {
                        match serde_json::from_str::<ErrorRecord>(&content) {
                            Ok(record) => {
                                // Check if suggestion exists
                                let suggestion_key =
                                    format!("{}{}", keys::SUGGESTION_PREFIX, record.hash);
                                let safe_sug_key = suggestion_key.replace([':', '/'], "_");
                                let sug_path = self.data_dir.join(format!("{}.json", safe_sug_key));
                                let has_suggestion = sug_path.exists();

                                summaries.push(ErrorSummary {
                                    hash: record.hash,
                                    last_command: record.last_command,
                                    occurrences: record.occurrences as u32,
                                    last_seen: record.last_seen,
                                    has_suggestion,
                                });
                            }
                            Err(_) => {
                                // Skip corrupt JSON
                                continue;
                            }
                        }
                    }
                    Err(_) => {
                        // Skip files that can't be read
                        continue;
                    }
                }
            }
        }

        // Sort by last_seen DESC
        summaries.sort_by_key(|s| std::cmp::Reverse(s.last_seen));

        // Apply limit
        summaries.truncate(limit);

        Ok(summaries)
    }

    pub fn get_suggestion(&mut self, hash: &str) -> Result<Option<String>> {
        match self.get::<SuggestionRecord>(&format!("{}{}", keys::SUGGESTION_PREFIX, hash))? {
            Some(rec) => Ok(Some(rec.text)),
            None => Ok(None),
        }
    }

    /// Convenience: load both error record and cached suggestion text for a hash.
    /// Either or both may be `None` if not stored.
    pub fn load_pair(&mut self, hash: &str) -> Result<(Option<ErrorRecord>, Option<String>)> {
        let err = self.get_error(hash)?;
        let sug = self.get_suggestion(hash)?;
        Ok((err, sug))
    }

    pub fn put_suggestion(&mut self, hash: &str, text: &str) -> Result<()> {
        let rec = SuggestionRecord {
            text: text.to_string(),
            ts: chrono::Utc::now(),
        };
        self.put(&format!("{}{}", keys::SUGGESTION_PREFIX, hash), &rec)
    }

    /// Store a feedback record to disk.
    /// Filename: feedback_<error_hash>_<unix_timestamp>.json
    /// Uses atomic_write to ensure file safety.
    pub fn put_feedback(&mut self, fb: &FeedbackRecord) -> Result<()> {
        let ts_unix = fb.ts.timestamp();
        let key = format!("{}{}_{}", keys::FEEDBACK_PREFIX, fb.error_hash, ts_unix);
        self.put(&key, fb)
    }

    /// List all feedback records in the knowledge store.
    /// Silently skips any corrupted .json files in the feedback directory.
    pub fn list_feedback(&self) -> Result<Vec<FeedbackRecord>> {
        let mut records = Vec::new();
        let safe_prefix = keys::FEEDBACK_PREFIX.replace([':', '/'], "_");

        for entry in std::fs::read_dir(&self.data_dir)? {
            let entry = entry?;
            let fname = entry.file_name().to_string_lossy().into_owned();

            if fname.starts_with(&safe_prefix) && fname.ends_with(".json") {
                let path = entry.path();
                // Try to read and deserialize; skip silently on error
                if let Ok(content) = std::fs::read_to_string(&path) {
                    if let Ok(rec) = serde_json::from_str::<FeedbackRecord>(&content) {
                        records.push(rec);
                    }
                }
            }
        }

        // Sort by timestamp descending (most recent first)
        records.sort_by_key(|r| std::cmp::Reverse(r.ts));
        Ok(records)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::Verdict;

    #[test]
    fn test_put_and_get_suggestion() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();

        let hash = "test_hash_123";
        let text = "Try adding a trait implementation.";

        store.put_suggestion(hash, text).unwrap();
        let retrieved = store.get_suggestion(hash).unwrap();
        assert_eq!(retrieved, Some(text.to_string()));
    }

    #[test]
    fn test_get_nonexistent_suggestion() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();

        let retrieved = store.get_suggestion("nonexistent").unwrap();
        assert_eq!(retrieved, None);
    }

    #[test]
    fn test_load_pair_both_present() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();
        let hash = "abc123";
        let err = ErrorRecord {
            tool: "rustc".into(),
            kind: "E0599".into(),
            hash: hash.into(),
            raw_excerpt: "no method foo".into(),
            first_seen: chrono::Utc::now(),
            last_seen: chrono::Utc::now(),
            occurrences: 1,
            last_command: "cargo build".into(),
        };
        store.put_error(&err).unwrap();
        store.put_suggestion(hash, "do the thing").unwrap();
        let (e, s) = store.load_pair(hash).unwrap();
        assert!(e.is_some());
        assert_eq!(s, Some("do the thing".to_string()));
    }

    #[test]
    fn test_load_pair_half_missing() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();
        store.put_suggestion("only_sug", "lonely").unwrap();
        let (e, s) = store.load_pair("only_sug").unwrap();
        assert!(e.is_none());
        assert_eq!(s, Some("lonely".to_string()));
    }

    #[test]
    fn test_suggestion_persistence() {
        let tmp = tempfile::TempDir::new().unwrap();
        {
            let mut store = KnowledgeStore::open(tmp.path()).unwrap();
            store
                .put_suggestion("persistent_hash", "Persist me!")
                .unwrap();
        }
        // Reopen store
        {
            let mut store = KnowledgeStore::open(tmp.path()).unwrap();
            let retrieved = store.get_suggestion("persistent_hash").unwrap();
            assert_eq!(retrieved, Some("Persist me!".to_string()));
        }
    }

    #[test]
    fn test_list_errors_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = KnowledgeStore::open(tmp.path()).unwrap();
        let summaries = store.list_errors_summary(20).unwrap();
        assert_eq!(summaries.len(), 0);
    }

    #[test]
    fn test_list_errors_sorted() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();

        // Create 3 errors with different last_seen times
        let now = chrono::Utc::now();
        let err1 = ErrorRecord {
            tool: "rustc".into(),
            kind: "E0599".into(),
            hash: "hash1".into(),
            raw_excerpt: "error1".into(),
            first_seen: now - chrono::Duration::hours(3),
            last_seen: now - chrono::Duration::hours(3),
            occurrences: 1,
            last_command: "cargo build".into(),
        };
        let err2 = ErrorRecord {
            tool: "rustc".into(),
            kind: "E0599".into(),
            hash: "hash2".into(),
            raw_excerpt: "error2".into(),
            first_seen: now - chrono::Duration::hours(1),
            last_seen: now - chrono::Duration::hours(1),
            occurrences: 2,
            last_command: "cargo test".into(),
        };
        let err3 = ErrorRecord {
            tool: "rustc".into(),
            kind: "E0599".into(),
            hash: "hash3".into(),
            raw_excerpt: "error3".into(),
            first_seen: now - chrono::Duration::minutes(30),
            last_seen: now - chrono::Duration::minutes(30),
            occurrences: 3,
            last_command: "cargo run".into(),
        };

        store.put_error(&err1).unwrap();
        store.put_error(&err2).unwrap();
        store.put_error(&err3).unwrap();

        let summaries = store.list_errors_summary(20).unwrap();
        assert_eq!(summaries.len(), 3);
        // Should be sorted by last_seen DESC (most recent first)
        assert_eq!(summaries[0].hash, "hash3");
        assert_eq!(summaries[1].hash, "hash2");
        assert_eq!(summaries[2].hash, "hash1");
    }

    #[test]
    fn test_list_errors_limit() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();

        let now = chrono::Utc::now();
        for i in 0..5 {
            let err = ErrorRecord {
                tool: "rustc".into(),
                kind: "E0599".into(),
                hash: format!("hash{}", i),
                raw_excerpt: format!("error{}", i),
                first_seen: now,
                last_seen: now - chrono::Duration::seconds(i as i64),
                occurrences: 1,
                last_command: "cargo build".into(),
            };
            store.put_error(&err).unwrap();
        }

        let summaries = store.list_errors_summary(2).unwrap();
        assert_eq!(summaries.len(), 2);
    }

    #[test]
    fn test_list_errors_has_suggestion() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();

        let now = chrono::Utc::now();
        let err = ErrorRecord {
            tool: "rustc".into(),
            kind: "E0599".into(),
            hash: "hashwithreason".into(),
            raw_excerpt: "error".into(),
            first_seen: now,
            last_seen: now,
            occurrences: 1,
            last_command: "cargo build".into(),
        };
        store.put_error(&err).unwrap();

        // Before adding suggestion
        let summaries = store.list_errors_summary(20).unwrap();
        assert_eq!(summaries.len(), 1);
        assert!(!summaries[0].has_suggestion);

        // After adding suggestion
        store
            .put_suggestion("hashwithreason", "do the thing")
            .unwrap();
        let summaries = store.list_errors_summary(20).unwrap();
        assert_eq!(summaries.len(), 1);
        assert!(summaries[0].has_suggestion);
    }

    #[test]
    fn test_list_errors_corrupt_skipped() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();

        let now = chrono::Utc::now();
        let good_err = ErrorRecord {
            tool: "rustc".into(),
            kind: "E0599".into(),
            hash: "goodhash".into(),
            raw_excerpt: "error".into(),
            first_seen: now,
            last_seen: now,
            occurrences: 1,
            last_command: "cargo build".into(),
        };
        store.put_error(&good_err).unwrap();

        // Write a corrupt error file
        let corrupt_path = tmp.path().join("error_corrupthash.json");
        fs::write(&corrupt_path, "{ invalid json }").unwrap();

        // list_errors_summary should skip the corrupt file and return only good one
        let summaries = store.list_errors_summary(20).unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].hash, "goodhash");
    }

    #[test]
    fn test_atomic_write_no_tmp_left() {
        let tmp = tempfile::TempDir::new().unwrap();
        let target = tmp.path().join("test_file.json");
        let content = b"test content";

        atomic_write(&target, content).unwrap();

        // File should exist
        assert!(target.exists());
        assert_eq!(fs::read(&target).unwrap(), content);

        // No .tmp file should be left behind
        let tmp_file = target.with_extension("tmp");
        assert!(
            !tmp_file.exists(),
            ".tmp file was not cleaned up after atomic_write"
        );
    }

    #[test]
    fn test_atomic_write_large_content() {
        let tmp = tempfile::TempDir::new().unwrap();
        let target = tmp.path().join("large_file.json");
        // Create 1MB of content
        let content = vec![b'x'; 1024 * 1024];

        let result = atomic_write(&target, &content);
        assert!(result.is_ok(), "atomic_write should handle large content");

        assert!(target.exists());
        assert_eq!(fs::read(&target).unwrap().len(), content.len());
    }

    #[test]
    fn test_put_and_get_feedback_roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();

        let now = chrono::Utc::now();
        let fb = FeedbackRecord {
            error_hash: "error123".into(),
            suggestion_hash: "sug456".into(),
            verdict: Verdict::Accepted,
            note: Some("looks good".into()),
            ts: now,
        };

        store.put_feedback(&fb).unwrap();
        let all = store.list_feedback().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].error_hash, "error123");
        assert_eq!(all[0].verdict, Verdict::Accepted);
        assert_eq!(all[0].note, Some("looks good".into()));
    }

    #[test]
    fn test_list_feedback_multiple_per_hash() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();

        let now = chrono::Utc::now();
        let hash = "sameerror";

        // First feedback
        let fb1 = FeedbackRecord {
            error_hash: hash.into(),
            suggestion_hash: "sug1".into(),
            verdict: Verdict::Rejected,
            note: None,
            ts: now - chrono::Duration::seconds(10),
        };

        // Second feedback for same error
        let fb2 = FeedbackRecord {
            error_hash: hash.into(),
            suggestion_hash: "sug2".into(),
            verdict: Verdict::Accepted,
            note: Some("better".into()),
            ts: now,
        };

        store.put_feedback(&fb1).unwrap();
        store.put_feedback(&fb2).unwrap();

        let all = store.list_feedback().unwrap();
        assert_eq!(all.len(), 2);
        // Most recent first (sorted descending)
        assert_eq!(all[0].verdict, Verdict::Accepted);
        assert_eq!(all[1].verdict, Verdict::Rejected);
    }

    #[test]
    fn test_list_feedback_empty() {
        let tmp = tempfile::TempDir::new().unwrap();
        let store = KnowledgeStore::open(tmp.path()).unwrap();

        let all = store.list_feedback().unwrap();
        assert_eq!(all.len(), 0);
    }

    #[test]
    fn test_list_feedback_corrupt_file_skipped() {
        let tmp = tempfile::TempDir::new().unwrap();
        let mut store = KnowledgeStore::open(tmp.path()).unwrap();

        // Put one valid feedback
        let now = chrono::Utc::now();
        let fb = FeedbackRecord {
            error_hash: "error_valid".into(),
            suggestion_hash: "sug_valid".into(),
            verdict: Verdict::Ignored,
            note: None,
            ts: now,
        };
        store.put_feedback(&fb).unwrap();

        // Manually create a corrupted feedback file
        let corrupt_path = tmp.path().join("feedback_error_corrupt_12345.json");
        fs::write(&corrupt_path, "{broken json").unwrap();

        // list_feedback should skip the corrupt file and return only the valid one
        let all = store.list_feedback().unwrap();
        assert_eq!(all.len(), 1);
        assert_eq!(all[0].error_hash, "error_valid");
    }
}
