// Copyright 2026 Stellar-K8s Contributors
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
//! Cryptographic policy engine for runtime admission decisions (#1483).
//!
//! Policies are authored in a CEL subset, compiled into a signed bundle, and
//! verified against a configured trust root (Ed25519 keys from the existing
//! key-management / SecretPolicy inventory) before they can affect admission.
//! Invalid bundles never become active (fail-closed). Verified bundles are
//! cached in-process by digest so the signature is re-checked only when the
//! payload hash changes.

pub mod admission_guard;
pub mod cel;
pub mod engine;

pub use engine::{
    percentile_p99, AdmissionView, AdmitDecision, CanonicalBundle, Clock, CompiledBundle,
    FixedClock, PolicyEngine, PolicyEngineMetrics, SignedPolicyBundle, SystemClock, TrustRoot,
    EVAL_P99_SLO, MAX_CLOCK_SKEW, PROPAGATION_SLO,
};
