//! QEMU process supervision: spawn, capture logs, monitor liveness, kill.

use crate::error::{Error, Result};
use std::ffi::OsString;
use std::path::Path;
use std::process::Stdio;
use tokio::process::{Child, Command};

/// Build a `PATH` value with `dir` prepended to the current process `PATH`
/// (deduplicated by simple prefix check). Used so the QEMU child can locate any
/// sibling helpers in the same install dir even when the inherited `PATH` is
/// the stripped Finder set (D3).
fn prepend_path(dir: &Path) -> OsString {
    match std::env::var_os("PATH") {
        Some(existing) => {
            let mut paths: Vec<std::path::PathBuf> = vec![dir.to_path_buf()];
            for p in std::env::split_paths(&existing) {
                if p != dir {
                    paths.push(p);
                }
            }
            std::env::join_paths(paths).unwrap_or(existing)
        }
        None => dir.as_os_str().to_os_string(),
    }
}

pub struct QemuProcess {
    child: Child,
    pub pid: Option<u32>,
}

impl QemuProcess {
    /// Spawn `bin args...`, redirecting stdout+stderr to `log_path`.
    /// `kill_on_drop` guarantees no orphaned QEMU if the handle is dropped.
    ///
    /// `bin` must be an **absolute** path (D3): the caller resolves QEMU once
    /// via [`crate::qemu_resolve::resolve_qemu_binary`] and passes the result
    /// here, so a Finder-launched `.app` with an empty inherited `PATH` still
    /// finds QEMU. `extra_path` is prepended to the child's `PATH` env (the
    /// resolved bin dir) so any QEMU helper processes are discoverable too.
    pub async fn spawn(
        bin: &Path,
        args: &[String],
        log_path: &Path,
        extra_path: Option<&Path>,
    ) -> Result<Self> {
        if let Some(parent) = log_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let log = std::fs::File::create(log_path)?;
        let log_err = log.try_clone()?;

        let mut cmd = Command::new(bin);
        cmd.args(args)
            .stdin(Stdio::null())
            .stdout(Stdio::from(log))
            .stderr(Stdio::from(log_err))
            .kill_on_drop(true);

        if let Some(dir) = extra_path {
            cmd.env("PATH", prepend_path(dir));
        }

        let child = cmd
            .spawn()
            .map_err(|e| Error::QemuNotFound(format!("{}: {e}", bin.display())))?;

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
