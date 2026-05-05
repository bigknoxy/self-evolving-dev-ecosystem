use anyhow::Result;
use crate::ErrorRecord;

/// Handles migration of `ErrorRecord` deserialized from external formats.
///
/// This function attempts to deserialize the raw JSON value into an `ErrorRecord`.
/// It then checks the `schema_v` field to ensure backward compatibility.
/// Currently, this client supports up to schema v1. If the incoming record
/// specifies a higher version, it returns an error to prevent data corruption
/// or logic errors in future schema updates.
///
/// In the future, this function can be extended with explicit migration steps
/// for versions like `schema_v = 2`, `schema_v = 3`, etc., by adding match
/// arms or version-specific transformation logic before deserialization.
pub fn migrate_error(raw: serde_json::Value) -> Result<ErrorRecord> {
    let record: ErrorRecord = serde_json::from_value(raw)?;

    let schema_v = record.schema_v;
    if schema_v > 1 {
        return anyhow::bail!(
            "Unsupported schema version {}: this client supports up to v1",
            schema_v
        );
    }

    Ok(record)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_v1_record_roundtrips() {
        let raw = serde_json::json!({
            "tool": "rustc",
            "kind": "compiler_error",
            "hash": "abc123def",
            "raw_excerpt": "error: expected type",
            "first_seen": "2023-01-01T00:00:00Z",
            "last_seen": "2023-01-02T12:00:00Z",
            "occurrences": 42,
            "last_command": "cargo build",
            "schema_v": 1
        });

        let record = migrate_error(raw).expect("v1 record should migrate successfully");
        assert_eq!(record.schema_v, 1);
        assert_eq!(record.tool, "rustc");
        assert_eq!(record.occurrences, 42);
    }

    #[test]
    fn test_high_schema_version_fails() {
        let raw = serde_json::json!({
            "tool": "rustc",
            "kind": "compiler_error",
            "hash": "abc123def",
            "raw_excerpt": "error: expected type",
            "first_seen": "2023-01-01T00:00:00Z",
            "last_seen": "2023-01-02T12:00:00Z",
            "occurrences": 42,
            "last_command": "cargo build",
            "schema_v": 99
        });

        let err = migrate_error(raw).expect_err("v99 record should fail migration");
        let err_msg = err.to_string();
        assert!(
            err_msg.contains("99"),
            "Error message should mention version 99, got: {}",
            err_msg
        );
        assert!(err_msg.contains("Unsupported schema version"));
    }
}
