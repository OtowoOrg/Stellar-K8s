//! Captive Core process supervision and IPC health checks.

pub mod ipc;
pub mod process;
pub mod supervisor;

pub use process::CaptiveCoreProcess;
pub use supervisor::{CaptiveCoreSupervisor, SupervisorConfig};
