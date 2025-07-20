use crate::errors::AnyResult;
use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Legacy response format for backward compatibility
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LegacyResponse {
    pub status: u16,
    pub headers: Value,
    pub body: String,
}

impl LegacyResponse {
    pub fn json(&self) -> AnyResult<Value> {
        serde_json::from_str(&self.body).map_err(Into::into)
    }
}
