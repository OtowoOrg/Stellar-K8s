//! Captive Core process lifecycle management
//!
//! Handles process spawning, termination, and recovery for the Captive Core
//! embedded in Soroban RPC instances.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::{Duration, SystemTime};
use std::os::unix::fs::MetadataExt;

use tokio::process::Command;
use tokio::time::sleep;
use tracing::{debug, error, info, warn};

use crate::error::{Error, Result};

/// Path to Captive Core lock file
pub const CORE_LOCK_PATH: &str = "/var/lib/stellar/core.lock";

/// Maximum time to wait for graceful process termination (SIGTERM)
pub const GRACEFUL_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(30);

/// Maximum time to wait for process to respond after forced termination (SIGKILL)
pub const FORCED_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(10);

/// Captive Core process state
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessState {
    /// Process is running
    Running,
    /// Process is not running
    Stopped,
    /// Process is terminating
    Terminating,
    /// Process is in unknown state
    Unknown,
}

/// Lock file information
#[derive(Debug, Clone)]
pub struct LockFileInfo {
    /// Whether the lock file exists
    pub exists: bool,
    /// Age of the lock file (if it exists)
    pub age: Option<Duration>,
    /// Process ID in the lock file (if readable)
    pub pid: Option<u32>,
}

/// Captive Core process handle
pub struct CaptiveCoreProcess {
    /// Process ID (if running)
    pid: Option<u32>,
    /// State of the process
    state: ProcessState,
    /// Lock file path
    lock_path: PathBuf,
    /// Captive Core executable path
    core_binary: PathBuf,
    /// Last known process state change time
    state_changed_at: Option<SystemTime>,
}

impl CaptiveCoreProcess {
    /// Create a new CaptiveCoreProcess handle
    ///
    /// # Arguments
    /// * `core_binary` - Path to the Captive Core binary
    /// * `lock_path` - Path to the lock file (defaults to CORE_LOCK_PATH)
    pub fn new(core_binary: PathBuf, lock_path: Option<PathBuf>) -> Self {
        Self {
            pid: None,
            state: ProcessState::Unknown,
            lock_path: lock_path.unwrap_or_else(|| PathBuf::from(CORE_LOCK_PATH)),
            core_binary,
            state_changed_at: None,
        }
    }

    /// Get the current process state
    pub fn state(&self) -> ProcessState {
        self.state
    }

    /// Get the current process PID
    pub fn pid(&self) -> Option<u32> {
        self.pid
    }

    /// Check if the lock file exists and is stale
    pub async fn check_lock_file(&self) -> Result<LockFileInfo> {
        match fs::metadata(&self.lock_path) {
            Ok(metadata) => {
                let modified = metadata.modified().map_err(|e| {
                    Error::Other(format!("Failed to read lock file metadata: {}", e))
                })?;

                let age = SystemTime::now()
                    .duration_since(modified)
                    .unwrap_or(Duration::from_secs(0));

                let pid = self.read_pid_from_lock().await;

                Ok(LockFileInfo {
                    exists: true,
                    age: Some(age),
                    pid,
                })
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok(LockFileInfo {
                    exists: false,
                    age: None,
                    pid: None,
                })
            }
            Err(e) => Err(Error::Other(format!(
                "Failed to check lock file: {}",
                e
            ))),
        }
    }

    /// Read the PID from the lock file
    async fn read_pid_from_lock(&self) -> Option<u32> {
        match fs::read_to_string(&self.lock_path) {
            Ok(content) => content.trim().parse::<u32>().ok(),
            Err(_) => None,
        }
    }

    /// Check if the lock file is stale (process not running but lock exists)
    pub async fn is_lock_stale(&self) -> Result<bool> {
        let lock_info = self.check_lock_file().await?;

        if !lock_info.exists {
            return Ok(false);
        }

        // If we can read the PID from lock, check if process is running
        if let Some(lock_pid) = lock_info.pid {
            if !self.process_exists(lock_pid).await {
                debug!(
                    "Lock file exists but process {} is not running",
                    lock_pid
                );
                return Ok(true);
            }
        }

        // If lock file is very old (more than 1 hour), consider it stale
        if let Some(age) = lock_info.age {
            if age > Duration::from_secs(3600) {
                warn!(
                    "Lock file is very old ({:?}), considering it stale",
                    age
                );
                return Ok(true);
            }
        }

        Ok(false)
    }

    /// Remove the lock file safely
    ///
    /// Only removes the lock file if:
    /// 1. The process is confirmed to be dead
    /// 2. The lock file is confirmed stale
    pub async fn remove_stale_lock(&mut self) -> Result<()> {
        // Double-check that the lock is actually stale
        if !self.is_lock_stale().await? {
            debug!("Lock is not stale, not removing");
            return Ok(());
        }

        // Verify once more that the process in the lock is truly dead
        let lock_info = self.check_lock_file().await?;
        if let Some(lock_pid) = lock_info.pid {
            if self.process_exists(lock_pid).await {
                return Err(Error::Other(
                    "Cannot remove lock file: process is still running".to_string(),
                ));
            }
        }

        match fs::remove_file(&self.lock_path) {
            Ok(_) => {
                info!("Successfully removed stale lock file");
                Ok(())
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                debug!("Lock file already removed");
                Ok(())
            }
            Err(e) => Err(Error::Other(format!(
                "Failed to remove lock file: {}",
                e
            ))),
        }
    }

    /// Check if a process with the given PID exists
    async fn process_exists(&self, pid: u32) -> bool {
        match fs::metadata(format!("/proc/{}", pid)) {
            Ok(_) => true,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
            Err(_) => false,
        }
    }

    /// Gracefully terminate the process with SIGTERM
    pub async fn terminate_graceful(&mut self, timeout: Duration) -> Result<bool> {
        if self.state == ProcessState::Stopped {
            debug!("Process already stopped");
            return Ok(true);
        }

        if let Some(pid) = self.pid {
            info!("Sending SIGTERM to process {}", pid);

            // Send SIGTERM
            if let Err(e) = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGTERM,
            ) {
                warn!("Failed to send SIGTERM to {}: {}", pid, e);
                return Ok(false);
            }

            self.state = ProcessState::Terminating;
            self.state_changed_at = Some(SystemTime::now());

            // Wait for process to exit
            let start = SystemTime::now();
            while start.elapsed().unwrap_or(Duration::MAX) < timeout {
                sleep(Duration::from_millis(100)).await;

                if !self.process_exists(pid).await {
                    self.state = ProcessState::Stopped;
                    self.pid = None;
                    info!("Process {} terminated gracefully", pid);
                    return Ok(true);
                }
            }

            warn!(
                "Process {} did not terminate after {:?}",
                pid, timeout
            );
            return Ok(false);
        }

        Ok(true)
    }

    /// Force terminate the process with SIGKILL
    pub async fn terminate_forced(&mut self) -> Result<()> {
        if self.state == ProcessState::Stopped {
            return Ok(());
        }

        if let Some(pid) = self.pid {
            info!("Sending SIGKILL to process {}", pid);

            if let Err(e) = nix::sys::signal::kill(
                nix::unistd::Pid::from_raw(pid as i32),
                nix::sys::signal::Signal::SIGKILL,
            ) {
                warn!("Failed to send SIGKILL to {}: {}", pid, e);
            }

            // Wait a bit for the process to be killed
            sleep(Duration::from_millis(500)).await;

            if !self.process_exists(pid).await {
                self.state = ProcessState::Stopped;
                self.pid = None;
                info!("Process {} killed", pid);
            }
        }

        Ok(())
    }

    /// Spawn the Captive Core process
    pub async fn spawn(&mut self, args: Vec<String>) -> Result<u32> {
        if self.state == ProcessState::Running {
            debug!("Process already running");
            return Ok(self.pid.ok_or_else(|| {
                Error::Other("Process state is Running but no PID".to_string())
            })?);
        }

        info!(
            "Spawning Captive Core process: {} {:?}",
            self.core_binary.display(),
            args
        );

        let mut cmd = Command::new(&self.core_binary);
        cmd.args(&args)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(false);

        match cmd.spawn() {
            Ok(child) => {
                let pid = child.id().ok_or_else(|| {
                    Error::Other("Failed to get child process ID".to_string())
                })?;

                self.pid = Some(pid);
                self.state = ProcessState::Running;
                self.state_changed_at = Some(SystemTime::now());

                info!("Spawned Captive Core with PID {}", pid);
                Ok(pid)
            }
            Err(e) => Err(Error::Other(format!(
                "Failed to spawn Captive Core: {}",
                e
            ))),
        }
    }

    /// Perform a graceful shutdown followed by forced termination if needed
    pub async fn shutdown(&mut self) -> Result<()> {
        debug!("Initiating Captive Core shutdown");

        // Try graceful shutdown first
        let graceful_success = self.terminate_graceful(GRACEFUL_SHUTDOWN_TIMEOUT).await?;

        if !graceful_success {
            // If graceful shutdown fails, force terminate
            warn!("Graceful shutdown failed, forcing termination");
            self.terminate_forced().await?;
        }

        // Clean up stale lock file if it exists
        if self.is_lock_stale().await? {
            self.remove_stale_lock().await?;
        }

        Ok(())
    }

    /// Restart the process after cleanup
    pub async fn restart(&mut self, args: Vec<String>) -> Result<u32> {
        info!("Restarting Captive Core process");

        // Shutdown current process
        self.shutdown().await?;

        // Clean up any stale locks
        if self.is_lock_stale().await? {
            self.remove_stale_lock().await?;
        }

        // Wait a bit before restarting
        sleep(Duration::from_secs(1)).await;

        // Spawn new process
        self.spawn(args).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_process_state_creation() {
        let process = CaptiveCoreProcess::new(
            PathBuf::from("/usr/bin/stellar-core"),
            Some(PathBuf::from("/tmp/test.lock")),
        );

        assert_eq!(process.state(), ProcessState::Unknown);
        assert_eq!(process.pid(), None);
    }

    #[test]
    fn test_lock_path_default() {
        let process = CaptiveCoreProcess::new(PathBuf::from("/usr/bin/stellar-core"), None);

        assert_eq!(process.lock_path, PathBuf::from(CORE_LOCK_PATH));
    }

    #[tokio::test]
    async fn test_nonexistent_lock_file() {
        let process = CaptiveCoreProcess::new(
            PathBuf::from("/usr/bin/stellar-core"),
            Some(PathBuf::from("/nonexistent/lock.file")),
        );

        let lock_info = process.check_lock_file().await.unwrap();
        assert!(!lock_info.exists);
        assert!(lock_info.age.is_none());
    }
}
