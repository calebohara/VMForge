//! # vmforge-core — the engine
//!
//! This crate owns *all* virtualization logic for VMForge: the
//! [`Hypervisor`](hypervisor::Hypervisor) abstraction, the QEMU
//! implementation, the QMP control client, process supervision, disk and
//! network management, and the on-disk config model.
//!
//! ## Engine boundary (sacred)
//! The Tauri shell (`src-tauri`) and the React frontend reach this crate
//! **only** through Tauri IPC commands. Nothing above this boundary ever
//! shells out to QEMU directly. Keeping it this way is what makes the
//! optional macOS-native (Virtualization.framework) backend a drop-in
//! behind the same trait. See `CLAUDE.md`.
//!
//! ## Module ownership (Phase 1 build-out)
//! - [`host`]     host capability probe — *done* (Phase 0)
//! - [`model`]    domain types / config — storage-engineer (+ all)
//! - [`hypervisor`] the trait + QEMU impl — hypervisor-engineer
//! - `qmp`        QMP client — hypervisor-engineer (Phase 1)
//! - `process`    QEMU process supervisor — hypervisor-engineer (Phase 1)
//! - `storage`    qemu-img wrapper — storage-engineer (Phase 1/3)
//! - `network`    netdev model — network-engineer (Phase 1/4)

pub mod console;
pub mod error;
pub mod host;
pub mod hypervisor;
pub mod model;
pub mod paths;
pub mod qemu;
pub mod storage;

pub use error::{Error, Result};
pub use hypervisor::Hypervisor;
pub use qemu::QemuHypervisor;
