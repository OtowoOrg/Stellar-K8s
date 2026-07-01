//! Shared imports for resources submodules.

pub use crate::controller::resource_meta::merge_resource_meta;

// *** NEW: import kms_secret so we can accept SeedInjectionSpec ***
pub use crate::controller::kms_secret;
pub use crate::controller::label_propagation::LabelPropagator;

pub use std::collections::{BTreeMap, BTreeSet};

pub use k8s_openapi::api::apps::v1::{Deployment, DeploymentSpec, StatefulSet, StatefulSetSpec};
pub use k8s_openapi::api::autoscaling::v2::{
    CrossVersionObjectReference, HPAScalingPolicy, HPAScalingRules, HorizontalPodAutoscaler,
    HorizontalPodAutoscalerBehavior, HorizontalPodAutoscalerSpec, MetricIdentifier, MetricSpec,
    MetricTarget, ObjectMetricSource,
};
pub use k8s_openapi::api::core::v1::{
    Affinity, Capabilities, ConfigMap, Container, ContainerPort, EnvVar, EnvVarSource,
    PersistentVolumeClaim, PersistentVolumeClaimSpec, PodAffinityTerm, PodAntiAffinity,
    PodSecurityContext, PodSpec, PodTemplateSpec, ResourceRequirements as K8sResources,
    SeccompProfile, SecretKeySelector, SecurityContext, Service, ServicePort, ServiceSpec,
    Toleration, TypedLocalObjectReference, Volume, VolumeMount, VolumeResourceRequirements,
    WeightedPodAffinityTerm,
};
pub use k8s_openapi::api::networking::v1::{
    HTTPIngressPath, HTTPIngressRuleValue, IPBlock, Ingress, IngressBackend, IngressRule,
    IngressServiceBackend, IngressSpec, IngressTLS, NetworkPolicy, NetworkPolicyIngressRule,
    NetworkPolicyPeer, NetworkPolicyPort, NetworkPolicySpec, ServiceBackendPort,
};
pub use k8s_openapi::api::policy::v1::{PodDisruptionBudget, PodDisruptionBudgetSpec};
pub use k8s_openapi::apimachinery::pkg::api::resource::Quantity;
pub use k8s_openapi::apimachinery::pkg::apis::meta::v1::{
    LabelSelector, ObjectMeta, OwnerReference,
};
pub use k8s_openapi::apimachinery::pkg::util::intstr::IntOrString;
pub use kube::api::{
    Api, ApiResource, DeleteParams, DynamicObject, GroupVersionKind, Patch, PatchParams, PostParams,
};
pub use kube::{Client, Resource, ResourceExt};
pub use tracing::{info, instrument, warn};

pub use crate::crd::types::{PodAntiAffinityStrength, ReplicationRole, RolloutStrategyType};
pub use crate::crd::{
    BackupConfiguration, BarmanObjectStore, BootstrapConfiguration, Cluster, ClusterSpec,
    ExternalCluster, HistoryMode, HsmProvider, IngressConfig, InitDbConfiguration, KeySource,
    ManagedDatabaseConfig, MonitoringConfiguration, NetworkPolicyConfig, NodeType, PgBouncerSpec,
    Pooler, PoolerCluster, PoolerSpec, PostgresConfiguration, RecoveryConfiguration,
    ReplicaConfiguration, ResourceRequirements, S3Credentials,
    SecretKeySelector as CnpgSecretKeySelector, StellarNode, StellarNodeSpec, StorageConfiguration,
    WalBackupConfiguration,
};
pub use crate::error::{Error, Result};
pub use crate::scheduler::scoring::extract_peer_names_from_toml;
