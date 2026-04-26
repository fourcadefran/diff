//! JSON-RPC 2.0 message types used between diff-agent (server) and the
//! RemoteGitBackend in src-tauri (client). Line-delimited framing: one JSON
//! object per line, terminated with '\n'.

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Message {
    Request(Request),
    Response(Response),
    Notification(Notification),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Request {
    pub jsonrpc: JsonRpcVersion,
    pub id: u64,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Response {
    pub jsonrpc: JsonRpcVersion,
    pub id: u64,
    #[serde(flatten)]
    pub outcome: ResponseOutcome,
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ResponseOutcome {
    Result(Value),
    Error(ErrorObj),
}

#[derive(Debug, Serialize, Deserialize)]
pub struct Notification {
    pub jsonrpc: JsonRpcVersion,
    pub method: String,
    #[serde(default)]
    pub params: Value,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ErrorObj {
    pub code: i32,
    pub message: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// Marker type that always serializes as the literal "2.0" required by
/// JSON-RPC 2.0. Using a typed marker (instead of `String`) catches malformed
/// frames at the deserialization boundary.
#[derive(Debug, Clone, Copy)]
pub struct JsonRpcVersion;

impl Serialize for JsonRpcVersion {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_str("2.0")
    }
}

impl<'de> Deserialize<'de> for JsonRpcVersion {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let s = String::deserialize(d)?;
        if s == "2.0" {
            Ok(JsonRpcVersion)
        } else {
            Err(serde::de::Error::custom(format!("unsupported jsonrpc version: {s}")))
        }
    }
}

// Standard JSON-RPC error codes plus our application range (-32000 to -32099).
pub mod error_codes {
    pub const PARSE_ERROR: i32 = -32700;
    pub const INVALID_REQUEST: i32 = -32600;
    pub const METHOD_NOT_FOUND: i32 = -32601;
    pub const INVALID_PARAMS: i32 = -32602;
    pub const INTERNAL_ERROR: i32 = -32603;
    pub const APPLICATION_ERROR: i32 = -32000;
}

/// Build a Response carrying a successful result.
pub fn ok_response(id: u64, result: Value) -> Response {
    Response {
        jsonrpc: JsonRpcVersion,
        id,
        outcome: ResponseOutcome::Result(result),
    }
}

/// Build a Response carrying an error.
pub fn err_response(id: u64, code: i32, message: String) -> Response {
    Response {
        jsonrpc: JsonRpcVersion,
        id,
        outcome: ResponseOutcome::Error(ErrorObj {
            code,
            message,
            data: None,
        }),
    }
}

/// Build a Notification (no id, no response expected).
pub fn notification(method: &str, params: Value) -> Notification {
    Notification {
        jsonrpc: JsonRpcVersion,
        method: method.to_string(),
        params,
    }
}
