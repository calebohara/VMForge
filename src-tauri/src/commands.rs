//! Tauri IPC command surface.
//!
//! Each command is a thin adapter over `vmforge-core`: marshal args, call the
//! engine through the `Hypervisor` trait (+ a few inherent methods), and map
//! [`vmforge_core::Error`] to `String`. No business logic here — every command
//! delegates to a single `hv` method (the `list_vms` join is the one place we
//! stitch two reads together, and it stays a pure mapping).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;
use vmforge_core::host::{self, Accelerator, HostCapabilities, NetworkCapabilities};
use vmforge_core::model::{
    DiskSpec, Hardware, NetworkConfig, NetworkMode, PortForward, SharedFolder, SnapshotNode,
    VmConfig, VmState,
};
use vmforge_core::{Hypervisor, QemuHypervisor};

/// Shared engine handle, managed by Tauri.
pub struct AppState {
    pub hv: Arc<QemuHypervisor>,
}

// ---- DTOs (Deserialize req / Serialize resp, serde snake_case) ----

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct HardwareDto {
    pub cpus: u32,
    pub memory_mib: u32,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DiskDto {
    pub path: String,
    pub size_gib: u32,
    #[serde(default)]
    pub backing: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct NetworkDto {
    pub mode: NetworkMode,
    #[serde(default)]
    pub mac: Option<String>,
    #[serde(default)]
    pub port_forwards: Vec<PortForward>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SharedFolderDto {
    pub host_path: String,
    pub mount_tag: String,
    #[serde(default)]
    pub read_only: bool,
}

#[derive(Debug, Deserialize)]
pub struct CreateVmRequest {
    pub name: String,
    pub hardware: HardwareDto,
    pub disk_gib: u32,
    #[serde(default)]
    pub network: Option<NetworkDto>,
    #[serde(default)]
    pub iso: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct UpdateVmRequest {
    pub name: String,
    pub hardware: HardwareDto,
    #[serde(default)]
    pub network: Option<NetworkDto>,
    #[serde(default)]
    pub iso: Option<String>,
    #[serde(default)]
    pub shared_folders: Vec<SharedFolderDto>,
}

#[derive(Debug, Serialize)]
pub struct VmConfigDto {
    pub id: String,
    pub name: String,
    pub hardware: HardwareDto,
    pub disks: Vec<DiskDto>,
    pub network: NetworkDto,
    pub iso: Option<String>,
    pub shared_folders: Vec<SharedFolderDto>,
    pub suspended: bool,
}

#[derive(Debug, Serialize)]
pub struct VmListItem {
    pub id: String,
    pub name: String,
    pub state: VmState,
    pub accelerator: Accelerator,
    pub emulated: bool,
    pub cpus: u32,
    pub memory_mib: u32,
    pub iso: Option<String>,
    pub suspended: bool,
}

#[derive(Debug, Clone, Serialize)]
pub struct SnapshotDto {
    pub snapshot_id: String,
    pub name: String,
    pub parent_id: Option<String>,
    pub created_at: String,
    pub has_vm_state: bool,
    pub vm_state_size: u64,
    pub present_in_qcow2: bool,
}

// ---- DTO <-> model conversions ----

impl From<HardwareDto> for Hardware {
    fn from(d: HardwareDto) -> Self {
        Hardware {
            cpus: d.cpus,
            memory_mib: d.memory_mib,
        }
    }
}

impl From<Hardware> for HardwareDto {
    fn from(h: Hardware) -> Self {
        HardwareDto {
            cpus: h.cpus,
            memory_mib: h.memory_mib,
        }
    }
}

impl From<DiskSpec> for DiskDto {
    fn from(d: DiskSpec) -> Self {
        DiskDto {
            path: d.path,
            size_gib: d.size_gib,
            backing: d.backing,
        }
    }
}

impl From<NetworkDto> for NetworkConfig {
    fn from(d: NetworkDto) -> Self {
        NetworkConfig {
            mode: d.mode,
            mac: d.mac,
            port_forwards: d.port_forwards,
        }
    }
}

impl From<NetworkConfig> for NetworkDto {
    fn from(n: NetworkConfig) -> Self {
        NetworkDto {
            mode: n.mode,
            mac: n.mac,
            port_forwards: n.port_forwards,
        }
    }
}

impl From<SharedFolderDto> for SharedFolder {
    fn from(d: SharedFolderDto) -> Self {
        SharedFolder {
            host_path: d.host_path,
            mount_tag: d.mount_tag,
            read_only: d.read_only,
        }
    }
}

impl From<SharedFolder> for SharedFolderDto {
    fn from(s: SharedFolder) -> Self {
        SharedFolderDto {
            host_path: s.host_path,
            mount_tag: s.mount_tag,
            read_only: s.read_only,
        }
    }
}

impl From<VmConfig> for VmConfigDto {
    fn from(c: VmConfig) -> Self {
        let suspended = c.metadata.suspended_snapshot.is_some();
        VmConfigDto {
            id: c.id.to_string(),
            name: c.name,
            hardware: c.hardware.into(),
            disks: c.disks.into_iter().map(DiskDto::from).collect(),
            network: c.network.into(),
            iso: c.iso,
            shared_folders: c
                .shared_folders
                .into_iter()
                .map(SharedFolderDto::from)
                .collect(),
            suspended,
        }
    }
}

impl From<SnapshotNode> for SnapshotDto {
    fn from(n: SnapshotNode) -> Self {
        SnapshotDto {
            snapshot_id: n.meta.id.to_string(),
            name: n.meta.name,
            parent_id: n.meta.parent.map(|p| p.to_string()),
            created_at: n.meta.created_at,
            has_vm_state: n.meta.has_vm_state,
            vm_state_size: n.meta.vm_state_size,
            present_in_qcow2: n.present_in_qcow2,
        }
    }
}

/// Probe host virtualization capabilities (first-run screen).
#[tauri::command]
pub async fn probe_host() -> Result<HostCapabilities, String> {
    host::probe().map_err(|e| e.to_string())
}

/// Per-mode networking capabilities (user available; bridged/host-only gated
/// behind elevated permissions in this build). Drives the network-form mode
/// picker; shares the per-OS reason with the launch-reject path so the UI and
/// engine never disagree. Infallible probe, so no error mapping needed.
#[tauri::command]
pub async fn network_capabilities() -> Result<NetworkCapabilities, String> {
    Ok(host::probe_network(std::env::consts::OS))
}

/// Persist the user's "Locate QEMU…" directory override (Phase 6 — D3).
///
/// The first-run gate's directory picker calls this with the chosen directory;
/// it writes `qemu_dir` to the settings file consumed by
/// `vmforge_core::qemu_resolve::resolve_qemu_binary`. The gate then re-invokes
/// `probe_host` (no app restart) to pick up the new location. An empty/whitespace
/// string clears the override (falls back to `$PATH` + install prefixes).
///
/// This is the only first-run-UX command: the resolver reads a settings file the
/// webview cannot write cross-platform, so a thin IPC seam persists it (spec §C).
#[tauri::command(rename_all = "snake_case")]
pub async fn set_qemu_dir(dir: String) -> Result<(), String> {
    let trimmed = dir.trim();
    let settings = vmforge_core::settings::Settings {
        qemu_dir: if trimmed.is_empty() {
            None
        } else {
            Some(std::path::PathBuf::from(trimmed))
        },
    };
    vmforge_core::settings::save(&settings).map_err(|e| e.to_string())
}

/// Persist a new VM (dir + `vmforge.toml` + qcow2). Does NOT launch.
#[tauri::command]
pub async fn create_vm(
    state: State<'_, AppState>,
    req: CreateVmRequest,
) -> Result<VmConfigDto, String> {
    let config = VmConfig {
        id: Uuid::new_v4(),
        name: req.name,
        schema_version: 1,
        // Slug is assigned by the library on create.
        dir_slug: String::new(),
        hardware: Hardware {
            cpus: req.hardware.cpus.max(1),
            memory_mib: req.hardware.memory_mib.max(256),
        },
        disks: vec![DiskSpec {
            path: "disk.qcow2".into(),
            size_gib: req.disk_gib.max(1),
            backing: None,
        }],
        network: req.network.map(NetworkConfig::from).unwrap_or_default(),
        display: Default::default(),
        iso: req.iso.filter(|s| !s.is_empty()),
        metadata: Default::default(),
        snapshots: Vec::new(),
        shared_folders: Vec::new(),
    };
    state
        .hv
        .create_vm(config)
        .await
        .map(VmConfigDto::from)
        .map_err(|e| e.to_string())
}

/// Library view: every persisted VM with live-state overlay, joined with its
/// hardware/iso detail. Sorted by case-insensitive name.
#[tauri::command]
pub async fn list_vms(state: State<'_, AppState>) -> Result<Vec<VmListItem>, String> {
    // Single library scan: summaries + the parsed configs to join detail from.
    let (summaries, configs) = state
        .hv
        .list_all_detailed()
        .await
        .map_err(|e| e.to_string())?;
    let detail: HashMap<String, VmConfig> =
        configs.into_iter().map(|c| (c.id.to_string(), c)).collect();

    let mut items: Vec<VmListItem> = summaries
        .into_iter()
        .map(|s| {
            let id = s.id.to_string();
            let cfg = detail.get(&id);
            VmListItem {
                id,
                name: s.name,
                state: s.state,
                accelerator: s.accelerator,
                emulated: s.emulated,
                cpus: cfg.map(|c| c.hardware.cpus).unwrap_or(0),
                memory_mib: cfg.map(|c| c.hardware.memory_mib).unwrap_or(0),
                iso: cfg.and_then(|c| c.iso.clone()),
                suspended: cfg
                    .map(|c| c.metadata.suspended_snapshot.is_some())
                    .unwrap_or(false),
            }
        })
        .collect();
    items.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    Ok(items)
}

/// Load a single persisted config by id.
#[tauri::command]
pub async fn get_vm(state: State<'_, AppState>, id: String) -> Result<VmConfigDto, String> {
    state
        .hv
        .get_config(&id)
        .await
        .map(VmConfigDto::from)
        .map_err(|e| e.to_string())
}

/// Update editable fields (rejected if the VM is live). Disk changes are out of
/// scope for Phase 2: the existing disks are preserved verbatim.
#[tauri::command]
pub async fn update_vm(
    state: State<'_, AppState>,
    id: String,
    req: UpdateVmRequest,
) -> Result<VmConfigDto, String> {
    let mut config = state.hv.get_config(&id).await.map_err(|e| e.to_string())?;
    config.name = req.name;
    config.hardware = req.hardware.into();
    config.network = req.network.map(NetworkConfig::from).unwrap_or_default();
    config.iso = req.iso.filter(|s| !s.is_empty());
    config.shared_folders = req.shared_folders.into_iter().map(Into::into).collect();
    state
        .hv
        .update_config(&id, config)
        .await
        .map(VmConfigDto::from)
        .map_err(|e| e.to_string())
}

/// Delete a VM (rejected if live). `delete_disks` removes the whole directory.
//
// `rename_all = "snake_case"`: the JS wrapper sends `{ id, delete_disks }`, but a
// bare `#[tauri::command]` matches arg keys as camelCase ("deleteDisks") and
// would fail the lookup at runtime. Override #9 forbids only camelCase.
#[tauri::command(rename_all = "snake_case")]
pub async fn delete_vm(
    state: State<'_, AppState>,
    id: String,
    delete_disks: bool,
) -> Result<(), String> {
    state
        .hv
        .delete(&id, delete_disks)
        .await
        .map_err(|e| e.to_string())
}

/// Load the persisted config by id and launch it.
#[tauri::command]
pub async fn start_vm(state: State<'_, AppState>, id: String) -> Result<(), String> {
    let config = state.hv.get_config(&id).await.map_err(|e| e.to_string())?;
    state.hv.start(&config).await.map_err(|e| e.to_string())
}

/// Rename a VM (metadata only; allowed while running).
// `rename_all = "snake_case"` to match the JS `{ id, new_name }` arg key
// (see delete_vm).
#[tauri::command(rename_all = "snake_case")]
pub async fn rename_vm(
    state: State<'_, AppState>,
    id: String,
    new_name: String,
) -> Result<VmConfigDto, String> {
    state
        .hv
        .rename(&id, &new_name)
        .await
        .map(VmConfigDto::from)
        .map_err(|e| e.to_string())
}

/// Start (or reuse) the noVNC bridge; returns the loopback WebSocket port.
#[tauri::command]
pub async fn open_console(state: State<'_, AppState>, id: String) -> Result<u16, String> {
    state.hv.open_console(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn vm_state(state: State<'_, AppState>, id: String) -> Result<VmState, String> {
    state.hv.state(&id).await.map_err(|e| e.to_string())
}

/// Graceful ACPI shutdown (guest must honor it).
#[tauri::command]
pub async fn power_off(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.hv.shutdown(&id).await.map_err(|e| e.to_string())
}

/// Force-terminate the QEMU process.
#[tauri::command]
pub async fn force_off(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.hv.kill(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn pause_vm(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.hv.pause(&id).await.map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn resume_vm(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.hv.resume(&id).await.map_err(|e| e.to_string())
}

/// Suspend a running VM: capture its RAM/device state to the qcow2 vmstate, then
/// terminate the process (Phase 5). Distinct from `pause_vm` (= QMP `stop`).
/// Accelerator-gated below the boundary (refused on aarch64 + HVF).
#[tauri::command]
pub async fn suspend_vm(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state.hv.suspend(&id).await.map_err(|e| e.to_string())
}

/// Restore a suspended VM: relaunch with `-S`, `snapshot-load`, then `cont`
/// (Phase 5). Distinct from `resume_vm` (= QMP `cont` on a paused VM).
#[tauri::command]
pub async fn restore_vm(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state
        .hv
        .restore_suspended(&id)
        .await
        .map_err(|e| e.to_string())
}

/// Discard a suspended VM's captured state without resuming (escape hatch for
/// the "Discard & stop" action): clears the suspend marker and deletes the
/// vmstate snapshot, returning the VM to plain stopped.
#[tauri::command]
pub async fn discard_suspend(state: State<'_, AppState>, id: String) -> Result<(), String> {
    state
        .hv
        .discard_suspend(&id)
        .await
        .map_err(|e| e.to_string())
}

// ---- Snapshots & clones (Phase 3) ----
//
// `rename_all = "snake_case"` on every command with a multi-word arg
// (`snapshot_id`, `new_name`): a bare `#[tauri::command]` matches arg keys as
// camelCase and would fail the runtime lookup against the JS wrappers in
// `src/lib/ipc.ts` (see `delete_vm`). Long-ops are synchronous (A6): the
// Promise resolves on completion.

/// The reconciled snapshot tree for a VM (metadata joined with the qcow2
/// image's internal snapshots). Read on demand, never in the 2s library poll.
#[tauri::command]
pub async fn list_snapshots(
    state: State<'_, AppState>,
    id: String,
) -> Result<Vec<SnapshotDto>, String> {
    state
        .hv
        .list_snapshots(&id)
        .await
        .map(|nodes| nodes.into_iter().map(SnapshotDto::from).collect())
        .map_err(|e| e.to_string())
}

/// Take a snapshot (live via QMP job or offline via `qemu-img`, routed below
/// the boundary). Top-level (`parent = None`), no notes from this surface.
#[tauri::command(rename_all = "snake_case")]
pub async fn create_snapshot(
    state: State<'_, AppState>,
    id: String,
    name: String,
) -> Result<SnapshotDto, String> {
    state
        .hv
        .create_snapshot(&id, &name, None, "")
        .await
        .map(|snap| {
            SnapshotDto::from(SnapshotNode {
                meta: snap,
                present_in_qcow2: true,
                children: Vec::new(),
            })
        })
        .map_err(|e| e.to_string())
}

/// Restore (revert) a snapshot. Disk-only, stopped-only (A7); refused for a
/// live VM or a VM with linked clones, below the boundary.
#[tauri::command(rename_all = "snake_case")]
pub async fn restore_snapshot(
    state: State<'_, AppState>,
    id: String,
    snapshot_id: String,
) -> Result<(), String> {
    let snapshot_id = Uuid::parse_str(&snapshot_id).map_err(|e| e.to_string())?;
    state
        .hv
        .restore_snapshot(&id, snapshot_id)
        .await
        .map_err(|e| e.to_string())
}

/// Delete a snapshot (live or offline), re-parenting its children onto the
/// grandparent below the boundary.
#[tauri::command(rename_all = "snake_case")]
pub async fn delete_snapshot(
    state: State<'_, AppState>,
    id: String,
    snapshot_id: String,
) -> Result<(), String> {
    let snapshot_id = Uuid::parse_str(&snapshot_id).map_err(|e| e.to_string())?;
    state
        .hv
        .delete_snapshot(&id, snapshot_id)
        .await
        .map_err(|e| e.to_string())
}

/// Clone a VM into a brand-new VM (full = deep copy, linked = CoW overlay).
/// Stopped-source-only; returns the new VM's config as a normal `VmConfigDto`.
#[tauri::command(rename_all = "snake_case")]
pub async fn clone_vm(
    state: State<'_, AppState>,
    id: String,
    new_name: String,
    linked: bool,
) -> Result<VmConfigDto, String> {
    state
        .hv
        .clone_vm(&id, &new_name, linked)
        .await
        .map(VmConfigDto::from)
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    //! Wire-shape contract for the IPC DTOs. These pin the EXACT JSON field
    //! names and enum strings the frontend (`src/lib/ipc.ts`) depends on, so a
    //! Rust-side rename/casing change breaks here instead of silently at
    //! runtime. Runs under `cargo test -p vmforge` / `--workspace`.
    use super::*;
    use serde_json::{json, Value};
    use vmforge_core::host::ModeCapability;

    fn keys(v: &Value) -> Vec<String> {
        let mut k: Vec<String> = v.as_object().expect("object").keys().cloned().collect();
        k.sort();
        k
    }

    #[test]
    fn vm_list_item_wire_shape() {
        let item = VmListItem {
            id: "id".into(),
            name: "n".into(),
            state: VmState::Running,
            accelerator: Accelerator::Hvf,
            emulated: false,
            cpus: 2,
            memory_mib: 2048,
            iso: None,
            suspended: false,
        };
        let v = serde_json::to_value(&item).unwrap();
        assert_eq!(
            keys(&v),
            [
                "accelerator",
                "cpus",
                "emulated",
                "id",
                "iso",
                "memory_mib",
                "name",
                "state",
                "suspended"
            ]
        );
        assert_eq!(v["state"], json!("running")); // VmState lowercase
        assert_eq!(v["accelerator"], json!("hvf")); // Accelerator lowercase
        assert_eq!(v["suspended"], json!(false)); // derived suspended-ness
    }

    #[test]
    fn vm_config_dto_wire_shape() {
        let dto = VmConfigDto {
            id: "id".into(),
            name: "n".into(),
            hardware: HardwareDto {
                cpus: 1,
                memory_mib: 512,
            },
            disks: vec![DiskDto {
                path: "disk.qcow2".into(),
                size_gib: 8,
                backing: None,
            }],
            network: NetworkDto {
                mode: NetworkMode::HostOnly,
                mac: None,
                port_forwards: vec![],
            },
            iso: Some("/x.iso".into()),
            shared_folders: vec![SharedFolderDto {
                host_path: "/host/share".into(),
                mount_tag: "share0".into(),
                read_only: true,
            }],
            suspended: true,
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(
            keys(&v),
            [
                "disks",
                "hardware",
                "id",
                "iso",
                "name",
                "network",
                "shared_folders",
                "suspended"
            ]
        );
        assert_eq!(keys(&v["hardware"]), ["cpus", "memory_mib"]);
        assert_eq!(keys(&v["network"]), ["mac", "mode", "port_forwards"]);
        assert_eq!(v["network"]["mode"], json!("host-only")); // NetworkMode kebab
        assert_eq!(keys(&v["disks"][0]), ["backing", "path", "size_gib"]);
        assert_eq!(
            keys(&v["shared_folders"][0]),
            ["host_path", "mount_tag", "read_only"]
        );
        assert_eq!(v["suspended"], json!(true)); // derived suspended-ness
    }

    #[test]
    fn shared_folder_dto_wire_shape() {
        let dto = SharedFolderDto {
            host_path: "/host/share".into(),
            mount_tag: "share0".into(),
            read_only: true,
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(keys(&v), ["host_path", "mount_tag", "read_only"]);
        assert_eq!(v["host_path"], json!("/host/share"));
        assert_eq!(v["mount_tag"], json!("share0"));
        assert_eq!(v["read_only"], json!(true));

        // `read_only` is `#[serde(default)]` (additive, back-compat): a legacy
        // folder without it parses with the export-writable default (false).
        let legacy: SharedFolderDto =
            serde_json::from_str(r#"{"host_path":"/h","mount_tag":"t"}"#).unwrap();
        assert!(!legacy.read_only);
    }

    #[test]
    fn snapshot_dto_wire_shape() {
        let dto = SnapshotDto {
            snapshot_id: "sid".into(),
            name: "snap".into(),
            parent_id: Some("pid".into()),
            created_at: "2026-06-24T00:00:00Z".into(),
            has_vm_state: true,
            vm_state_size: 4096,
            present_in_qcow2: true,
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(
            keys(&v),
            [
                "created_at",
                "has_vm_state",
                "name",
                "parent_id",
                "present_in_qcow2",
                "snapshot_id",
                "vm_state_size"
            ]
        );
        assert_eq!(v["parent_id"], json!("pid"));
        assert_eq!(v["has_vm_state"], json!(true));

        // Top-level snapshot serializes parent_id as JSON null (not absent).
        let root = SnapshotDto {
            parent_id: None,
            ..dto
        };
        let rv = serde_json::to_value(&root).unwrap();
        assert_eq!(rv["parent_id"], Value::Null);
    }

    #[test]
    fn requests_parse_snake_case() {
        let create: CreateVmRequest = serde_json::from_str(
            r#"{"name":"n","hardware":{"cpus":2,"memory_mib":2048},"disk_gib":10,"network":{"mode":"user","mac":null,"port_forwards":[]},"iso":"/x.iso"}"#,
        )
        .unwrap();
        assert_eq!(create.name, "n");
        assert_eq!(create.disk_gib, 10);
        assert_eq!(create.hardware.memory_mib, 2048);

        // network + iso are serde-optional.
        let minimal: CreateVmRequest = serde_json::from_str(
            r#"{"name":"m","hardware":{"cpus":1,"memory_mib":256},"disk_gib":1}"#,
        )
        .unwrap();
        assert!(minimal.network.is_none());
        assert!(minimal.iso.is_none());

        // `shared_folders` is serde-optional on UpdateVmRequest (back-compat):
        // a legacy payload without it parses to an empty Vec.
        let update: UpdateVmRequest =
            serde_json::from_str(r#"{"name":"n2","hardware":{"cpus":4,"memory_mib":4096}}"#)
                .unwrap();
        assert_eq!(update.hardware.cpus, 4);
        assert!(update.shared_folders.is_empty());

        // With shared_folders present: parsed snake_case, read_only optional.
        let update_sf: UpdateVmRequest = serde_json::from_str(
            r#"{"name":"n3","hardware":{"cpus":2,"memory_mib":2048},"shared_folders":[{"host_path":"/h","mount_tag":"t","read_only":true},{"host_path":"/h2","mount_tag":"t2"}]}"#,
        )
        .unwrap();
        assert_eq!(update_sf.shared_folders.len(), 2);
        assert_eq!(update_sf.shared_folders[0].host_path, "/h");
        assert_eq!(update_sf.shared_folders[0].mount_tag, "t");
        assert!(update_sf.shared_folders[0].read_only);
        assert!(!update_sf.shared_folders[1].read_only);
    }

    #[test]
    fn network_capabilities_wire_shape() {
        let caps = NetworkCapabilities {
            modes: vec![ModeCapability {
                mode: NetworkMode::HostOnly,
                available: false,
                requires_elevation: true,
                reason: "needs elevated permissions".into(),
            }],
            port_forward_loopback_only: true,
        };
        let v = serde_json::to_value(&caps).unwrap();
        assert_eq!(keys(&v), ["modes", "port_forward_loopback_only"]);

        let mode = &v["modes"][0];
        assert_eq!(
            keys(mode),
            ["available", "mode", "reason", "requires_elevation"]
        );
        // NetworkMode is kebab-case on the wire.
        assert_eq!(mode["mode"], json!("host-only"));
    }

    #[test]
    fn port_forward_dto_carries_expose_lan() {
        // `update_vm` reflects `expose_lan` automatically: the field rides on the
        // model `PortForward` carried by `NetworkDto`. Round-trip in + out.
        let net: NetworkDto = serde_json::from_str(
            r#"{"mode":"user","mac":null,"port_forwards":[{"host":2222,"guest":22,"udp":false,"expose_lan":true}]}"#,
        )
        .unwrap();
        assert!(net.port_forwards[0].expose_lan);

        let v = serde_json::to_value(&net).unwrap();
        assert_eq!(
            keys(&v["port_forwards"][0]),
            ["expose_lan", "guest", "host", "udp"]
        );
        assert_eq!(v["port_forwards"][0]["expose_lan"], json!(true));

        // `expose_lan` is `#[serde(default)]` (additive, back-compat): a legacy
        // forward without it parses with the loopback-only default (false).
        let legacy: NetworkDto = serde_json::from_str(
            r#"{"mode":"user","mac":null,"port_forwards":[{"host":8080,"guest":80}]}"#,
        )
        .unwrap();
        assert!(!legacy.port_forwards[0].expose_lan);
    }
}
