//! VMForge Tauri shell.
//!
//! Intentionally thin: this crate owns window + IPC wiring only. All
//! virtualization logic lives in `vmforge-core` behind the `Hypervisor`
//! trait. The frontend must never reach QEMU directly — every engine
//! interaction crosses this IPC boundary. See `CLAUDE.md` ("Engine
//! boundary is sacred").

mod commands;

use commands::AppState;
use std::sync::Arc;
use vmforge_core::QemuHypervisor;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "vmforge=debug,vmforge_core=debug".into()),
        )
        .init();

    let hv = Arc::new(QemuHypervisor::new().expect("failed to initialize QEMU hypervisor"));

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .manage(AppState { hv })
        .invoke_handler(tauri::generate_handler![
            commands::probe_host,
            commands::network_capabilities,
            commands::create_vm,
            commands::list_vms,
            commands::get_vm,
            commands::update_vm,
            commands::delete_vm,
            commands::start_vm,
            commands::rename_vm,
            commands::open_console,
            commands::vm_state,
            commands::power_off,
            commands::force_off,
            commands::pause_vm,
            commands::resume_vm,
            commands::suspend_vm,
            commands::restore_vm,
            commands::discard_suspend,
            commands::list_snapshots,
            commands::create_snapshot,
            commands::restore_snapshot,
            commands::delete_snapshot,
            commands::clone_vm,
        ])
        .run(tauri::generate_context!())
        .expect("error while running VMForge");
}
