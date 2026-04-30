//! File-based key-value store backed by JSON files.
//! Simple and portable — no native dependencies.
//! Suitable for Level 0-2 capability. Swap for RocksDB at Level 3+.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

use crate::types::{keys, ErrorRecord, FixRecord, PatternRecord, ProjectMeta, SuggestionRecord};

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
        fs::write(&path, &content).with_context(|| format!("Writing {:?}", path))?;
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

    pub fn get_suggestion(&mut self, hash: &str) -> Result<Option<String>> {
        match self.get::<SuggestionRecord>(&format!("{}{}", keys::SUGGESTION_PREFIX, hash))? {
            Some(rec) => Ok(Some(rec.text)),
            None => Ok(None),
        }
    }

    pub fn put_suggestion(&mut self, hash: &str, text: &str) -> Result<()> {
        let rec = SuggestionRecord {
            text: text.to_string(),
            ts: chrono::Utc::now(),
        };
        self.put(&format!("{}{}", keys::SUGGESTION_PREFIX, hash), &rec)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
