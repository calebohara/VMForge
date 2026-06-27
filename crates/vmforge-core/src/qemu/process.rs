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
    ///
    /// `work_dir` sets the child's **current working directory** and must be a
    /// clean, app-owned directory (the per-VM dir). This is not cosmetic: QEMU's
    /// `qemu_find_file` resolves resource names (e.g. the default VNC keymap
    /// `en-us`) by trying the bare name relative to the CWD *before* its data
    /// dir. A GUI-launched app commonly inherits CWD `C:\Windows\System32`, where
    /// `en-US\` exists as a directory — `fopen` on it returns EACCES and QEMU
    /// aborts with `Could not open 'en-us': Permission denied` before the QMP
    /// server ever binds. Anchoring CWD to the VM dir (no such collision) makes
    /// QEMU fall through to its data dir and load the keymap correctly.
    pub async fn spawn(
        bin: &Path,
        args: &[String],
        log_path: &Path,
        work_dir: &Path,
        extra_path: Option<&Path>,
    ) -> Result<Self> {
        if let Some(parent) = log_path.parent() {
            tokio::fs::create_dir_all(parent).await?;
        }
        let log = std::fs::File::create(log_path)?;
        let log_err = log.try_clone()?;

        let mut cmd = Command::new(bin);
        cmd.args(args)
            .current_dir(work_dir)
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

#[cfg(test)]
mod tests {
    use super::*;

    /// `spawn` must run the child with its CWD set to `work_dir`. Regression
    /// guard for the Windows `en-us` keymap EACCES: QEMU resolves the default VNC
    /// keymap relative to the CWD before its data dir, so an inherited
    /// `C:\Windows\System32` (which has an `en-US\` directory) made QEMU abort at
    /// launch. Runs on both platforms: the child writes a file via a *relative*
    /// path, which lands in its CWD — so the file appearing under `work_dir`
    /// proves `spawn` set the CWD. (Comparing path strings instead would trip on
    /// Windows `canonicalize()` returning a `\\?\` extended-length prefix.)
    #[tokio::test]
    async fn spawn_runs_child_in_work_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let work = tmp.path();
        let log = work.join("spawn.log");
        let (bin, args): (&str, [String; 2]) = if cfg!(windows) {
            ("cmd", ["/C".into(), "echo ok> proof.txt".into()])
        } else {
            ("/bin/sh", ["-c".into(), "echo ok > proof.txt".into()])
        };
        let mut proc = QemuProcess::spawn(Path::new(bin), &args, &log, work, None)
            .await
            .expect("spawn child");
        proc.wait().await.expect("await child");
        assert!(
            work.join("proof.txt").exists(),
            "child wrote its relative file outside work_dir → CWD was not set"
        );
    }
}
