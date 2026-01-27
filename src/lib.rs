//! Stellar-K8s: Cloud-Native Kubernetes Operator for Stellar Infrastructure
//!
//! This crate provides a Kubernetes operator for managing Stellar Core,
//! Horizon, and Soroban RPC nodes on Kubernetes clusters.

pub mod controller;
pub mod crd;
pub mod error;
pub mod scheduler;
pub mod telemetry;

#[cfg(feature = "rest-api")]
pub mod rest_api;

#[cfg(feature = "admission-webhook")]
pub mod webhook;

pub use crate::error::{Error, Result};

/// Configuration for mTLS
#[derive(Clone, Debug)]
pub struct MtlsConfig {
    pub cert_pem: Vec<u8>,
    pub key_pem: Vec<u8>,
    pub ca_pem: Vec<u8>,
}
