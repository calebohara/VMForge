//! Tauri IPC command surface.
//!
//! Each command is a thin adapter over `vmforge-core`: marshal args, call the
//! engine through the `Hypervisor` trait (+ a few inherent methods), and map
//! [`vmforge_core::Error`] to `String`. No business logic here.

use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tauri::State;
use uuid::Uuid;
use vmforge_core::host::{self, HostCapabilities};
use vmforge_core::model::{DiskSpec, Hardware, NetworkConfig, VmConfig, VmState};
use vmforge_core::{Hypervisor, QemuHypervisor};

/// Shared engine handle, managed by Tauri.
pub struct AppState {
    pub hv: Arc<QemuHypervisor>,
}

#[derive(Debug, Deserialize)]
pub struct NewVmRequest {
    pub name: String,
    pub cpus: u32,
    pub memory_mib: u32,
    pub disk_gib: u32,
    pub iso: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct VmDescriptor {
    pub id: String,
    pub name: String,
    pub vnc_port: u16,
}

/// Probe host virtualization capabilities (first-run screen).
#[tauri::command]
pub async fn probe_host() -> Result<HostCapabilities, String> {
    host::probe().map_err(|e| e.to_string())
}

/// Create a VM (qcow2 + config) and launch it. Returns its id and VNC port.
#[tauri::command]
pub async fn create_and_start_vm(
    state: State<'_, AppState>,
    req: NewVmRequest,
) -> Result<VmDescriptor, String> {
    let id = Uuid::new_v4();
    let config = VmConfig {
        id,
        name: req.name.clone(),
        hardware: Hardware {
            cpus: req.cpus.max(1),
            memory_mib: req.memory_mib.max(256),
        },
        disks: vec![DiskSpec {
            path: "disk.qcow2".into(),
            size_gib: req.disk_gib.max(1),
            backing: None,
        }],
        network: NetworkConfig::default(),
        display: Default::default(),
        iso: req.iso.clone(),
    };
    state.hv.start(&config).await.map_err(|e| e.to_string())?;
    let vnc_port = state
        .hv
        .vnc_port(&id.to_string())
        .await
        .map_err(|e| e.to_string())?;
    Ok(VmDescriptor {
        id: id.to_string(),
        name: req.name,
        vnc_port,
    })
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
