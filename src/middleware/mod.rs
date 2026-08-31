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
//! HTTP middleware: correlation IDs + structured logging
//!
//! - `correlation_middleware`: extracts `X-Correlation-ID` / `X-Request-ID` or generates UUID,
//!   stores in request extensions, echoes in response headers, and injects into tracing span.
//! - `graceful_degradation`: helper to build degraded responses for partial failures.

pub mod correlation;
pub mod degradation;

pub use correlation::{correlation_middleware, CorrelationId};
pub use degradation::{degraded_response, DegradationContext};
