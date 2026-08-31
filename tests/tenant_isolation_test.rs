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
use stellar_k8s::crd::{TenantQuotaHard, TenantSpec};

fn tenant_spec() -> TenantSpec {
    TenantSpec {
        tenant_id: "acme".to_string(),
        namespace: "tenant-acme".to_string(),
        network: None,
        quota: TenantQuotaHard {
            cpu: Some("2".to_string()),
            memory: Some("4Gi".to_string()),
        },
        billing: None,
        cleanup_on_delete: true,
    }
}

#[test]
fn tenant_manifests_enforce_quota_and_namespace_isolation() {
    let spec = tenant_spec();
    let labels = spec.namespace_labels();
    assert_eq!(
        labels.get("tenant.stellar.org/id"),
        Some(&"acme".to_string())
    );

    let quota = spec.resource_quota_manifest();
    assert_eq!(quota["metadata"]["namespace"], "tenant-acme");
    assert_eq!(quota["spec"]["hard"]["limits.cpu"], "2");
    assert_eq!(quota["spec"]["hard"]["requests.memory"], "4Gi");

    let policy = spec.network_policy_manifest();
    assert_eq!(policy["spec"]["policyTypes"][0], "Ingress");
    assert_eq!(policy["spec"]["policyTypes"][1], "Egress");
    assert_eq!(
        policy["spec"]["ingress"][0]["from"][0]["namespaceSelector"]["matchLabels"]
            ["tenant.stellar.org/id"],
        "acme"
    );
}
