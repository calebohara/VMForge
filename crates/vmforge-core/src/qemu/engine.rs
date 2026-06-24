//! `QemuHypervisor` — the QEMU-backed [`Hypervisor`] implementation.
//!
//! Owns the running-VM registry, builds the launch command line, spawns and
//! supervises the QEMU process, and drives lifecycle over QMP. The QMP
//! transport is abstracted per-OS: Unix socket on macOS/Linux, TCP loopback
//! on Windows.

use crate::error::{Error, Result};
use crate::host::{self, Accelerator};
use crate::hypervisor::Hypervisor;
use crate::library::Library;
use crate::model::{VmConfig, VmId, VmState, VmSummary};
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
    /// Persistence store (`vmforge.toml` per VM directory). Rooted at
    /// `library_dir`.
    library: Library,
    running: Mutex<HashMap<String, Arc<RunningVm>>>,
    /// Serializes the VNC-display pick across concurrent `start` calls.
    /// Held across display-pick → spawn → QMP-connect, but **never** held
    /// together with the `running` registry lock (override #1).
    start_lock: Mutex<()>,
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
            library: Library::new(library_dir.clone()),
            library_dir,
            running: Mutex::new(HashMap::new()),
            start_lock: Mutex::new(()),
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
                accelerator: self.accel,
                emulated: false,
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

    /// Effective state of a VM that is *in the running registry*, applying the
    /// natural-exit reaper: if the process already exited, the entry is removed
    /// (and its QMP socket deleted) and `Some(Stopped)` is returned. `None` if
    /// the VM is not in the registry at all (caller treats that as `Defined`).
    pub async fn running_state(&self, id: &str) -> Option<VmState> {
        self.running_states().await.get(id).copied()
    }

    /// One-pass effective states for every VM in the running registry, reaping
    /// any that have exited (remove entry + delete socket). Takes the `running`
    /// lock only for the (short) scan and removals — never across QMP I/O.
    pub async fn running_states(&self) -> HashMap<String, VmState> {
        // Snapshot the registry under a short lock, then probe outside it.
        let entries: Vec<(String, Arc<RunningVm>)> = {
            let reg = self.running.lock().await;
            reg.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
        };

        let mut states = HashMap::new();
        let mut reap: Vec<(String, Arc<RunningVm>)> = Vec::new();
        for (id, vm) in entries {
            let exited = {
                let mut proc = vm.process.lock().await;
                proc.try_wait().ok().flatten().is_some()
            };
            if exited {
                states.insert(id.clone(), VmState::Stopped);
                reap.push((id, vm));
            } else {
                // Reflect the live QMP status; fall back to Running if the
                // query fails transiently (the process is alive).
                let mut qmp = vm.qmp.lock().await;
                let st = qmp.query_status().await.unwrap_or(VmState::Running);
                states.insert(id, st);
            }
        }

        if !reap.is_empty() {
            let mut reg = self.running.lock().await;
            for (id, old_arc) in &reap {
                // Identity-check before removing: a concurrent `start` may have
                // replaced this id with a fresh entry while we probed outside
                // the lock (ABA). Only reap the exact allocation we observed
                // exited, so we never evict a just-restarted VM or delete its
                // live QMP socket.
                if reg.get(id).is_some_and(|cur| Arc::ptr_eq(cur, old_arc)) {
                    reg.remove(id);
                    if let Some(sock) = &old_arc.qmp_socket {
                        let _ = tokio::fs::remove_file(sock).await;
                    }
                }
            }
        }
        states
    }

    /// The full library view: every persisted VM as `Defined`, overlaid with
    /// the live state of anything in the running registry (and any running VM
    /// not yet persisted, defensively). Each summary carries this host's
    /// accelerator and `emulated == false` (override #2).
    pub async fn list_all(&self) -> Result<Vec<VmSummary>> {
        Ok(self.list_all_detailed().await?.0)
    }

    /// Like [`list_all`](Self::list_all), but also returns the parsed configs
    /// from the SAME single library scan, so the IPC layer can join
    /// hardware/iso detail without re-reading and re-parsing every
    /// `vmforge.toml` (one `read_dir` + parse per poll, not two).
    pub async fn list_all_detailed(&self) -> Result<(Vec<VmSummary>, Vec<VmConfig>)> {
        // Single directory scan + parse.
        let configs = self.library.load_all().await?;

        // Persisted VMs → Defined (accelerator/emulated filled here).
        let mut by_id: HashMap<VmId, VmSummary> = configs
            .iter()
            .map(|c| {
                (
                    c.id,
                    VmSummary {
                        id: c.id,
                        name: c.name.clone(),
                        state: VmState::Defined,
                        accelerator: self.accel,
                        emulated: false,
                    },
                )
            })
            .collect();

        // Overlay running state by id (reaps exited entries).
        let running = self.running_states().await;
        if !running.is_empty() {
            // Build a lookup of running configs for names not in the library.
            let live_names: HashMap<String, String> = {
                let reg = self.running.lock().await;
                reg.iter()
                    .map(|(k, v)| (k.clone(), v.config.name.clone()))
                    .collect()
            };
            for (id_str, state) in running {
                let Ok(id) = id_str.parse::<VmId>() else {
                    continue;
                };
                match by_id.get_mut(&id) {
                    Some(summary) => summary.state = state,
                    None => {
                        // Running but not persisted — include defensively.
                        let name = live_names
                            .get(&id_str)
                            .cloned()
                            .unwrap_or_else(|| id_str.clone());
                        by_id.insert(
                            id,
                            VmSummary {
                                id,
                                name,
                                state,
                                accelerator: self.accel,
                                emulated: false,
                            },
                        );
                    }
                }
            }
        }

        Ok((by_id.into_values().collect(), configs))
    }

    // ---- persistence passthroughs (keep commands.rs thin) ----

    /// Persist a new VM (dir + toml + qcow2). Does not launch.
    pub async fn create_vm(&self, config: VmConfig) -> Result<VmConfig> {
        self.library.create_vm(config).await
    }

    /// Load a persisted config by id string.
    pub async fn get_config(&self, id: &str) -> Result<VmConfig> {
        let id = parse_id(id)?;
        self.library.load_config(&id).await
    }

    /// Load every well-formed persisted config. Used by the IPC layer to join
    /// hardware/iso detail onto the `list_all` summaries while keeping
    /// `commands.rs` thin.
    pub async fn load_all(&self) -> Result<Vec<VmConfig>> {
        self.library.load_all().await
    }

    /// Overwrite a persisted config. Rejected with [`Error::Config`] if the VM
    /// is currently live (effective state not in `{Defined, Stopped}`).
    pub async fn update_config(&self, id: &str, updated: VmConfig) -> Result<VmConfig> {
        // Hold start_lock across the live-check + write so a concurrent `start`
        // cannot launch the VM in the TOCTOU window after the check.
        let _g = self.start_lock.lock().await;
        self.reject_if_live(id, "edit").await?;
        self.library.save_config(&updated).await?;
        let parsed = parse_id(id)?;
        self.library.load_config(&parsed).await
    }

    /// Delete a persisted VM. Rejected if live; also clears any stale QMP
    /// socket for the id.
    pub async fn delete(&self, id: &str, delete_disks: bool) -> Result<()> {
        // Hold start_lock across the live-check + delete so a concurrent `start`
        // can't launch the VM whose directory we're about to remove.
        let _g = self.start_lock.lock().await;
        self.reject_if_live(id, "delete").await?;
        let parsed = parse_id(id)?;
        self.library.delete_vm(&parsed, delete_disks).await?;
        #[cfg(unix)]
        {
            let sock = crate::paths::qmp_socket_path(id);
            let _ = tokio::fs::remove_file(&sock).await;
        }
        Ok(())
    }

    /// Rename a persisted VM (metadata only). Allowed while running.
    pub async fn rename(&self, id: &str, new_name: &str) -> Result<VmConfig> {
        let parsed = parse_id(id)?;
        self.library.rename_vm(&parsed, new_name).await
    }

    /// Reject the operation if the VM is currently live (running/paused/etc).
    /// A VM that is absent from the registry, or whose process has exited
    /// (reaped to `Stopped`), is editable.
    async fn reject_if_live(&self, id: &str, action: &str) -> Result<()> {
        match self.running_state(id).await {
            None | Some(VmState::Stopped) | Some(VmState::Defined) => Ok(()),
            Some(state) => Err(Error::Config(format!(
                "cannot {action} VM {id} while it is {state:?}; stop it first"
            ))),
        }
    }
}

/// Parse an id string into a [`VmId`], mapping a bad id to [`Error::VmNotFound`].
fn parse_id(id: &str) -> Result<VmId> {
    id.parse::<VmId>()
        .map_err(|_| Error::VmNotFound(id.to_string()))
}

#[async_trait]
impl Hypervisor for QemuHypervisor {
    fn accelerator(&self) -> Accelerator {
        self.accel
    }

    async fn start(&self, config: &VmConfig) -> Result<()> {
        let id = config.id.to_string();

        // Serialize the entire start critical section (membership check → VNC
        // display pick → spawn → QMP-connect → insert) against other starts and
        // against delete/update_config (override #1 / review). This closes the
        // TOCTOU where a VM could be launched in the window of a concurrent
        // delete, and stops two starts colliding on a VNC display. The short
        // `running` registry lock is still taken only for the membership check
        // and the final insert — never across the QMP connect.
        let _start_guard = self.start_lock.lock().await;

        if self.running.lock().await.contains_key(&id) {
            return Err(Error::Config(format!("VM {id} is already running")));
        }

        // Directory is slug-addressed (Phase 2). Fall back to the name only for
        // hand-built configs that never went through the library.
        let dir_key = if config.dir_slug.is_empty() {
            config.name.as_str()
        } else {
            config.dir_slug.as_str()
        };
        let vm_dir = crate::paths::vm_dir(&self.library_dir, dir_key);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DiskSpec, Hardware, NetworkConfig, VmConfig};
    use uuid::Uuid;

    fn defined_config(id: Uuid, name: &str, slug: &str) -> VmConfig {
        VmConfig {
            id,
            name: name.to_string(),
            schema_version: 1,
            dir_slug: slug.to_string(),
            hardware: Hardware {
                cpus: 2,
                memory_mib: 1024,
            },
            disks: vec![DiskSpec {
                path: "disk.qcow2".into(),
                size_gib: 4,
                backing: None,
            }],
            network: NetworkConfig::default(),
            display: Default::default(),
            iso: None,
            metadata: Default::default(),
        }
    }

    // ---- (18) list_all over empty registry → all Defined ----
    #[tokio::test]
    async fn list_all_defined_with_empty_registry() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img().await;
        let hv =
            QemuHypervisor::with_library_dir(tmp.path().to_path_buf()).expect("build hypervisor");

        // Persist two VMs through the library (no launch).
        hv.create_vm(defined_config(Uuid::new_v4(), "Alpha", "alpha"))
            .await
            .unwrap();
        hv.create_vm(defined_config(Uuid::new_v4(), "Beta", "beta"))
            .await
            .unwrap();

        let all = hv.list_all().await.unwrap();
        assert_eq!(all.len(), 2);
        for s in &all {
            assert_eq!(s.state, VmState::Defined);
            assert_eq!(s.accelerator, hv.accelerator());
            assert!(!s.emulated);
        }
        // Empty registry → no running states.
        assert!(hv.running_states().await.is_empty());
        assert!(hv
            .running_state(&Uuid::new_v4().to_string())
            .await
            .is_none());
    }

    // ---- (19) reaper drops an exited process (Stopped + remove + del socket)
    #[cfg(unix)]
    #[tokio::test]
    async fn reaper_drops_exited_process() {
        let tmp = tempfile::tempdir().unwrap();
        let hv =
            QemuHypervisor::with_library_dir(tmp.path().to_path_buf()).expect("build hypervisor");

        // Spawn a process that exits immediately (portable `sh -c 'exit 0'`).
        let log = tmp.path().join("proc.log");
        let mut proc = QemuProcess::spawn("/bin/sh", &["-c".into(), "exit 0".into()], &log)
            .await
            .expect("spawn sh");
        // Wait for it to actually exit so try_wait reports Some.
        proc.wait().await.expect("await exit");

        // Fake a QMP socket file that the reaper must delete.
        let id = Uuid::new_v4();
        let id_str = id.to_string();
        let sock = tmp.path().join(format!("{id_str}.sock"));
        tokio::fs::write(&sock, b"").await.unwrap();

        // Insert a registry entry pointing at the exited process + fake socket.
        let config = defined_config(id, "Reaped", "reaped");
        hv.running.lock().await.insert(
            id_str.clone(),
            Arc::new(RunningVm {
                config,
                vnc_port: 5901,
                qmp_socket: Some(sock.clone()),
                process: Mutex::new(proc),
                qmp: Mutex::new(QmpClient::dummy()),
                bridge: Mutex::new(None),
            }),
        );

        // running_states must reap: report Stopped, drop the entry, del socket.
        let states = hv.running_states().await;
        assert_eq!(states.get(&id_str).copied(), Some(VmState::Stopped));
        assert!(
            !hv.running.lock().await.contains_key(&id_str),
            "exited entry must be removed from the registry"
        );
        assert!(!sock.exists(), "QMP socket must be deleted on reap");

        // A second pass sees nothing (already reaped).
        assert!(hv.running_states().await.is_empty());
        assert!(hv.running_state(&id_str).await.is_none());
    }
}
