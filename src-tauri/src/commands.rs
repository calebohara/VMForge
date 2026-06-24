//! Tauri IPC command surface.
//!
//! Each command is a thin adapter over `vmforge-core`. Keep business logic
//! out of here — commands should marshal arguments, call the core, and map
//! [`vmforge_core::Error`] to a `String` for the frontend.

use vmforge_core::host::{self, HostCapabilities};

/// Probe the host for virtualization capabilities (OS, arch, accelerator,
/// QEMU availability). Backs the first-run capability screen.
#[tauri::command]
pub async fn probe_host() -> Result<HostCapabilities, String> {
    host::probe().map_err(|e| e.to_string())
}
