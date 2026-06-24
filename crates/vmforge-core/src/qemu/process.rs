//! QEMU process supervision: spawn, capture logs, monitor liveness, kill.

use crate::error::{Error, Result};
use std::path::Path;
use std::process::Stdio;
use tokio::process::{Child, Command};

pub struct QemuProcess {
    child: Child,
    pub pid: Option<u32>,
}

impl QemuProcess {
    /// Spawn `bin args...`, redirecting stdout+stderr to `log_path`.
    /// `kill_on_drop` guarantees no orphaned QEMU if the handle is dropped.
    pub async fn spawn(bin: &str, args: &[String], log_path: &Path) -> Result<Self> {
        if let Some(parent) = log_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let log = std::fs::File::create(log_path)?;
        let log_err = log.try_clone()?;

        let child = Command::new(bin)
            .args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err))
            .kill_on_drop(true)
            .spawn()
            .map_err(|e| Error::QemuNotFound(format!("{bin}: {e}")))?;

        let pid = child.id();
        Ok(Self { child, pid })
    }

    /// Force-terminate the process (SIGKILL / TerminateProcess).
    pub async fn kill(&mut self) -> Result<()> {
        self.child.kill().await.map_err(Error::from)
    }

    /// Non-blocking liveness check; `Some(status)` once exited.
    pub fn try_wait(&mut self) -> Result<Option<std::process::ExitStatus>> {
        self.child.try_wait().map_err(Error::from)
    }

    /// Await process exit.
    pub async fn wait(&mut self) -> Result<std::process::ExitStatus> {
        self.child.wait().await.map_err(Error::from)
    }
}
