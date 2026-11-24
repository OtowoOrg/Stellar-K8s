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
//! Reconcile `StellarPolicyBundle` specs into the in-process policy engine.

use crate::crd::StellarPolicyBundleSpec;
use crate::error::Result;
use crate::policy_engine::PolicyEngine;

/// Apply a CRD spec to the admission policy engine.
///
/// On cryptographic failure the engine stays on the last-known-good bundle
/// (or remain fail-closed if none exists). When `spec.rollbackTarget` is set
/// to the previously active hash/version, a rollback is performed instead of
/// loading the new payload.
pub fn reconcile_policy_bundle(
    engine: &PolicyEngine,
    spec: &StellarPolicyBundleSpec,
) -> Result<String> {
    if spec.rollback_target.is_some() {
        return engine.rollback();
    }
    engine.load_spec(spec)
}
