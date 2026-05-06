use crate::ErrorRecord;
use anyhow::Result;

/// Highest schema version this build understands.
/// Bump when introducing a new on-disk format and add a migration arm below.
pub const MAX_SUPPORTED_SCHEMA_V: u32 = 1;

/// Migrate a raw `ErrorRecord` JSON value to the in-memory `ErrorRecord` type.
///
/// Rules:
/// * `schema_v` missing → defaulted to 1 by serde (legacy unversioned writer).
/// * `schema_v == 0` → rejected. Zero is never written by any released build;
///   seeing it implies a corrupt or hand-edited file.
/// * `schema_v` in `1..=MAX_SUPPORTED_SCHEMA_V` → accepted.
/// * `schema_v > MAX_SUPPORTED_SCHEMA_V` → rejected; written by a newer client.
pub fn migrate_error(raw: serde_json::Value) -> Result<ErrorRecord> {
    let record: ErrorRecord = serde_json::from_value(raw)?;

    let schema_v = record.schema_v;
    if schema_v == 0 {
        anyhow::bail!("Invalid schema version 0: corrupt or hand-edited record");
    }
    if schema_v > MAX_SUPPORTED_SCHEMA_V {
        anyhow::bail!(
            "Unsupported schema version {}: this client supports up to v{}",
            schema_v,
            MAX_SUPPORTED_SCHEMA_V
        );
    }

    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn base_v1() -> serde_json::Value {
        serde_json::json!({
            "tool": "rustc",
            "kind": "compiler_error",
            "hash": "abc123def",
            "raw_excerpt": "error: expected type",
            "first_seen": "2023-01-01T00:00:00Z",
            "last_seen": "2023-01-02T12:00:00Z",
            "occurrences": 42,
            "last_command": "cargo build",
            "schema_v": 1
        })
    }

    #[test]
    fn test_v1_record_roundtrips() {
        let record = migrate_error(base_v1()).expect("v1 record should migrate successfully");
        assert_eq!(record.schema_v, 1);
        assert_eq!(record.tool, "rustc");
        assert_eq!(record.occurrences, 42);
    }

    #[test]
    fn test_high_schema_version_fails() {
        let mut raw = base_v1();
        raw["schema_v"] = serde_json::json!(99);

        let err = migrate_error(raw).expect_err("v99 record should fail migration");
        let err_msg = err.to_string();
        assert!(err_msg.contains("99"), "got: {}", err_msg);
        assert!(err_msg.contains("Unsupported schema version"));
    }

    #[test]
    fn test_zero_schema_version_fails() {
        let mut raw = base_v1();
        raw["schema_v"] = serde_json::json!(0);

        let err = migrate_error(raw).expect_err("v0 record should fail migration");
        assert!(err.to_string().contains("Invalid schema version 0"));
    }

    #[test]
    fn test_missing_schema_v_defaults_to_1() {
        let mut raw = base_v1();
        raw.as_object_mut().unwrap().remove("schema_v");

        let record = migrate_error(raw).expect("missing schema_v should default to 1");
        assert_eq!(record.schema_v, 1);
    }
}
