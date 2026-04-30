use crate::events::OrganismEvent;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Protocol version. Increment when breaking changes are made.
pub const PROTOCOL_VERSION: u8 = 1;

/// Versioned envelope wrapping all messages
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub v: u8,
    #[serde(rename = "type")]
    pub msg_type: MessageType,
    pub id: String,
    pub ts: DateTime<Utc>,
    pub payload: serde_json::Value,
}

impl Envelope {
    pub fn new(msg_type: MessageType, payload: serde_json::Value) -> Self {
        Self {
            v: PROTOCOL_VERSION,
            msg_type,
            id: Uuid::new_v4().to_string(),
            ts: Utc::now(),
            payload,
        }
    }

    pub fn request(method: &str, params: serde_json::Value) -> Self {
        Self::new(
            MessageType::Request,
            serde_json::json!({ "method": method, "params": params }),
        )
    }

    pub fn ok_response(request_id: &str, result: serde_json::Value) -> Self {
        let mut env = Self::new(
            MessageType::Response,
            serde_json::json!({ "result": result }),
        );
        env.id = request_id.to_string();
        env
    }

    pub fn error_response(request_id: &str, message: &str) -> Self {
        let mut env = Self::new(MessageType::Error, serde_json::json!({ "error": message }));
        env.id = request_id.to_string();
        env
    }

    pub fn event(event: &OrganismEvent) -> anyhow::Result<Self> {
        Ok(Self::new(MessageType::Event, serde_json::to_value(event)?))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub enum MessageType {
    Request,
    Response,
    Error,
    Event,
    Heartbeat,
}

/// Status response payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StatusResponse {
    pub status: String,
    pub uptime_s: u64,
    pub trust_level: String,
    pub active_sensors: Vec<String>,
    pub events_processed: u64,
}

/// Suggest request payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestRequest {
    pub error_key: Option<String>,
}

/// Suggest response payload
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuggestResponse {
    pub text: String,
    pub cached: bool,
}
