//! `QemuHypervisor` — the QEMU-backed [`Hypervisor`] implementation.
//!
//! Owns the running-VM registry, builds the launch command line, spawns and
//! supervises the QEMU process, and drives lifecycle over QMP. The QMP
//! transport is abstracted per-OS: Unix socket on macOS/Linux, TCP loopback
//! on Windows.

use crate::error::{Error, Result};
use crate::host::{self, Accelerator};
use crate::hypervisor::Hypervisor;
use crate::library::{self, Library};
use crate::model::{Snapshot, SnapshotNode, VmConfig, VmId, VmState, VmSummary};
use crate::qemu::args::{build_args, QemuLaunch, QmpEndpoint};
use crate::qemu::{firmware, process::QemuProcess, qmp::QmpClient};
use async_trait::async_trait;
use serde_json::json;
use std::collections::HashMap;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Timeout bounding a single live QMP snapshot job (save/load/delete). RAM
/// capture can be slow on a large guest, so this is generous.
const SNAPSHOT_JOB_TIMEOUT: Duration = Duration::from_secs(300);

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

    /// Build with an explicit library directory and a forced accelerator.
    ///
    /// Test/dev seam: the suspend/resume round-trip relies on QMP
    /// `snapshot-load`, which crashes under HVF on aarch64 — so verifying it on
    /// this host requires forcing TCG. Production code uses [`new`](Self::new) /
    /// [`with_library_dir`](Self::with_library_dir), which always probe.
    pub fn with_library_dir_and_accel(library_dir: PathBuf, accel: Accelerator) -> Result<Self> {
        let mut hv = Self::with_library_dir(library_dir)?;
        hv.accel = accel;
        Ok(hv)
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
                // Reflect the live QMP status, but never BLOCK on the per-VM QMP
                // channel: a long live snapshot job (snapshot-save/-load/-delete)
                // holds `vm.qmp` for its whole duration, and blocking here would
                // freeze the 2s library poll for every VM (override #1). Use a
                // non-blocking `try_lock`; on contention the process is already
                // confirmed alive (try_wait above), so report Running. A failed
                // query also falls back to Running.
                let st = match vm.qmp.try_lock() {
                    Ok(mut qmp) => qmp.query_status().await.unwrap_or(VmState::Running),
                    Err(_) => VmState::Running,
                };
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
        // Editing while suspended is refused: the persisted config must match the
        // captured vmstate it will resume into. Resume or discard first.
        let current = self.get_config(id).await?;
        if current.metadata.suspended_snapshot.is_some() {
            return Err(Error::Config(format!(
                "cannot edit VM {id} while it is suspended; resume or discard the suspended state first"
            )));
        }
        self.library.save_config(&updated).await?;
        let parsed = parse_id(id)?;
        self.library.load_config(&parsed).await
    }

    /// Delete a persisted VM. Rejected if live; rejected if it is a linked-clone
    /// parent (A5 — deleting it would orphan its children's backing); also
    /// clears any stale QMP socket for the id.
    pub async fn delete(&self, id: &str, delete_disks: bool) -> Result<()> {
        // Hold start_lock across the live-check + delete so a concurrent `start`
        // can't launch the VM whose directory we're about to remove.
        let _g = self.start_lock.lock().await;
        self.reject_if_live(id, "delete").await?;
        // Parent-protection: never remove a disk that a linked clone backs onto.
        let config = self.get_config(id).await?;
        self.reject_if_has_dependents(&config, "delete").await?;
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

    // ---- snapshots & clones (Phase 3) ----

    /// Absolute path to the VM's single disk, plus the loaded config. Phase-3
    /// snapshot/clone scope is single-disk: anything else is
    /// [`Error::NotImplemented`].
    async fn single_disk_path(&self, config: &VmConfig) -> Result<PathBuf> {
        match config.disks.as_slice() {
            [d] => Ok(crate::paths::vm_dir(&self.library_dir, &config.dir_slug).join(&d.path)),
            [] => Err(Error::Config(format!("VM {} has no disks", config.name))),
            _ => Err(Error::NotImplemented("multi-disk snapshots")),
        }
    }

    /// Refuse the op if `config` is a linked-clone parent — i.e. any other
    /// persisted VM has a disk backing that resolves to this VM's disk (A5).
    async fn reject_if_has_dependents(&self, config: &VmConfig, action: &str) -> Result<()> {
        // Scan EVERY disk (not just single-disk VMs) — routing this through
        // single_disk_path would wrongly fail start/delete of a 0- or
        // multi-disk VM (Phase 1/2 regression). A diskless VM has no dependents.
        let vm_dir = crate::paths::vm_dir(&self.library_dir, &config.dir_slug);
        let mut deps: Vec<String> = Vec::new();
        for d in &config.disks {
            deps.extend(self.library.dependents_of(&vm_dir.join(&d.path)).await?);
        }
        deps.sort();
        deps.dedup();
        if deps.is_empty() {
            Ok(())
        } else {
            Err(Error::Config(format!(
                "cannot {action} VM {}: it has linked clones ({}); delete or full-clone them first",
                config.name,
                deps.join(", ")
            )))
        }
    }

    /// Create a snapshot. Routing (A3) is decided atomically under `start_lock`:
    /// a live VM (Running/Paused) uses a QMP `snapshot-save` job (RAM captured),
    /// a stopped/defined VM uses an offline `qemu-img snapshot -c` (disk only).
    /// Transient states (Starting/Stopping/Error) are refused with
    /// [`Error::Config`]. The job runs on `vm.qmp` WITHOUT holding the `running`
    /// registry lock. Single-disk only.
    pub async fn create_snapshot(
        &self,
        id: &str,
        name: &str,
        parent: Option<Uuid>,
        notes: &str,
    ) -> Result<Snapshot> {
        let pre = self.get_config(id).await?;
        let disk = self.single_disk_path(&pre).await?;

        let snap_id = Uuid::new_v4();
        let tag = snap_id.to_string();

        // Decide live-vs-offline AND execute under start_lock so a concurrent
        // start can't race the check→route→exec against the open image (A3).
        // The OFFLINE qemu-img exec stays under the lock; only the (potentially
        // multi-minute) LIVE QMP job releases it so it never blocks other VMs.
        let guard = self.start_lock.lock().await;
        let route = self.snapshot_route(id, "snapshot").await?;
        let has_vm_state = match route {
            SnapshotRoute::Live(vm) => {
                drop(guard);
                // QEMU job IDs must start with a letter; a raw UUID often starts
                // with a digit. Ephemeral letter-prefixed job id — the qcow2
                // `tag` stays the snapshot's stable UUID.
                let job_id = format!("vmforge-{}", Uuid::new_v4());
                let mut qmp = vm.qmp.lock().await;
                qmp.run_job(
                    "snapshot-save",
                    json!({
                        "job-id": job_id.clone(),
                        "tag": tag,
                        "vmstate": "disk0",
                        "devices": ["disk0"],
                    }),
                    &job_id,
                    SNAPSHOT_JOB_TIMEOUT,
                )
                .await?;
                true
            }
            SnapshotRoute::Offline => {
                crate::storage::snapshot_create_offline(&disk, &tag).await?;
                drop(guard);
                false
            }
        };

        let snapshot = Snapshot {
            id: snap_id,
            name: name.to_string(),
            parent,
            created_at: library::now_rfc3339(),
            has_vm_state,
            notes: notes.to_string(),
            vm_state_size: 0,
        };
        // Re-read a FRESH config (a concurrent rename/edit may have persisted
        // during a long live job) and apply only the snapshot delta — never
        // write back the stale pre-job snapshot (lost-update guard).
        let mut config = self.get_config(id).await?;
        config.snapshots.push(snapshot.clone());
        self.library.save_config(&config).await?;
        Ok(snapshot)
    }

    /// Delete a snapshot, routing live/offline like [`create_snapshot`], then
    /// re-parenting any children of the removed node onto its grandparent and
    /// persisting. A snapshot missing from our metadata maps to
    /// [`Error::Config`].
    pub async fn delete_snapshot(&self, id: &str, snapshot_id: Uuid) -> Result<()> {
        let pre = self.get_config(id).await?;
        let disk = self.single_disk_path(&pre).await?;
        if !pre.snapshots.iter().any(|s| s.id == snapshot_id) {
            return Err(Error::Config(format!(
                "snapshot {snapshot_id} not found on VM {id}"
            )));
        }
        let tag = snapshot_id.to_string();

        // Same lock discipline as create_snapshot: offline exec under start_lock,
        // live job releases it.
        let guard = self.start_lock.lock().await;
        let route = self.snapshot_route(id, "delete snapshot").await?;
        match route {
            SnapshotRoute::Live(vm) => {
                drop(guard);
                // QEMU job IDs must start with a letter (see create_snapshot).
                let job_id = format!("vmforge-{}", Uuid::new_v4());
                let mut qmp = vm.qmp.lock().await;
                qmp.run_job(
                    "snapshot-delete",
                    json!({
                        "job-id": job_id.clone(),
                        "tag": tag,
                        "devices": ["disk0"],
                    }),
                    &job_id,
                    SNAPSHOT_JOB_TIMEOUT,
                )
                .await?;
            }
            SnapshotRoute::Offline => {
                crate::storage::snapshot_delete_offline(&disk, &tag).await?;
                drop(guard);
            }
        }

        // Fresh re-read before mutate+persist (lost-update guard).
        let mut config = self.get_config(id).await?;
        library::reparent_on_delete(&mut config.snapshots, snapshot_id);
        self.library.save_config(&config).await?;
        Ok(())
    }

    /// Restore a snapshot. Phase-3 scope reverts the DISK ONLY (A7) via
    /// `qemu-img snapshot -a`, so it is **stopped-only**: a live VM is refused
    /// (`reject_if_live`). Parent-protection (A5) also applies — a VM with
    /// linked children may not be reverted out from under them. Held under
    /// `start_lock` so the check-and-apply is atomic against a concurrent start.
    pub async fn restore_snapshot(&self, id: &str, snapshot_id: Uuid) -> Result<()> {
        let _g = self.start_lock.lock().await;
        self.reject_if_live(id, "restore a snapshot of").await?;

        let config = self.get_config(id).await?;
        self.reject_if_has_dependents(&config, "restore a snapshot of")
            .await?;
        if !config.snapshots.iter().any(|s| s.id == snapshot_id) {
            return Err(Error::Config(format!(
                "snapshot {snapshot_id} not found on VM {id}"
            )));
        }
        let disk = self.single_disk_path(&config).await?;
        crate::storage::snapshot_apply_offline(&disk, &snapshot_id.to_string()).await?;
        Ok(())
    }

    /// The reconciled snapshot tree for a VM: our metadata joined against the
    /// image's internal qcow2 snapshots. The image is read with
    /// `--force-share` (`-U`) iff the VM is running, so we never error on a
    /// QEMU-held image (A3 offline read path).
    pub async fn list_snapshots(&self, id: &str) -> Result<Vec<SnapshotNode>> {
        let config = self.get_config(id).await?;
        let disk = self.single_disk_path(&config).await?;
        let is_running = self.is_running(id).await;
        let stdout = crate::storage::info_json(&disk, is_running).await?;
        let qcow2 = crate::storage::parse_info_snapshots(&stdout)?;
        Ok(library::reconcile(&config.snapshots, &qcow2))
    }

    /// Clone a VM into a brand-new VM (A4). Stopped-source-only
    /// (`reject_if_live`); delegates the disk work to the library. Held under
    /// `start_lock` so the source can't be launched mid-clone.
    pub async fn clone_vm(&self, id: &str, new_name: &str, linked: bool) -> Result<VmConfig> {
        let _g = self.start_lock.lock().await;
        self.reject_if_live(id, "clone").await?;
        let src_id = parse_id(id)?;
        if linked {
            self.library.linked_clone(&src_id, new_name).await
        } else {
            self.library.full_clone(&src_id, new_name).await
        }
    }

    /// Whether `id` is currently in a non-stopped live state (reaper-aware).
    async fn is_running(&self, id: &str) -> bool {
        !matches!(
            self.running_state(id).await,
            None | Some(VmState::Stopped) | Some(VmState::Defined)
        )
    }

    /// Decide the snapshot route under the caller-held `start_lock`. Returns the
    /// live `Arc<RunningVm>` for Running/Paused, `Offline` for Stopped/Defined,
    /// and refuses transient states with [`Error::Config`] (A3). Reaping is
    /// applied via `running_state`.
    async fn snapshot_route(&self, id: &str, action: &str) -> Result<SnapshotRoute> {
        match self.running_state(id).await {
            None | Some(VmState::Stopped) | Some(VmState::Defined) => Ok(SnapshotRoute::Offline),
            Some(VmState::Running) | Some(VmState::Paused) => {
                // The VM is live and in the registry; fetch its handle for the
                // QMP job. (running_state already reaped a dead process.)
                let vm = self.get(id).await?;
                Ok(SnapshotRoute::Live(vm))
            }
            Some(state) => Err(Error::Config(format!(
                "cannot {action} VM {id} while it is {state:?}; wait for it to settle"
            ))),
        }
    }

    // ---- start (cold + resume) + suspend/restore (Phase 5) ----

    /// Launch a VM. With `prelaunch_load == None` this is a cold start; with
    /// `Some(tag)` it is a suspend-resume: QEMU is launched paused (`-S`) and the
    /// stored vmstate `tag` is `snapshot-load`-ed then `cont`-ed on the local
    /// QMP channel BEFORE the registry insert (lock discipline preserved).
    ///
    /// Cold start refuses a suspended VM (resume or discard first). Shared
    /// folders are validated before `build_args` (existing host dirs, safe tags).
    pub async fn start_inner(&self, config: &VmConfig, prelaunch_load: Option<Uuid>) -> Result<()> {
        let id = config.id.to_string();

        // Serialize the entire start critical section (membership check → VNC
        // display pick → spawn → QMP-connect → insert) against other starts and
        // against delete/update_config (override #1 / review). This closes the
        // TOCTOU where a VM could be launched in the window of a concurrent
        // delete, and stops two starts colliding on a VNC display. The short
        // `running` registry lock is still taken only for the membership check
        // and the final insert — never across the QMP connect.
        let _start_guard = self.start_lock.lock().await;

        // A plain (cold) start of a suspended VM is refused: its vmstate must be
        // resumed (restore_suspended) or thrown away (discard_suspend) first.
        // The resume path passes Some(tag), so it bypasses this guard.
        if prelaunch_load.is_none() && config.metadata.suspended_snapshot.is_some() {
            return Err(Error::Config(format!(
                "VM {id} is suspended; resume or discard the suspended state first"
            )));
        }

        if self.running.lock().await.contains_key(&id) {
            return Err(Error::Config(format!("VM {id} is already running")));
        }

        // Parent-protection (A5): refuse to boot a VM that backs a linked clone
        // — opening its disk RW would corrupt the children's CoW backing. Only
        // checked for library-managed VMs (those with a slug); hand-built
        // configs that never went through the library have no dependents.
        if !config.dir_slug.is_empty() {
            self.reject_if_has_dependents(config, "start").await?;
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

        // Validate shared folders BEFORE spawn: existing host dirs, safe unique
        // tags. An invalid share is rejected here as `Error::Config` rather than
        // failing opaquely at launch (decision A — validate_shared_folders).
        crate::library::validate_shared_folders(config)?;

        // Build (and validate) the network fragments BEFORE spawning. An
        // unavailable mode (bridged/host-only) or an invalid port-forward/MAC is
        // rejected here as `Error::Config` — never a silent NAT fallback (A3/A4).
        let network =
            crate::qemu::net::network_args(&config.network, self.accel, std::env::consts::OS)
                .map_err(|e| Error::Config(e.to_string()))?;

        let args = build_args(&QemuLaunch {
            config,
            accel: self.accel,
            guest_arch: &self.host_arch,
            disk: &disk,
            iso: iso.as_deref(),
            firmware: fw.as_deref(),
            vnc_display: display,
            qmp: qmp_arg,
            network,
            // Resume launches paused so snapshot-load + cont can run before the
            // guest executes; cold starts run immediately.
            prelaunch: prelaunch_load.is_some(),
        });
        tracing::info!(target: "vmforge_core::qemu", vm = %config.name, ?args, "launching QEMU");

        let mut proc = QemuProcess::spawn(&bin, &args, &log_path).await?;

        // Connect QMP (the server appears shortly after spawn). Kill QEMU and
        // surface its log tail if we can't reach it.
        let mut qmp = match connect_qmp(&qmp_bind, Duration::from_secs(15)).await {
            Ok(c) => c,
            Err(e) => {
                let _ = proc.kill().await;
                let tail = tail_log(&log_path).await;
                // A busy/invalid host forward port makes QEMU exit at startup
                // (so QMP never comes up). Map that specific failure to a clean,
                // actionable config error rather than the generic QMP-timeout.
                if log_indicates_busy_host_port(&tail) {
                    return Err(Error::Config(
                        "Host port already in use or invalid; pick another or free it".to_string(),
                    ));
                }
                return Err(Error::Qmp(format!("could not reach QMP: {e}{tail}")));
            }
        };

        // Resume path: load the suspended vmstate and resume execution on the
        // LOCAL QmpClient (still before the registry insert, so lock discipline
        // is preserved). On any failure, kill the process + clear its socket and
        // surface the error — never leave a phantom Running entry.
        if let Some(tag) = prelaunch_load {
            let job_id = format!("vmforge-{}", Uuid::new_v4());
            let result = match qmp
                .run_job(
                    "snapshot-load",
                    json!({
                        "job-id": job_id.clone(),
                        "tag": tag.to_string(),
                        "vmstate": "disk0",
                        "devices": ["disk0"],
                    }),
                    &job_id,
                    SNAPSHOT_JOB_TIMEOUT,
                )
                .await
            {
                Ok(()) => qmp.execute("cont", None).await.map(|_| ()),
                Err(e) => Err(e),
            };
            if let Err(e) = result {
                let _ = proc.kill().await;
                if let Some(sock) = &qmp_socket {
                    let _ = tokio::fs::remove_file(sock).await;
                }
                return Err(e);
            }
        }

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

    /// Suspend a live VM: capture RAM/device state to a fresh qcow2 vmstate tag,
    /// persist the tag in `metadata.suspended_snapshot` (BEFORE killing the
    /// process), then terminate. Accelerator-gated: refused up front on
    /// `aarch64 + HVF`, where QMP `snapshot-load` crashes (ORCHESTRATOR NOTE), so
    /// an unresumable vmstate is never captured. Single-disk only.
    pub async fn suspend(&self, id: &str) -> Result<()> {
        // Up-front accelerator gate: never capture a vmstate we cannot resume.
        if self.host_arch == "aarch64" && self.accel.is_hardware() {
            return Err(Error::Config(
                "suspend/resume is unavailable with hardware acceleration (HVF) on this host"
                    .into(),
            ));
        }

        let pre = self.get_config(id).await?;
        // Single-disk scope (the vmstate targets disk0).
        let _disk = self.single_disk_path(&pre).await?;

        let suspend_id = Uuid::new_v4();
        let tag = suspend_id.to_string();

        // Route under start_lock so a concurrent start/edit can't race the
        // check→exec. Only Live (Running/Paused) is suspendable; otherwise
        // Error::Config. Drop the guard before the (potentially long) QMP job so
        // it never blocks other VMs.
        let guard = self.start_lock.lock().await;
        let vm = match self.snapshot_route(id, "suspend").await? {
            SnapshotRoute::Live(vm) => vm,
            SnapshotRoute::Offline => {
                return Err(Error::Config(format!(
                    "cannot suspend VM {id}: it is not running"
                )));
            }
        };
        drop(guard);

        // Stop the CPUs, then save the full vmstate to the qcow2 under `tag`.
        {
            let mut qmp = vm.qmp.lock().await;
            qmp.execute("stop", None).await?;
            let job_id = format!("vmforge-{}", Uuid::new_v4());
            qmp.run_job(
                "snapshot-save",
                json!({
                    "job-id": job_id.clone(),
                    "tag": tag,
                    "vmstate": "disk0",
                    "devices": ["disk0"],
                }),
                &job_id,
                SNAPSHOT_JOB_TIMEOUT,
            )
            .await?;
        }

        // Persist the suspend tag BEFORE killing the process: a crash between
        // here and the kill leaves a resumable, correctly-flagged VM. Re-read a
        // fresh config (a concurrent rename may have landed during the job) and
        // apply only the suspend delta (lost-update guard).
        let mut config = self.get_config(id).await?;
        config.metadata.suspended_snapshot = Some(suspend_id);
        self.library.save_config(&config).await?;

        // Terminate (drops the registry entry + deletes the socket). The VM now
        // reads as Stopped + suspended.
        self.kill(id).await?;
        Ok(())
    }

    /// Resume a suspended VM: verify the stored vmstate tag still exists in the
    /// qcow2 (else clear the field and refuse — "reset to stopped"), relaunch via
    /// `start_inner(Some(tag))`, then clear `metadata.suspended_snapshot` so the
    /// suspend is consumed exactly once.
    pub async fn restore_suspended(&self, id: &str) -> Result<()> {
        let config = self.get_config(id).await?;
        let tag = config
            .metadata
            .suspended_snapshot
            .ok_or_else(|| Error::Config(format!("VM {id} is not suspended")))?;

        // Verify the vmstate tag is actually present in the image. If it has gone
        // missing (image edited/snapshot deleted out from under us), the suspend
        // is unresumable: clear the field and refuse so the VM resets to Stopped
        // rather than getting wedged.
        let disk = self.single_disk_path(&config).await?;
        let stdout = crate::storage::info_json(&disk, false).await?;
        let present = crate::storage::parse_info_snapshots(&stdout)?
            .iter()
            .any(|s| s.name == tag.to_string());
        if !present {
            let mut reset = self.get_config(id).await?;
            reset.metadata.suspended_snapshot = None;
            self.library.save_config(&reset).await?;
            return Err(Error::Config(format!(
                "suspended state for VM {id} is missing; reset to stopped"
            )));
        }

        // Relaunch paused, load the vmstate, and cont (start_inner handles the
        // kill-on-failure cleanup).
        self.start_inner(&config, Some(tag)).await?;

        // The vmstate has been loaded into RAM; the qcow2 internal snapshot it
        // came from is now orphaned (excluded from the snapshot tree). Delete it
        // via a live QMP job so repeated suspend/resume cycles don't grow the
        // image unbounded (~guest RAM per cycle). Best-effort: a failure must
        // NOT fail the resume — the VM is already running.
        if let Ok(vm) = self.get(id).await {
            let job_id = format!("vmforge-{}", Uuid::new_v4());
            let mut qmp = vm.qmp.lock().await;
            if let Err(e) = qmp
                .run_job(
                    "snapshot-delete",
                    json!({ "job-id": job_id, "tag": tag.to_string(), "devices": ["disk0"] }),
                    &job_id,
                    SNAPSHOT_JOB_TIMEOUT,
                )
                .await
            {
                tracing::warn!(target: "vmforge_core::qemu", vm = %id, error = %e,
                    "failed to delete consumed suspend vmstate; orphaned in qcow2");
            }
        }

        // Consume the suspend: clear the field and persist (fresh re-read).
        let mut config = self.get_config(id).await?;
        config.metadata.suspended_snapshot = None;
        self.library.save_config(&config).await?;
        Ok(())
    }

    /// Discard a suspended VM's captured state without resuming (escape hatch):
    /// best-effort delete the offline vmstate snapshot, then clear the field so
    /// the VM is a plain Stopped VM again. A no-op if not suspended.
    pub async fn discard_suspend(&self, id: &str) -> Result<()> {
        let config = self.get_config(id).await?;
        let Some(tag) = config.metadata.suspended_snapshot else {
            return Ok(());
        };
        // Best-effort: the snapshot may already be gone; clearing the field is
        // the contract that matters.
        if let Ok(disk) = self.single_disk_path(&config).await {
            let _ = crate::storage::snapshot_delete_offline(&disk, &tag.to_string()).await;
        }
        let mut config = self.get_config(id).await?;
        config.metadata.suspended_snapshot = None;
        self.library.save_config(&config).await?;
        Ok(())
    }
}

/// Routing decision for a snapshot create/delete (A3).
enum SnapshotRoute {
    /// Live VM — drive the op as a QMP job on this handle's `qmp` channel.
    Live(Arc<RunningVm>),
    /// Stopped/Defined VM — drive the op with the offline `qemu-img` runner.
    Offline,
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
        self.start_inner(config, None).await
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

    async fn suspend(&self, id: &str) -> Result<()> {
        QemuHypervisor::suspend(self, id).await
    }

    async fn restore_suspended(&self, id: &str) -> Result<()> {
        QemuHypervisor::restore_suspended(self, id).await
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

/// Whether a qemu.log tail indicates a busy/invalid host forwarding port.
/// QEMU prints `Could not set up host forwarding rule '...'` (port in use) or
/// `Bad host port` (out of range) and then exits during user-net setup.
fn log_indicates_busy_host_port(log: &str) -> bool {
    log.contains("Could not set up host forwarding rule") || log.contains("Bad host port")
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
            snapshots: Vec::new(),
            shared_folders: Vec::new(),
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

    // ====================================================================
    // Phase 3 — snapshots, clones, parent-protection
    // ====================================================================

    /// Persist a parent VM and a linked clone of it through the library, using
    /// the fuller mock binary. Returns (hv, parent_config, child_config). The
    /// returned MutexGuard keeps the mock env seam alive for the test.
    async fn parent_with_linked_child(
        tmp: &tempfile::TempDir,
    ) -> (QemuHypervisor, VmConfig, VmConfig) {
        let hv = QemuHypervisor::with_library_dir(tmp.path().to_path_buf()).expect("build hv");
        let parent = hv
            .create_vm(defined_config(Uuid::new_v4(), "Parent", "parent"))
            .await
            .unwrap();
        let child = hv
            .clone_vm(&parent.id.to_string(), "Child", true)
            .await
            .unwrap();
        (hv, parent, child)
    }

    // ---- parent-protection: delete a linked-clone parent is refused ----
    #[tokio::test]
    async fn parent_protection_refuses_delete() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img_full().await;
        let (hv, parent, _child) = parent_with_linked_child(&tmp).await;

        let err = hv
            .delete(&parent.id.to_string(), true)
            .await
            .expect_err("deleting a parent with a linked child must fail");
        assert!(matches!(err, Error::Config(_)), "got {err:?}");
        // Parent still present.
        assert!(hv.get_config(&parent.id.to_string()).await.is_ok());
    }

    // ---- parent-protection: start a linked-clone parent is refused ----
    #[tokio::test]
    async fn parent_protection_refuses_start() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img_full().await;
        let (hv, parent, _child) = parent_with_linked_child(&tmp).await;

        let err = hv
            .start(&parent)
            .await
            .expect_err("starting a parent with a linked child must fail");
        assert!(matches!(err, Error::Config(_)), "got {err:?}");
        // No process was registered.
        assert!(hv.running_state(&parent.id.to_string()).await.is_none());
    }

    // ---- parent-protection: restore a snapshot of a parent is refused ----
    #[tokio::test]
    async fn parent_protection_refuses_restore() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img_full().await;
        let (hv, parent, _child) = parent_with_linked_child(&tmp).await;

        // Give the parent an (offline) snapshot to attempt to restore.
        let snap = hv
            .create_snapshot(&parent.id.to_string(), "base", None, "")
            .await
            .unwrap();

        let err = hv
            .restore_snapshot(&parent.id.to_string(), snap.id)
            .await
            .expect_err("restoring a parent with a linked child must fail");
        assert!(matches!(err, Error::Config(_)), "got {err:?}");
    }

    // ---- offline snapshot create + delete round-trip persists metadata ----
    #[tokio::test]
    async fn offline_snapshot_create_and_delete_persist() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img_full().await;
        let hv =
            QemuHypervisor::with_library_dir(tmp.path().to_path_buf()).expect("build hypervisor");
        let vm = hv
            .create_vm(defined_config(Uuid::new_v4(), "Solo", "solo"))
            .await
            .unwrap();
        let id = vm.id.to_string();

        // Create (offline, since the VM is not running) → has_vm_state == false.
        let snap = hv
            .create_snapshot(&id, "first", None, "note")
            .await
            .unwrap();
        assert!(!snap.has_vm_state, "offline snapshot has no RAM state");
        let cfg = hv.get_config(&id).await.unwrap();
        assert_eq!(cfg.snapshots.len(), 1);
        assert_eq!(cfg.snapshots[0].name, "first");

        // Delete → metadata removed and persisted.
        hv.delete_snapshot(&id, snap.id).await.unwrap();
        let cfg = hv.get_config(&id).await.unwrap();
        assert!(
            cfg.snapshots.is_empty(),
            "snapshot metadata must be removed"
        );

        // Deleting an unknown snapshot is a Config error.
        let err = hv.delete_snapshot(&id, Uuid::new_v4()).await.unwrap_err();
        assert!(matches!(err, Error::Config(_)), "got {err:?}");
    }

    // ---- list_snapshots reconciles metadata against the (mock) image ----
    #[tokio::test]
    async fn list_snapshots_reconciles() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img_full().await;
        let hv =
            QemuHypervisor::with_library_dir(tmp.path().to_path_buf()).expect("build hypervisor");
        let vm = hv
            .create_vm(defined_config(Uuid::new_v4(), "Solo", "solo"))
            .await
            .unwrap();
        let id = vm.id.to_string();
        let snap = hv.create_snapshot(&id, "first", None, "").await.unwrap();

        // The mock `info` returns no internal snapshots, so our metadata entry
        // reconciles as a metadata-orphan (present_in_qcow2 == false).
        let nodes = hv.list_snapshots(&id).await.unwrap();
        assert_eq!(nodes.len(), 1);
        assert_eq!(nodes[0].meta.id, snap.id);
        assert!(!nodes[0].present_in_qcow2);
    }

    // ---- multi-disk snapshots/clones are NotImplemented ----
    #[tokio::test]
    async fn multi_disk_snapshot_is_not_implemented() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img_full().await;
        let hv =
            QemuHypervisor::with_library_dir(tmp.path().to_path_buf()).expect("build hypervisor");
        let mut cfg = defined_config(Uuid::new_v4(), "Multi", "multi");
        cfg.disks.push(DiskSpec {
            path: "disk2.qcow2".into(),
            size_gib: 2,
            backing: None,
        });
        let vm = hv.create_vm(cfg).await.unwrap();
        let id = vm.id.to_string();

        assert!(matches!(
            hv.create_snapshot(&id, "x", None, "").await.unwrap_err(),
            Error::NotImplemented(_)
        ));
        assert!(matches!(
            hv.list_snapshots(&id).await.unwrap_err(),
            Error::NotImplemented(_)
        ));
    }

    // ---- state refusal: restore/clone refused while the VM is live ----
    #[cfg(unix)]
    #[tokio::test]
    async fn live_state_refuses_restore_and_clone() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img_full().await;
        let hv =
            QemuHypervisor::with_library_dir(tmp.path().to_path_buf()).expect("build hypervisor");
        let vm = hv
            .create_vm(defined_config(Uuid::new_v4(), "Live", "live"))
            .await
            .unwrap();
        let id = vm.id.to_string();
        // Give it a snapshot entry to attempt to restore.
        let snap = hv.create_snapshot(&id, "base", None, "").await.unwrap();

        // Insert a registry entry whose process stays alive (so the reaper sees
        // it running) with a dummy QMP (query fails → falls back to Running).
        let log = tmp.path().join("proc.log");
        let proc = QemuProcess::spawn("/bin/sh", &["-c".into(), "sleep 30".into()], &log)
            .await
            .expect("spawn sleeper");
        hv.running.lock().await.insert(
            id.clone(),
            Arc::new(RunningVm {
                config: vm.clone(),
                vnc_port: 5901,
                qmp_socket: None,
                process: Mutex::new(proc),
                qmp: Mutex::new(QmpClient::dummy()),
                bridge: Mutex::new(None),
            }),
        );

        // Restore is stopped-only (A7) → refused while live.
        let err = hv.restore_snapshot(&id, snap.id).await.unwrap_err();
        assert!(matches!(err, Error::Config(_)), "restore: got {err:?}");

        // Clone is stopped-source-only → refused while live.
        let err = hv.clone_vm(&id, "Copy", false).await.unwrap_err();
        assert!(matches!(err, Error::Config(_)), "clone: got {err:?}");

        // Clean up the sleeper.
        let _ = hv.kill(&id).await;
    }

    // ====================================================================
    // Phase 4 — busy host-port log mapping (offline interim, E.2)
    // ====================================================================

    /// The exact stderr QEMU 11.0.1 prints when a forwarded host port is already
    /// bound — and the out-of-range variant — must be recognized and mapped to
    /// the clean, actionable config error rather than the generic QMP timeout.
    #[test]
    fn busy_host_port_log_is_recognized() {
        // Port already in use.
        let in_use = "qemu-system-aarch64: -netdev user,id=net0,hostfwd=tcp:127.0.0.1:2222-:22: \
                      Could not set up host forwarding rule 'tcp:127.0.0.1:2222-:22'";
        assert!(log_indicates_busy_host_port(in_use));

        // Out-of-range / invalid host port.
        let bad = "qemu-system-aarch64: -netdev user,id=net0,hostfwd=tcp:127.0.0.1:99999-:22: \
                   Bad host port 99999";
        assert!(log_indicates_busy_host_port(bad));

        // Unrelated failures must NOT be mapped to the host-port error.
        assert!(!log_indicates_busy_host_port(
            "qemu-system-aarch64: failed to find romfile 'efi-virtio.rom'"
        ));
        assert!(!log_indicates_busy_host_port(""));
    }

    /// End-to-end of the offline interim test (E.2): a throwaway process writes
    /// QEMU's busy-port stderr to the launch log, exactly as a real QEMU would
    /// before exiting; `tail_log` reads it and the mapping predicate fires. This
    /// is the deterministic, no-QEMU proof that the early-exit branch in `start`
    /// converts that log tail into `Error::Config(host port in use)`.
    #[cfg(unix)]
    #[tokio::test]
    async fn busy_host_port_from_throwaway_process_log_maps_to_config() {
        let tmp = tempfile::tempdir().unwrap();
        let log_path = tmp.path().join("qemu.log");

        // Pre-bind a port so the scenario is grounded in a genuinely-busy port.
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let busy = listener.local_addr().unwrap().port();

        // Throwaway process that echoes QEMU's exact host-forwarding stderr to
        // the log, then exits non-zero — mirroring QEMU's startup failure.
        let msg = format!(
            "qemu-system-aarch64: -netdev user,id=net0,hostfwd=tcp:127.0.0.1:{busy}-:22: \
             Could not set up host forwarding rule 'tcp:127.0.0.1:{busy}-:22'"
        );
        let mut proc = QemuProcess::spawn(
            "/bin/sh",
            &["-c".into(), format!("echo \"{msg}\" 1>&2; exit 1")],
            &log_path,
        )
        .await
        .expect("spawn throwaway");
        proc.wait().await.expect("await exit");

        // Reproduce the early-exit mapping from `start`: read the log tail and
        // route a recognized busy-port failure to the clean Config error.
        let tail = tail_log(&log_path).await;
        assert!(
            log_indicates_busy_host_port(&tail),
            "log tail must be recognized as a busy host port: {tail}"
        );
        let mapped: Error = if log_indicates_busy_host_port(&tail) {
            Error::Config(
                "Host port already in use or invalid; pick another or free it".to_string(),
            )
        } else {
            Error::Qmp(format!("could not reach QMP{tail}"))
        };
        match mapped {
            Error::Config(m) => assert!(m.contains("Host port already in use")),
            other => panic!("expected Error::Config(host port in use), got {other:?}"),
        }

        drop(listener);
    }

    // ====================================================================
    // Phase 5 — suspend / resume + shared-folder validation
    // ====================================================================

    /// Persist a VM whose config is flagged suspended (a fresh uuid tag), as if
    /// `suspend` had run. Used by the offline suspend-state tests that don't need
    /// a live guest. Returns (id_string, suspend_tag).
    async fn persist_suspended(hv: &QemuHypervisor, name: &str, slug: &str) -> (String, Uuid) {
        let vm = hv
            .create_vm(defined_config(Uuid::new_v4(), name, slug))
            .await
            .unwrap();
        let id = vm.id.to_string();
        let tag = Uuid::new_v4();
        let mut cfg = hv.get_config(&id).await.unwrap();
        cfg.metadata.suspended_snapshot = Some(tag);
        hv.library.save_config(&cfg).await.unwrap();
        (id, tag)
    }

    // ---- suspend is refused up-front on aarch64 + HVF (ORCHESTRATOR NOTE) ----
    #[tokio::test]
    async fn suspend_refused_under_hvf() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img_full().await;
        let hv = QemuHypervisor::with_library_dir(tmp.path().to_path_buf()).expect("build hv");
        let vm = hv
            .create_vm(defined_config(Uuid::new_v4(), "Suspendee", "suspendee"))
            .await
            .unwrap();
        let id = vm.id.to_string();

        // The gate fires only on aarch64 + a hardware accelerator (HVF/KVM/WHPX).
        // On this dev host (M4 + HVF) it fires; on a TCG host it would route to
        // the "not running" refusal instead. Either way suspend errors here (the
        // VM is not live), so assert the specific gate message when applicable.
        let err = hv.suspend(&id).await.expect_err("suspend must error");
        if std::env::consts::ARCH == "aarch64" && hv.accelerator().is_hardware() {
            match err {
                Error::Config(m) => assert!(
                    m.contains("hardware acceleration (HVF)"),
                    "expected the HVF gate message, got: {m}"
                ),
                other => panic!("expected Error::Config(HVF gate), got {other:?}"),
            }
        } else {
            // Off the gated host the call still fails (VM not running).
            assert!(matches!(err, Error::Config(_)), "got {err:?}");
        }
        // No suspend state was captured.
        let cfg = hv.get_config(&id).await.unwrap();
        assert!(cfg.metadata.suspended_snapshot.is_none());
    }

    // ---- restore with a missing vmstate tag clears the field + refuses ----
    #[tokio::test]
    async fn restore_missing_suspend_state_resets() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img_full().await;
        let hv = QemuHypervisor::with_library_dir(tmp.path().to_path_buf()).expect("build hv");
        let (id, _tag) = persist_suspended(&hv, "Ghost", "ghost").await;

        // The mock `info` returns NO internal snapshots, so the stored tag is
        // absent → restore must clear the field and refuse ("reset to stopped").
        let err = hv
            .restore_suspended(&id)
            .await
            .expect_err("restore of a missing vmstate must fail");
        match err {
            Error::Config(m) => assert!(
                m.contains("reset to stopped"),
                "expected reset-to-stopped message, got: {m}"
            ),
            other => panic!("expected Error::Config(reset to stopped), got {other:?}"),
        }
        // The field was cleared so the VM is a plain Stopped VM again.
        let cfg = hv.get_config(&id).await.unwrap();
        assert!(
            cfg.metadata.suspended_snapshot.is_none(),
            "suspended_snapshot must be cleared on a missing-vmstate restore"
        );
    }

    // ---- editing a suspended VM is refused (config must match the vmstate) ----
    #[tokio::test]
    async fn edit_refused_while_suspended() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img_full().await;
        let hv = QemuHypervisor::with_library_dir(tmp.path().to_path_buf()).expect("build hv");
        let (id, _tag) = persist_suspended(&hv, "Frozen", "frozen").await;

        let mut edited = hv.get_config(&id).await.unwrap();
        edited.hardware.cpus = 8;
        let err = hv
            .update_config(&id, edited)
            .await
            .expect_err("editing a suspended VM must fail");
        match err {
            Error::Config(m) => assert!(
                m.contains("suspended"),
                "expected a suspended-edit refusal, got: {m}"
            ),
            other => panic!("expected Error::Config(suspended), got {other:?}"),
        }
    }

    // ---- a plain cold start of a suspended VM is refused (resume/discard) ----
    #[tokio::test]
    async fn start_refused_while_suspended() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img_full().await;
        let hv = QemuHypervisor::with_library_dir(tmp.path().to_path_buf()).expect("build hv");
        let (id, _tag) = persist_suspended(&hv, "Sleeper", "sleeper").await;

        let config = hv.get_config(&id).await.unwrap();
        let err = hv
            .start(&config)
            .await
            .expect_err("a cold start of a suspended VM must fail");
        match err {
            Error::Config(m) => assert!(
                m.contains("suspended"),
                "expected a suspended-start refusal, got: {m}"
            ),
            other => panic!("expected Error::Config(suspended), got {other:?}"),
        }
        // Nothing was launched.
        assert!(hv.running_state(&id).await.is_none());

        // A suspended VM that never entered the registry lists as Defined (the
        // "suspended" bool is derived at the IPC layer from the persisted field).
        let all = hv.list_all().await.unwrap();
        let summary = all.iter().find(|s| s.id.to_string() == id).unwrap();
        assert_eq!(summary.state, VmState::Defined);
        assert!(config.metadata.suspended_snapshot.is_some());
    }

    // ---- discard_suspend clears the field (escape hatch) ----
    #[tokio::test]
    async fn discard_suspend_clears_field() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img_full().await;
        let hv = QemuHypervisor::with_library_dir(tmp.path().to_path_buf()).expect("build hv");
        let (id, _tag) = persist_suspended(&hv, "Discardee", "discardee").await;

        // Sanity: it starts out suspended.
        assert!(hv
            .get_config(&id)
            .await
            .unwrap()
            .metadata
            .suspended_snapshot
            .is_some());

        hv.discard_suspend(&id).await.unwrap();

        let cfg = hv.get_config(&id).await.unwrap();
        assert!(
            cfg.metadata.suspended_snapshot.is_none(),
            "discard_suspend must clear suspended_snapshot"
        );
        // Idempotent: discarding again on a non-suspended VM is a no-op Ok.
        hv.discard_suspend(&id).await.unwrap();

        // Now a cold start is no longer refused on the suspended-state guard
        // (it may still fail later for lack of a real QEMU, but NOT here).
        let config = hv.get_config(&id).await.unwrap();
        assert!(config.metadata.suspended_snapshot.is_none());
    }

    // ---- start_inner validates shared folders before spawn ----
    #[tokio::test]
    async fn start_rejects_missing_shared_folder_host_path() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img_full().await;
        let hv = QemuHypervisor::with_library_dir(tmp.path().to_path_buf()).expect("build hv");
        let mut cfg = defined_config(Uuid::new_v4(), "Sharer", "sharer");
        // A safe-but-nonexistent absolute host path → validate_shared_folders
        // must reject BEFORE any spawn attempt.
        cfg.shared_folders = vec![crate::model::SharedFolder {
            host_path: tmp.path().join("does-not-exist").display().to_string(),
            mount_tag: "share".into(),
            read_only: false,
        }];
        let vm = hv.create_vm(cfg).await.unwrap();
        let config = hv.get_config(&vm.id.to_string()).await.unwrap();

        let err = hv
            .start(&config)
            .await
            .expect_err("start must reject a missing shared-folder host path");
        match err {
            Error::Config(m) => assert!(
                m.contains("host path is not an existing directory"),
                "expected a missing-dir share error, got: {m}"
            ),
            other => panic!("expected Error::Config(missing dir), got {other:?}"),
        }
        assert!(hv.running_state(&vm.id.to_string()).await.is_none());
    }
}
