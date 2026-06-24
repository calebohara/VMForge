//! `QemuHypervisor` — the QEMU-backed [`Hypervisor`] implementation.
//!
//! Owns the running-VM registry, builds the launch command line, spawns and
//! supervises the QEMU process, and drives lifecycle over QMP. The QMP
//! transport is abstracted per-OS: Unix socket on macOS/Linux, TCP loopback
//! on Windows.

use crate::error::{Error, Result};
use crate::host::{self, Accelerator};
use crate::hypervisor::Hypervisor;
use crate::model::{VmConfig, VmState, VmSummary};
use crate::qemu::args::{build_args, QemuLaunch, QmpEndpoint};
use crate::qemu::{firmware, process::QemuProcess, qmp::QmpClient};
use async_trait::async_trait;
use std::collections::HashMap;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;

/// How the QMP channel for a given VM is bound. Unix socket where available,
/// TCP loopback on Windows.
enum QmpBind {
    #[cfg(unix)]
    Unix(PathBuf),
    // Constructed only on non-unix (Windows uses TCP loopback for QMP).
    #[allow(dead_code)]
    Tcp(u16),
}

struct RunningVm {
    config: VmConfig,
    vnc_port: u16,
    #[cfg_attr(not(unix), allow(dead_code))]
    qmp_socket: Option<PathBuf>,
    process: Mutex<QemuProcess>,
    qmp: Mutex<QmpClient>,
    bridge: Mutex<Option<crate::console::VncBridge>>,
}

pub struct QemuHypervisor {
    accel: Accelerator,
    host_arch: String,
    library_dir: PathBuf,
    running: Mutex<HashMap<String, Arc<RunningVm>>>,
}

impl QemuHypervisor {
    /// Build using the host's preferred accelerator and the default
    /// `~/VMForge` library directory.
    pub fn new() -> Result<Self> {
        Self::with_library_dir(crate::paths::library_dir()?)
    }

    /// Build with an explicit library directory (used by tests).
    pub fn with_library_dir(library_dir: PathBuf) -> Result<Self> {
        let caps = host::probe()?;
        Ok(Self {
            accel: caps.preferred_accelerator,
            host_arch: caps.arch,
            library_dir,
            running: Mutex::new(HashMap::new()),
        })
    }

    /// Host VNC port for a running VM (for the noVNC bridge / IPC).
    pub async fn vnc_port(&self, id: &str) -> Result<u16> {
        Ok(self.get(id).await?.vnc_port)
    }

    /// Start (or reuse) the VNC↔WebSocket bridge for a VM and return the
    /// loopback WebSocket port noVNC should connect to.
    pub async fn open_console(&self, id: &str) -> Result<u16> {
        let vm = self.get(id).await?;
        let mut bridge = vm.bridge.lock().await;
        if let Some(b) = bridge.as_ref() {
            return Ok(b.ws_port);
        }
        let b = crate::console::VncBridge::start(vm.vnc_port).await?;
        let port = b.ws_port;
        *bridge = Some(b);
        Ok(port)
    }

    /// Summaries of currently-running VMs.
    pub async fn list_running(&self) -> Vec<VmSummary> {
        self.running
            .lock()
            .await
            .values()
            .map(|vm| VmSummary {
                id: vm.config.id,
                name: vm.config.name.clone(),
                state: VmState::Running,
            })
            .collect()
    }

    async fn get(&self, id: &str) -> Result<Arc<RunningVm>> {
        self.running
            .lock()
            .await
            .get(id)
            .cloned()
            .ok_or_else(|| Error::VmNotFound(id.to_string()))
    }

    async fn exec(&self, id: &str, cmd: &str) -> Result<()> {
        let vm = self.get(id).await?;
        let mut qmp = vm.qmp.lock().await;
        qmp.execute(cmd, None).await.map(|_| ())
    }
}

#[async_trait]
impl Hypervisor for QemuHypervisor {
    fn accelerator(&self) -> Accelerator {
        self.accel
    }

    async fn start(&self, config: &VmConfig) -> Result<()> {
        let id = config.id.to_string();
        if self.running.lock().await.contains_key(&id) {
            return Err(Error::Config(format!("VM {id} is already running")));
        }

        let vm_dir = crate::paths::vm_dir(&self.library_dir, &config.name);
        tokio::fs::create_dir_all(&vm_dir).await?;

        // Disk (create if absent).
        let (disk_rel, size) = match config.disks.first() {
            Some(d) => (d.path.clone(), d.size_gib),
            None => ("disk.qcow2".to_string(), 20),
        };
        let disk = vm_dir.join(&disk_rel);
        crate::storage::create_qcow2(&disk, size).await?;

        let log_path = vm_dir.join("qemu.log");
        let display = find_free_vnc_display()
            .ok_or_else(|| Error::Other("no free VNC port (5901-5963)".into()))?;
        let vnc_port = 5900 + display;

        // aarch64 needs UEFI firmware; x86 uses built-in SeaBIOS.
        let bin = host::system_binary(&self.host_arch).to_string();
        let fw = if self.host_arch == "aarch64" {
            let f = firmware::find_aarch64_uefi(&bin).ok_or_else(|| {
                Error::Other(
                    "aarch64 UEFI firmware (edk2-aarch64-code.fd) not found; install QEMU firmware"
                        .into(),
                )
            })?;
            Some(f)
        } else {
            None
        };

        let iso = config.iso.as_ref().map(PathBuf::from);

        // Bind QMP per-OS. Sockets go in a short runtime dir (macOS path-length
        // limit), not under the VM data dir.
        #[cfg(unix)]
        let qmp_bind = {
            tokio::fs::create_dir_all(crate::paths::runtime_dir()).await?;
            let sock = crate::paths::qmp_socket_path(&id);
            let _ = tokio::fs::remove_file(&sock).await; // clear stale socket
            QmpBind::Unix(sock)
        };
        #[cfg(not(unix))]
        let qmp_bind = QmpBind::Tcp(free_tcp_port()?);

        #[cfg(unix)]
        let qmp_socket = if let QmpBind::Unix(p) = &qmp_bind {
            Some(p.clone())
        } else {
            None
        };
        #[cfg(not(unix))]
        let qmp_socket: Option<PathBuf> = None;

        let qmp_arg = match &qmp_bind {
            #[cfg(unix)]
            QmpBind::Unix(p) => QmpEndpoint::UnixSocket(p),
            QmpBind::Tcp(port) => QmpEndpoint::Tcp(*port),
        };

        let args = build_args(&QemuLaunch {
            config,
            accel: self.accel,
            guest_arch: &self.host_arch,
            disk: &disk,
            iso: iso.as_deref(),
            firmware: fw.as_deref(),
            vnc_display: display,
            qmp: qmp_arg,
        });
        tracing::info!(target: "vmforge_core::qemu", vm = %config.name, ?args, "launching QEMU");

        let mut proc = QemuProcess::spawn(&bin, &args, &log_path).await?;

        // Connect QMP (the server appears shortly after spawn). Kill QEMU and
        // surface its log tail if we can't reach it.
        let qmp = match connect_qmp(&qmp_bind, Duration::from_secs(15)).await {
            Ok(c) => c,
            Err(e) => {
                let _ = proc.kill().await;
                let tail = tail_log(&log_path).await;
                return Err(Error::Qmp(format!("could not reach QMP: {e}{tail}")));
            }
        };

        self.running.lock().await.insert(
            id,
            Arc::new(RunningVm {
                config: config.clone(),
                vnc_port,
                qmp_socket,
                process: Mutex::new(proc),
                qmp: Mutex::new(qmp),
                bridge: Mutex::new(None),
            }),
        );
        Ok(())
    }

    async fn shutdown(&self, id: &str) -> Result<()> {
        // Graceful ACPI shutdown; the guest must honor it.
        self.exec(id, "system_powerdown").await
    }

    async fn kill(&self, id: &str) -> Result<()> {
        let vm = self.get(id).await?;
        vm.process.lock().await.kill().await?;
        if let Some(sock) = &vm.qmp_socket {
            let _ = tokio::fs::remove_file(sock).await;
        }
        self.running.lock().await.remove(id);
        Ok(())
    }

    async fn pause(&self, id: &str) -> Result<()> {
        self.exec(id, "stop").await
    }

    async fn resume(&self, id: &str) -> Result<()> {
        self.exec(id, "cont").await
    }

    async fn state(&self, id: &str) -> Result<VmState> {
        let vm = self.get(id).await?;
        // If the process already exited, report Stopped without poking QMP.
        {
            let mut proc = vm.process.lock().await;
            if proc.try_wait()?.is_some() {
                return Ok(VmState::Stopped);
            }
        }
        let mut qmp = vm.qmp.lock().await;
        qmp.query_status().await
    }
}

/// First free VNC display in 1..64 (host port 5900 + N).
fn find_free_vnc_display() -> Option<u16> {
    (1..64u16).find(|&d| TcpListener::bind(("127.0.0.1", 5900 + d)).is_ok())
}

#[cfg(not(unix))]
fn free_tcp_port() -> Result<u16> {
    let l = TcpListener::bind(("127.0.0.1", 0))?;
    Ok(l.local_addr()?.port())
}

async fn connect_qmp(bind: &QmpBind, timeout: Duration) -> Result<QmpClient> {
    let start = tokio::time::Instant::now();
    loop {
        let attempt = match bind {
            #[cfg(unix)]
            QmpBind::Unix(p) => QmpClient::connect_unix(p).await,
            QmpBind::Tcp(port) => QmpClient::connect_tcp(&format!("127.0.0.1:{port}")).await,
        };
        match attempt {
            Ok(c) => return Ok(c),
            Err(e) => {
                if start.elapsed() > timeout {
                    return Err(e);
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
}

async fn tail_log(path: &std::path::Path) -> String {
    match tokio::fs::read_to_string(path).await {
        Ok(s) if !s.trim().is_empty() => {
            let tail: String = s
                .lines()
                .rev()
                .take(8)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");
            format!("\n--- qemu.log (tail) ---\n{tail}")
        }
        _ => String::new(),
    }
}
