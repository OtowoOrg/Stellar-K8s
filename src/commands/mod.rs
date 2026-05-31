//! Subcommand implementations for the Stellar-K8s operator CLI.
//!
//! Each module corresponds to a major functional area of the operator's
//! command-line interface, such as running the operator, the simulator,
//! or generating runbooks.

pub mod benchmark;
pub mod check_crd;
pub mod doctor;
pub mod export_compliance;
pub mod info;
pub mod operator;
pub mod runbook;
pub mod simulator;
pub mod webhook;
