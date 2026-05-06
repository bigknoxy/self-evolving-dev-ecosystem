use anyhow::Result;
use organism_knowledge::{AcceptedSuggestion, KnowledgeStore, Verdict};

use crate::data_dir;

pub async fn cmd_backfill_accepts() -> Result<()> {
    let mut store = KnowledgeStore::open(&data_dir())?;
    let feedback = store.list_feedback()?;
    let total = feedback.len();
    let mut snapped = 0u32;
    let mut skipped_already = 0u32;
    let mut skipped_no_text = 0u32;
    let mut errored = 0u32;

    for fb in feedback {
        if !matches!(fb.verdict, Verdict::Accepted) {
            continue;
        }
        match store.get_accepted(&fb.suggestion_hash) {
            Ok(Some(_)) => {
                skipped_already += 1;
                continue;
            }
            Err(e) => {
                println!(
                    "warning: failed to check existing accepted suggestion for {}: {}",
                    fb.suggestion_hash, e
                );
                errored += 1;
                continue;
            }
            Ok(None) => {}
        }
        match store.get_suggestion(&fb.error_hash) {
            Ok(Some(text)) => {
                match store.put_accepted(&AcceptedSuggestion::from_feedback(&fb, text)) {
                    Ok(()) => snapped += 1,
                    Err(e) => {
                        println!(
                            "warning: failed to snapshot accepted suggestion {}: {}",
                            fb.suggestion_hash, e
                        );
                        errored += 1;
                    }
                }
            }
            Ok(None) => skipped_no_text += 1,
            Err(e) => {
                println!(
                    "warning: failed to get suggestion text for {}: {}",
                    fb.error_hash, e
                );
                errored += 1;
            }
        }
    }

    println!("backfill-accepts: scanned {} feedback records", total);
    println!("  snapshotted:    {}", snapped);
    println!("  already exists: {}", skipped_already);
    println!("  no source text: {}", skipped_no_text);
    println!("  errored:        {}", errored);
    Ok(())
}
