use anyhow::{anyhow, Context, Result};
use chrono::{DateTime, NaiveDate, Utc};
use organism_knowledge::{AcceptedSuggestion, ErrorRecord, FeedbackRecord, KnowledgeStore, StyleProfile};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::fs;

use crate::data_dir;

#[derive(Debug, Serialize, Deserialize)]
struct ExportEnvelope {
    schema_v: u32,
    exported_at: DateTime<Utc>,
    organism_version: String,
    filters: ExportFilters,
    records: ExportRecords,
    counts: ExportCounts,
}

#[derive(Debug, Serialize, Deserialize)]
struct ExportFilters {
    tool: Option<String>,
    since: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SuggestionExport {
    error_hash: String,
    text: String,
    ts: DateTime<Utc>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ExportRecords {
    errors: Vec<ErrorRecord>,
    suggestions: Vec<SuggestionExport>,
    accepted: Vec<AcceptedSuggestion>,
    feedback: Vec<FeedbackRecord>,
    style_profile: Option<StyleProfile>,
}

#[derive(Debug, Serialize, Deserialize)]
struct ExportCounts {
    errors: usize,
    suggestions: usize,
    accepted: usize,
    feedback: usize,
}

pub fn cmd_export(args: &[String]) -> Result<()> {
    let mut out_path: Option<String> = None;
    let mut tool_filter: Option<String> = None;
    let mut since_filter: Option<DateTime<Utc>> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--out" => {
                i += 1;
                if i >= args.len() {
                    return Err(anyhow!("--out requires an argument"));
                }
                out_path = Some(args[i].clone());
            }
            "--tool" => {
                i += 1;
                if i >= args.len() {
                    return Err(anyhow!("--tool requires an argument"));
                }
                tool_filter = Some(args[i].clone());
            }
            "--since" => {
                i += 1;
                if i >= args.len() {
                    return Err(anyhow!("--since requires an argument"));
                }
                since_filter = Some(parse_date(&args[i])?);
            }
            _ => {
                return Err(anyhow!("unknown argument: {}", args[i]));
            }
        }
        i += 1;
    }

    let data_dir = data_dir().join("knowledge");
    let mut store = KnowledgeStore::open(&data_dir)?;

    let all_errors = store.list_errors()?;
    let all_feedback = store.list_feedback()?;
    let all_accepted = store.list_accepted_records()?;
    let all_suggestions = store.list_suggestion_records()?;
    let style_profile = store.get_style_profile()?;

    let mut error_hashes_to_export = HashSet::new();

    for error in &all_errors {
        let passes_tool_filter = match &tool_filter {
            None => true,
            Some(t) => error.tool == *t,
        };

        let passes_since_filter = match since_filter {
            None => true,
            Some(dt) => error.last_seen >= dt,
        };

        if passes_tool_filter && passes_since_filter {
            error_hashes_to_export.insert(error.hash.clone());
        }
    }

    let exported_errors: Vec<ErrorRecord> = all_errors
        .into_iter()
        .filter(|e| error_hashes_to_export.contains(&e.hash))
        .collect();

    let feedback_for_exported: Vec<FeedbackRecord> = all_feedback
        .into_iter()
        .filter(|fb| error_hashes_to_export.contains(&fb.error_hash))
        .collect();

    let accepted_error_hashes: HashSet<String> =
        feedback_for_exported.iter().map(|fb| fb.error_hash.clone()).collect();

    let accepted_for_exported: Vec<AcceptedSuggestion> = all_accepted
        .into_iter()
        .filter(|a| accepted_error_hashes.contains(&a.error_hash))
        .collect();

    let accepted_suggestion_hashes: HashSet<String> =
        accepted_for_exported.iter().map(|a| a.suggestion_hash.clone()).collect();

    let suggestions_for_exported: Vec<SuggestionExport> = all_suggestions
        .into_iter()
        .filter(|(hash, _)| {
            error_hashes_to_export.contains(hash) || accepted_suggestion_hashes.contains(hash)
        })
        .map(|(hash, rec)| SuggestionExport {
            error_hash: hash,
            text: rec.text,
            ts: rec.ts,
        })
        .collect();

    let include_style_profile = tool_filter.is_none();

    let envelope = ExportEnvelope {
        schema_v: 1,
        exported_at: Utc::now(),
        organism_version: env!("CARGO_PKG_VERSION").to_string(),
        filters: ExportFilters {
            tool: tool_filter,
            since: since_filter,
        },
        records: ExportRecords {
            errors: exported_errors.clone(),
            suggestions: suggestions_for_exported.clone(),
            accepted: accepted_for_exported.clone(),
            feedback: feedback_for_exported.clone(),
            style_profile: if include_style_profile { style_profile } else { None },
        },
        counts: ExportCounts {
            errors: exported_errors.len(),
            suggestions: suggestions_for_exported.len(),
            accepted: accepted_for_exported.len(),
            feedback: feedback_for_exported.len(),
        },
    };

    let json = serde_json::to_string_pretty(&envelope)?;

    if let Some(path) = out_path {
        fs::write(&path, &json).context(format!("Writing export to {}", path))?;
        println!("exported to {}", path);
    } else {
        println!("{}", json);
    }

    Ok(())
}

pub fn cmd_import(args: &[String]) -> Result<()> {
    if args.is_empty() {
        return Err(anyhow!("import requires a file path as first argument"));
    }

    let file_path = &args[0];
    let mut merge_mode = true;

    for arg in &args[1..] {
        match arg.as_str() {
            "--merge" => merge_mode = true,
            "--replace" => merge_mode = false,
            _ => {
                return Err(anyhow!("unknown argument: {}", arg));
            }
        }
    }

    let json_content = fs::read_to_string(file_path)
        .context(format!("Reading import file: {}", file_path))?;

    let envelope: ExportEnvelope = serde_json::from_str(&json_content)
        .context("Parsing import file as JSON")?;

    if envelope.schema_v != 1 {
        return Err(anyhow!(
            "unsupported schema version: {} (only schema_v=1 is supported)",
            envelope.schema_v
        ));
    }

    let data_dir = data_dir().join("knowledge");
    let mut store = KnowledgeStore::open(&data_dir)?;

    let mut imported_errors = 0usize;
    let mut imported_suggestions = 0usize;
    let mut imported_accepted = 0usize;
    let mut imported_feedback = 0usize;
    let mut skipped = 0usize;

    for error in envelope.records.errors {
        let key = format!("error:{}", error.hash);
        let existing = store.get::<ErrorRecord>(&key)?;

        if existing.is_some() && merge_mode {
            skipped += 1;
        } else {
            store.put_error(&error)?;
            imported_errors += 1;
        }
    }

    for sugg in envelope.records.suggestions {
        let key = format!("suggestion:{}", sugg.error_hash);
        let existing = store.get::<organism_knowledge::SuggestionRecord>(&key)?;

        if existing.is_some() && merge_mode {
            skipped += 1;
        } else {
            store.put_suggestion(&sugg.error_hash, &sugg.text)?;
            imported_suggestions += 1;
        }
    }

    for accepted in envelope.records.accepted {
        let key = format!("accepted:{}", accepted.suggestion_hash);
        let existing = store.get::<AcceptedSuggestion>(&key)?;

        if existing.is_some() && merge_mode {
            skipped += 1;
        } else {
            store.put_accepted(&accepted)?;
            imported_accepted += 1;
        }
    }

    for feedback in envelope.records.feedback {
        let ts_unix = feedback.ts.timestamp();
        let key = format!("feedback:{}_{}", feedback.error_hash, ts_unix);
        let existing = store.get::<FeedbackRecord>(&key)?;

        if existing.is_some() && merge_mode {
            skipped += 1;
        } else {
            store.put_feedback(&feedback)?;
            imported_feedback += 1;
        }
    }

    if let Some(style_profile) = envelope.records.style_profile {
        let existing = store.get_style_profile()?;
        if existing.is_some() && merge_mode {
            skipped += 1;
        } else {
            store.put_style_profile(&style_profile)?;
        }
    }

    println!(
        "imported {} errors, {} suggestions, {} accepted, {} feedback ({} skipped)",
        imported_errors, imported_suggestions, imported_accepted, imported_feedback, skipped
    );

    Ok(())
}

fn parse_date(date_str: &str) -> Result<DateTime<Utc>> {
    if date_str.contains('T') {
        DateTime::parse_from_rfc3339(date_str)
            .map(|dt| dt.with_timezone(&Utc))
            .context(format!("Parsing RFC3339 date: {}", date_str))
    } else {
        NaiveDate::parse_from_str(date_str, "%Y-%m-%d")
            .context(format!("Parsing date: {}", date_str))
            .and_then(|nd| {
                Ok(nd
                    .and_hms_opt(0, 0, 0)
                    .ok_or_else(|| anyhow!("Invalid date: {}", date_str))?
                    .and_utc())
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use organism_knowledge::Verdict;

    #[test]
    fn roundtrip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().join("knowledge");
        fs::create_dir_all(&data_dir).unwrap();
        let mut store = KnowledgeStore::open(&data_dir).unwrap();

        let now = Utc::now();

        let err1 = ErrorRecord {
            tool: "rustc".to_string(),
            kind: "E0599".to_string(),
            hash: "hash1".to_string(),
            raw_excerpt: "error[E0599]".to_string(),
            first_seen: now,
            last_seen: now,
            occurrences: 5,
            last_command: "cargo build".to_string(),
            schema_v: 1,
        };

        let err2 = ErrorRecord {
            tool: "cargo".to_string(),
            kind: "build-failed".to_string(),
            hash: "hash2".to_string(),
            raw_excerpt: "build failed".to_string(),
            first_seen: now,
            last_seen: now,
            occurrences: 2,
            last_command: "cargo test".to_string(),
            schema_v: 1,
        };

        store.put_error(&err1).unwrap();
        store.put_error(&err2).unwrap();

        let fb1 = FeedbackRecord {
            error_hash: "hash1".to_string(),
            suggestion_hash: "sugg1".to_string(),
            verdict: Verdict::Accepted,
            note: None,
            ts: now,
            schema_v: 1,
        };

        let fb2 = FeedbackRecord {
            error_hash: "hash2".to_string(),
            suggestion_hash: "sugg2".to_string(),
            verdict: Verdict::Rejected,
            note: Some("not applicable".to_string()),
            ts: now,
            schema_v: 1,
        };

        store.put_feedback(&fb1).unwrap();
        store.put_feedback(&fb2).unwrap();

        let accepted = AcceptedSuggestion {
            suggestion_hash: "sugg1".to_string(),
            error_hash: "hash1".to_string(),
            text: "Try adding derive(Clone)".to_string(),
            ts: now,
            schema_v: 1,
        };

        store.put_accepted(&accepted).unwrap();

        let tmp_export = tempfile::TempDir::new().unwrap();
        let export_file = tmp_export.path().join("export.json");

        let old_home = std::env::var("ORGANISM_HOME").ok();
        std::env::set_var("ORGANISM_HOME", tmp.path());
        cmd_export(&["--out".to_string(), export_file.to_string_lossy().to_string()]).unwrap();
        if let Some(h) = old_home {
            std::env::set_var("ORGANISM_HOME", h);
        } else {
            std::env::remove_var("ORGANISM_HOME");
        }

        let json_content = fs::read_to_string(&export_file).unwrap();
        let envelope: ExportEnvelope = serde_json::from_str(&json_content).unwrap();

        assert_eq!(envelope.schema_v, 1);
        assert_eq!(envelope.counts.errors, 2);
        assert_eq!(envelope.counts.feedback, 2);
        assert_eq!(envelope.counts.accepted, 1);

        let tmp_import = tempfile::TempDir::new().unwrap();
        let import_data_dir = tmp_import.path().join("knowledge");
        fs::create_dir_all(&import_data_dir).unwrap();

        let old_home = std::env::var("ORGANISM_HOME").ok();
        std::env::set_var("ORGANISM_HOME", tmp_import.path());
        cmd_import(&[export_file.to_string_lossy().to_string()]).unwrap();
        if let Some(h) = old_home {
            std::env::set_var("ORGANISM_HOME", h);
        } else {
            std::env::remove_var("ORGANISM_HOME");
        }

        let mut import_store = KnowledgeStore::open(&import_data_dir).unwrap();
        let imported_errors = import_store.list_errors().unwrap();
        let imported_feedback = import_store.list_feedback().unwrap();
        let imported_accepted = import_store.list_accepted_records().unwrap();

        assert_eq!(imported_errors.len(), 2);
        assert_eq!(imported_feedback.len(), 2);
        assert_eq!(imported_accepted.len(), 1);

        let tmp_export2 = tempfile::TempDir::new().unwrap();
        let export_file2 = tmp_export2.path().join("export2.json");

        let old_home = std::env::var("ORGANISM_HOME").ok();
        std::env::set_var("ORGANISM_HOME", tmp_import.path());
        cmd_export(&["--out".to_string(), export_file2.to_string_lossy().to_string()]).unwrap();
        if let Some(h) = old_home {
            std::env::set_var("ORGANISM_HOME", h);
        } else {
            std::env::remove_var("ORGANISM_HOME");
        }

        let json_content2 = fs::read_to_string(&export_file2).unwrap();
        let envelope2: ExportEnvelope = serde_json::from_str(&json_content2).unwrap();

        assert_eq!(envelope2.counts.errors, 2);
        assert_eq!(envelope2.counts.feedback, 2);
        assert_eq!(envelope2.counts.accepted, 1);
    }

    #[test]
    fn filter_tool() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().join("knowledge");
        fs::create_dir_all(&data_dir).unwrap();
        let mut store = KnowledgeStore::open(&data_dir).unwrap();

        let now = Utc::now();

        let cargo_err = ErrorRecord {
            tool: "cargo".to_string(),
            kind: "build-failed".to_string(),
            hash: "cargo_hash".to_string(),
            raw_excerpt: "build failed".to_string(),
            first_seen: now,
            last_seen: now,
            occurrences: 1,
            last_command: "cargo build".to_string(),
            schema_v: 1,
        };

        let rustc_err = ErrorRecord {
            tool: "rustc".to_string(),
            kind: "E0599".to_string(),
            hash: "rustc_hash".to_string(),
            raw_excerpt: "error[E0599]".to_string(),
            first_seen: now,
            last_seen: now,
            occurrences: 1,
            last_command: "cargo build".to_string(),
            schema_v: 1,
        };

        store.put_error(&cargo_err).unwrap();
        store.put_error(&rustc_err).unwrap();

        let tmp_export = tempfile::TempDir::new().unwrap();
        let export_file = tmp_export.path().join("export.json");

        let old_home = std::env::var("ORGANISM_HOME").ok();
        std::env::set_var("ORGANISM_HOME", tmp.path());
        cmd_export(&[
            "--tool".to_string(),
            "cargo".to_string(),
            "--out".to_string(),
            export_file.to_string_lossy().to_string(),
        ])
        .unwrap();
        if let Some(h) = old_home {
            std::env::set_var("ORGANISM_HOME", h);
        } else {
            std::env::remove_var("ORGANISM_HOME");
        }

        let json_content = fs::read_to_string(&export_file).unwrap();
        let envelope: ExportEnvelope = serde_json::from_str(&json_content).unwrap();

        assert_eq!(envelope.counts.errors, 1);
        assert_eq!(envelope.records.errors[0].tool, "cargo");
    }

    #[test]
    fn import_merge_skip() {
        let tmp = tempfile::TempDir::new().unwrap();
        let data_dir = tmp.path().join("knowledge");
        fs::create_dir_all(&data_dir).unwrap();
        let mut store = KnowledgeStore::open(&data_dir).unwrap();

        let now = Utc::now();

        let err1 = ErrorRecord {
            tool: "rustc".to_string(),
            kind: "E0599".to_string(),
            hash: "hash1".to_string(),
            raw_excerpt: "error[E0599]".to_string(),
            first_seen: now,
            last_seen: now,
            occurrences: 1,
            last_command: "cargo build".to_string(),
            schema_v: 1,
        };

        store.put_error(&err1).unwrap();

        let tmp_export = tempfile::TempDir::new().unwrap();
        let export_file = tmp_export.path().join("export.json");

        let old_home = std::env::var("ORGANISM_HOME").ok();
        std::env::set_var("ORGANISM_HOME", tmp.path());
        cmd_export(&["--out".to_string(), export_file.to_string_lossy().to_string()]).unwrap();
        if let Some(h) = old_home {
            std::env::set_var("ORGANISM_HOME", h);
        } else {
            std::env::remove_var("ORGANISM_HOME");
        }

        let first_import_output = std::panic::catch_unwind(|| {
            let old_home = std::env::var("ORGANISM_HOME").ok();
            std::env::set_var("ORGANISM_HOME", tmp.path());
            let result = cmd_import(&[export_file.to_string_lossy().to_string()]);
            if let Some(h) = old_home {
                std::env::set_var("ORGANISM_HOME", h);
            } else {
                std::env::remove_var("ORGANISM_HOME");
            }
            result
        });

        assert!(first_import_output.is_ok());

        let second_import = std::panic::catch_unwind(|| {
            let old_home = std::env::var("ORGANISM_HOME").ok();
            std::env::set_var("ORGANISM_HOME", tmp.path());
            let result = cmd_import(&[export_file.to_string_lossy().to_string()]);
            if let Some(h) = old_home {
                std::env::set_var("ORGANISM_HOME", h);
            } else {
                std::env::remove_var("ORGANISM_HOME");
            }
            result
        });

        assert!(second_import.is_ok());
    }

    #[test]
    fn import_bad_schema_v() {
        let bad_envelope = ExportEnvelope {
            schema_v: 99,
            exported_at: Utc::now(),
            organism_version: "0.1.0".to_string(),
            filters: ExportFilters {
                tool: None,
                since: None,
            },
            records: ExportRecords {
                errors: vec![],
                suggestions: vec![],
                accepted: vec![],
                feedback: vec![],
                style_profile: None,
            },
            counts: ExportCounts {
                errors: 0,
                suggestions: 0,
                accepted: 0,
                feedback: 0,
            },
        };

        let tmp_export = tempfile::TempDir::new().unwrap();
        let export_file = tmp_export.path().join("export.json");
        let json = serde_json::to_string_pretty(&bad_envelope).unwrap();
        fs::write(&export_file, json).unwrap();

        let result = cmd_import(&[export_file.to_string_lossy().to_string()]);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("schema version"));
    }

    #[test]
    fn import_missing_file() {
        let result = cmd_import(&["/nonexistent/path/to/file.json".to_string()]);
        assert!(result.is_err());
    }
}
