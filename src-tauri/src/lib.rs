//! VMForge Tauri shell.
//!
//! Intentionally thin: this crate owns window + IPC wiring only. All
//! virtualization logic lives in `vmforge-core` behind the `Hypervisor`
//! trait. The frontend must never reach QEMU directly — every engine
//! interaction crosses this IPC boundary. See `CLAUDE.md` ("Engine
//! boundary is sacred").

mod commands;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "vmforge=debug,vmforge_core=debug".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .invoke_handler(tauri::generate_handler![commands::probe_host])
        .run(tauri::generate_context!())
        .expect("error while running VMForge");
}
