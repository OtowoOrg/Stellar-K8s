// Copyright 2024 Stellar-K8s Contributors
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.
//! Protocol transformation: REST ↔ gRPC ↔ GraphQL request/response adapters.

use crate::api_gateway::config::Protocol;
use serde_json::Value;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum TransformError {
    #[error("unsupported protocol transformation: {0:?} → {1:?}")]
    Unsupported(Protocol, Protocol),
    #[error("serialization error: {0}")]
    Serialization(#[from] serde_json::Error),
    #[error("transform error: {0}")]
    Other(String),
}

/// Normalised representation of a gateway request body.
#[derive(Debug, Clone)]
pub struct NormalizedRequest {
    pub body: Value,
    pub content_type: String,
}

/// Normalised representation of an upstream response body.
#[derive(Debug, Clone)]
pub struct NormalizedResponse {
    pub body: Value,
    pub content_type: String,
    pub status: u16,
}

/// Transform an incoming request body from `from_protocol` into the format
/// expected by the upstream `to_protocol`.
pub fn transform_request(
    raw_body: &[u8],
    from: &Protocol,
    to: &Protocol,
) -> Result<NormalizedRequest, TransformError> {
    match (from, to) {
        // REST → REST: pass through, just parse JSON
        (Protocol::Rest, Protocol::Rest) => {
            let body: Value = if raw_body.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(raw_body).map_err(TransformError::Serialization)?
            };
            Ok(NormalizedRequest {
                body,
                content_type: "application/json".into(),
            })
        }
        // GraphQL → REST: unwrap the `query` field and map to REST params
        (Protocol::GraphQL, Protocol::Rest) => {
            let gql: Value = serde_json::from_slice(raw_body)?;
            let query = gql
                .get("query")
                .and_then(Value::as_str)
                .unwrap_or("")
                .to_string();
            let variables = gql.get("variables").cloned().unwrap_or(Value::Null);
            Ok(NormalizedRequest {
                body: serde_json::json!({ "query": query, "variables": variables }),
                content_type: "application/json".into(),
            })
        }
        // gRPC → REST: decode JSON-encoded protobuf body (grpc-gateway style)
        (Protocol::Grpc, Protocol::Rest) => {
            let body: Value = if raw_body.is_empty() {
                Value::Null
            } else {
                serde_json::from_slice(raw_body).map_err(TransformError::Serialization)?
            };
            Ok(NormalizedRequest {
                body,
                content_type: "application/json".into(),
            })
        }
        _ => Err(TransformError::Unsupported(from.clone(), to.clone())),
    }
}

/// Transform an upstream response body from `upstream_protocol` back to the
/// format expected by the client (`client_protocol`).
pub fn transform_response(
    upstream_body: &[u8],
    upstream_protocol: &Protocol,
    client_protocol: &Protocol,
    status: u16,
) -> Result<NormalizedResponse, TransformError> {
    let body: Value = if upstream_body.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(upstream_body).unwrap_or(Value::Null)
    };

    match (upstream_protocol, client_protocol) {
        (Protocol::Rest, Protocol::GraphQL) => {
            // Wrap REST response in a GraphQL `data` envelope
            Ok(NormalizedResponse {
                body: serde_json::json!({ "data": body }),
                content_type: "application/json".into(),
                status,
            })
        }
        _ => Ok(NormalizedResponse {
            body,
            content_type: "application/json".into(),
            status,
        }),
    }
}
