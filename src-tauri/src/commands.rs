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
use vmforge_core::host::{self, Accelerator, HostCapabilities};
use vmforge_core::model::{
    DiskSpec, Hardware, NetworkConfig, NetworkMode, PortForward, VmConfig, VmState,
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
}

#[derive(Debug, Serialize)]
pub struct VmConfigDto {
    pub id: String,
    pub name: String,
    pub hardware: HardwareDto,
    pub disks: Vec<DiskDto>,
    pub network: NetworkDto,
    pub iso: Option<String>,
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

impl From<VmConfig> for VmConfigDto {
    fn from(c: VmConfig) -> Self {
        VmConfigDto {
            id: c.id.to_string(),
            name: c.name,
            hardware: c.hardware.into(),
            disks: c.disks.into_iter().map(DiskDto::from).collect(),
            network: c.network.into(),
            iso: c.iso,
        }
    }
}

/// Probe host virtualization capabilities (first-run screen).
#[tauri::command]
pub async fn probe_host() -> Result<HostCapabilities, String> {
    host::probe().map_err(|e| e.to_string())
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

#[cfg(test)]
mod tests {
    //! Wire-shape contract for the IPC DTOs. These pin the EXACT JSON field
    //! names and enum strings the frontend (`src/lib/ipc.ts`) depends on, so a
    //! Rust-side rename/casing change breaks here instead of silently at
    //! runtime. Runs under `cargo test -p vmforge` / `--workspace`.
    use super::*;
    use serde_json::{json, Value};

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
                "state"
            ]
        );
        assert_eq!(v["state"], json!("running")); // VmState lowercase
        assert_eq!(v["accelerator"], json!("hvf")); // Accelerator lowercase
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
        };
        let v = serde_json::to_value(&dto).unwrap();
        assert_eq!(
            keys(&v),
            ["disks", "hardware", "id", "iso", "name", "network"]
        );
        assert_eq!(keys(&v["hardware"]), ["cpus", "memory_mib"]);
        assert_eq!(keys(&v["network"]), ["mac", "mode", "port_forwards"]);
        assert_eq!(v["network"]["mode"], json!("host-only")); // NetworkMode kebab
        assert_eq!(keys(&v["disks"][0]), ["backing", "path", "size_gib"]);
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

        let update: UpdateVmRequest =
            serde_json::from_str(r#"{"name":"n2","hardware":{"cpus":4,"memory_mib":4096}}"#)
                .unwrap();
        assert_eq!(update.hardware.cpus, 4);
    }
}
