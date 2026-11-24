//! Restarting supervisor for the Captive Core child process.

use std::{path::PathBuf, time::Duration};

use tokio::{
    process::{Child, Command},
    time::{sleep, timeout},
};

use super::ipc::probe_http;

/// Runtime settings for a Captive Core supervisor.
#[derive(Debug, Clone)]
pub struct SupervisorConfig {
    pub command: String,
    pub args: Vec<String>,
    pub ipc_endpoint: String,
    pub lock_path: PathBuf,
    pub probe_interval: Duration,
    pub hung_after: Duration,
    pub term_grace: Duration,
}

impl Default for SupervisorConfig {
    fn default() -> Self {
        Self {
            command: "stellar-core".into(),
            args: Vec::new(),
            ipc_endpoint: "http://127.0.0.1:11626/info".into(),
            lock_path: "/var/lib/stellar/core.lock".into(),
            probe_interval: Duration::from_secs(5),
            hung_after: Duration::from_secs(15),
            term_grace: Duration::from_secs(5),
        }
    }
}

/// Owns one Captive Core process and restarts it after crash or IPC hangs.
pub struct CaptiveCoreSupervisor {
    config: SupervisorConfig,
    child: Option<Child>,
    client: reqwest::Client,
}

impl CaptiveCoreSupervisor {
    pub fn new(config: SupervisorConfig) -> Self {
        Self {
            config,
            child: None,
            client: reqwest::Client::new(),
        }
    }

    /// Remove a stale lock before spawning a fresh process.
    fn clean_lock(&self) -> std::io::Result<()> {
        match std::fs::remove_file(&self.config.lock_path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error),
        }
    }

    pub async fn start(&mut self) -> std::io::Result<()> {
        self.clean_lock()?;
        self.child = Some(
            Command::new(&self.config.command)
                .args(&self.config.args)
                .spawn()?,
        );
        Ok(())
    }

    /// Run supervision until the child exits or `stop` is called by the owner.
    /// Returns `true` when a restart was required and completed.
    pub async fn supervise_once(&mut self) -> Result<bool, String> {
        if self.child.is_none() {
            self.start().await.map_err(|error| error.to_string())?;
            return Ok(true);
        }
        if self
            .child
            .as_mut()
            .expect("child checked")
            .try_wait()
            .map_err(|e| e.to_string())?
            .is_some()
        {
            self.start().await.map_err(|error| error.to_string())?;
            return Ok(true);
        }

        sleep(self.config.probe_interval).await;
        if probe_http(
            &self.client,
            &self.config.ipc_endpoint,
            self.config.hung_after,
        )
        .await
        .is_ok()
        {
            return Ok(false);
        }

        self.terminate().await?;
        self.start().await.map_err(|error| error.to_string())?;
        Ok(true)
    }

    async fn terminate(&mut self) -> Result<(), String> {
        let Some(child) = self.child.as_mut() else {
            return Ok(());
        };
        let _ = child.start_kill();
        if timeout(self.config.term_grace, child.wait()).await.is_err() {
            let _ = child.kill().await;
        }
        self.child = None;
        Ok(())
    }

    pub async fn stop(&mut self) -> Result<(), String> {
        self.terminate().await
    }
}
