//! Tenant lifecycle reconciliation with namespace isolation, resource quotas, and network policies.
//!
//! This module implements the core multi-tenancy enforcement for Stellar-K8s:
//! 1. Tenant namespace creation and labeling
//! 2. Resource quota enforcement (CPU, memory, storage)
//! 3. Network policy for tenant isolation (ingress/egress)
//! 4. RBAC role and rolebinding setup per tenant

use anyhow::Result;
use k8s_openapi::api::core::v1::{Namespace, ResourceQuota, ResourceQuotaSpec};
use k8s_openapi::api::networking::v1::{
    NetworkPolicy, NetworkPolicyIngressRule, NetworkPolicyPeer, NetworkPolicySpec,
};
use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
use k8s_openapi::apimachinery::pkg::apis::meta::v1::{LabelSelector, ObjectMeta};
use kube::api::{Patch, PatchParams, PostParams};
use kube::{Api, Client};
use std::collections::BTreeMap;
use tracing::{info, warn};

use crate::crd::TenantSpec;

/// Reconcile tenant lifecycle: create/update/delete namespace and isolation policies
pub async fn reconcile_tenant(tenant_spec: &TenantSpec, client: &Client) -> Result<()> {
    let namespace_name = &tenant_spec.namespace;
    let tenant_id = &tenant_spec.tenant_id;

    info!(
        tenant = %tenant_id,
        namespace = %namespace_name,
        "Reconciling tenant"
    );

    // 1. Create or update namespace with tenant labels
    create_or_update_namespace(tenant_spec, client).await?;

    // 2. Apply resource quota
    apply_resource_quota(tenant_spec, client).await?;

    // 3. Apply network policies for isolation
    apply_network_policies(tenant_spec, client).await?;

    // 4. Set up RBAC (rolebindings scoped to tenant)
    setup_rbac(tenant_spec, client).await?;

    info!(
        tenant = %tenant_id,
        namespace = %namespace_name,
        "Tenant reconciliation complete"
    );

    Ok(())
}

/// Create or update namespace with tenant labels and annotations
async fn create_or_update_namespace(tenant_spec: &TenantSpec, client: &Client) -> Result<()> {
    let ns_api: Api<Namespace> = Api::all(client.clone());

    let mut labels = tenant_spec.namespace_labels();
    labels.insert(
        "app.kubernetes.io/managed-by".to_string(),
        "stellar-operator".to_string(),
    );

    let namespace = Namespace {
        metadata: ObjectMeta {
            name: Some(tenant_spec.namespace.clone()),
            labels: Some(labels.clone()),
            ..Default::default()
        },
        ..Default::default()
    };

    match ns_api.get_opt(&tenant_spec.namespace).await? {
        Some(mut existing) => {
            // Update existing namespace with labels
            if let Some(ref mut ns_labels) = existing.metadata.labels {
                ns_labels.extend(labels);
            } else {
                existing.metadata.labels = Some(labels);
            }
            let label_patch = serde_json::json!({
                "apiVersion": "v1",
                "kind": "Namespace",
                "metadata": {
                    "name": tenant_spec.namespace,
                    "labels": existing.metadata.labels,
                }
            });
            ns_api
                .patch(
                    &tenant_spec.namespace,
                    &PatchParams::apply("stellar-operator"),
                    &Patch::Apply(&label_patch),
                )
                .await?;
            info!(namespace = %tenant_spec.namespace, "Updated existing namespace");
        }
        None => {
            ns_api
                .create(&PostParams::default(), &namespace)
                .await?;
            info!(namespace = %tenant_spec.namespace, "Created namespace");
        }
    }

    Ok(())
}

/// Apply ResourceQuota to enforce tenant resource limits
async fn apply_resource_quota(tenant_spec: &TenantSpec, client: &Client) -> Result<()> {
    let quota_api: Api<ResourceQuota> = Api::namespaced(client.clone(), &tenant_spec.namespace);

    let quota_name = format!("{}-quota", tenant_spec.tenant_id);
    let mut hard = BTreeMap::new();

    // Set CPU limits
    if let Some(cpu) = &tenant_spec.quota.cpu {
        hard.insert("requests.cpu".to_string(), Quantity(cpu.clone()));
        hard.insert("limits.cpu".to_string(), Quantity(cpu.clone()));
    }

    // Set memory limits
    if let Some(mem) = &tenant_spec.quota.memory {
        hard.insert("requests.memory".to_string(), Quantity(mem.clone()));
        hard.insert("limits.memory".to_string(), Quantity(mem.clone()));
    }

    // Set pod count limit
    hard.insert("pods".to_string(), Quantity("1000".to_string()));
    hard.insert("requests.storage".to_string(), Quantity("100Gi".to_string()));

    let quota = ResourceQuota {
        metadata: ObjectMeta {
            name: Some(quota_name.clone()),
            namespace: Some(tenant_spec.namespace.clone()),
            ..Default::default()
        },
        spec: Some(ResourceQuotaSpec {
            hard: Some(hard),
            ..Default::default()
        }),
        ..Default::default()
    };

    match quota_api.get_opt(&quota_name).await? {
        Some(_) => {
            quota_api
                .replace(&quota_name, &PostParams::default(), &quota)
                .await?;
            info!(quota = %quota_name, namespace = %tenant_spec.namespace, "Updated ResourceQuota");
        }
        None => {
            quota_api
                .create(&PostParams::default(), &quota)
                .await?;
            info!(quota = %quota_name, namespace = %tenant_spec.namespace, "Created ResourceQuota");
        }
    }

    Ok(())
}

/// Apply NetworkPolicy to enforce tenant isolation (deny all, allow same-tenant)
async fn apply_network_policies(tenant_spec: &TenantSpec, client: &Client) -> Result<()> {
    let policy_api: Api<NetworkPolicy> = Api::namespaced(client.clone(), &tenant_spec.namespace);

    let tenant_id = &tenant_spec.tenant_id;

    // Deny ingress from other tenants, allow from same tenant
    let ingress_rule = NetworkPolicyIngressRule {
        from: Some(vec![NetworkPolicyPeer {
            namespace_selector: Some(LabelSelector {
                match_labels: Some({
                    let mut labels = BTreeMap::new();
                    labels.insert("tenant.stellar.org/id".to_string(), tenant_id.clone());
                    labels
                }),
                match_expressions: None,
            }),
            ..Default::default()
        }]),
        ports: None,
    };

    // Deny egress to other tenants, allow to same tenant and external APIs.
    // TODO(tenant-egress): scope egress to same-tenant + external APIs instead of
    // the fully restrictive empty rule set below.

    let policy = NetworkPolicy {
        metadata: ObjectMeta {
            name: Some(format!("{}-isolation", tenant_id)),
            namespace: Some(tenant_spec.namespace.clone()),
            ..Default::default()
        },
        spec: Some(NetworkPolicySpec {
            policy_types: Some(vec!["Ingress".to_string(), "Egress".to_string()]),
            ingress: Some(vec![ingress_rule]),
            egress: Some(vec![]), // Restrictive: no external traffic by default
            pod_selector: Default::default(),
        }),
        ..Default::default()
    };

    let policy_name = format!("{}-isolation", tenant_id);

    match policy_api.get_opt(&policy_name).await? {
        Some(_) => {
            policy_api
                .replace(&policy_name, &PostParams::default(), &policy)
                .await?;
            info!(policy = %policy_name, namespace = %tenant_spec.namespace, "Updated NetworkPolicy");
        }
        None => {
            policy_api
                .create(&PostParams::default(), &policy)
                .await?;
            info!(policy = %policy_name, namespace = %tenant_spec.namespace, "Created NetworkPolicy");
        }
    }

    Ok(())
}

/// Set up RBAC: create tenant-scoped role and rolebinding
async fn setup_rbac(tenant_spec: &TenantSpec, client: &Client) -> Result<()> {
    // TODO(tenant-rbac): Implement role and rolebinding creation per tenant
    // This will:
    // 1. Create a Role scoped to tenant resources
    // 2. Create a RoleBinding for tenant service accounts
    // 3. Restrict permissions to namespace and tenant labels
    info!(
        tenant = %tenant_spec.tenant_id,
        namespace = %tenant_spec.namespace,
        "RBAC setup placeholder (full implementation pending)"
    );

    Ok(())
}

/// Clean up tenant when deleted (cascade delete namespace and policies)
pub async fn cleanup_tenant(tenant_spec: &TenantSpec, client: &Client) -> Result<()> {
    let ns_api: Api<Namespace> = Api::all(client.clone());

    info!(
        tenant = %tenant_spec.tenant_id,
        namespace = %tenant_spec.namespace,
        "Cleaning up tenant (deleting namespace)"
    );

    if tenant_spec.cleanup_on_delete {
        ns_api
            .delete(&tenant_spec.namespace, &kube::api::DeleteParams::default())
            .await?;
        info!(
            namespace = %tenant_spec.namespace,
            "Namespace deleted (cascade delete of all resources)"
        );
    } else {
        warn!(
            namespace = %tenant_spec.namespace,
            "Skipping namespace deletion (cleanup_on_delete=false)"
        );
    }

    Ok(())
}
