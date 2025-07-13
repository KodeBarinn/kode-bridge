use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::convert::From;

use crate::errors::{AnyError, AnyResult};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Response {
    pub status: u16,
    pub headers: serde_json::Value,
    pub body: String,
}

impl Response {
    pub fn json(&self) -> AnyResult<Value> {
        serde_json::from_str(&self.body).map_err(AnyError::from)
    }
}
