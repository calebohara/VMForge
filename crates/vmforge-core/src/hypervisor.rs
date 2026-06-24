//! The [`Hypervisor`] trait — VMForge's engine abstraction.
//!
//! The QEMU implementation (hypervisor-engineer, Phase 1) is the default
//! backend. A macOS-native Virtualization.framework backend can later slot
//! in behind this same trait without touching the IPC layer or the UI.
//!
//! The frontend reaches an implementor only through Tauri IPC — never
//! directly. That is the boundary that keeps the backend swappable.

use crate::error::Result;
use crate::host::Accelerator;
use crate::model::{VmConfig, VmState};
use async_trait::async_trait;

/// Engine that can launch and control VMs. Implemented by the QEMU
/// backend; intended to be used as `dyn Hypervisor` so backends are
/// interchangeable at runtime.
#[async_trait]
pub trait Hypervisor: Send + Sync {
    /// Accelerator this engine will use on the current host.
    fn accelerator(&self) -> Accelerator;

    /// Launch a VM from its config: spawn the engine process and open the
    /// control channel (QMP for QEMU).
    async fn start(&self, config: &VmConfig) -> Result<()>;

    /// Graceful ACPI shutdown (QMP `system_powerdown`).
    async fn shutdown(&self, id: &str) -> Result<()>;

    /// Force-terminate the engine process.
    async fn kill(&self, id: &str) -> Result<()>;

    /// Pause execution (QMP `stop`).
    async fn pause(&self, id: &str) -> Result<()>;

    /// Resume execution (QMP `cont`).
    async fn resume(&self, id: &str) -> Result<()>;

    /// Current lifecycle state (QMP `query-status`).
    async fn state(&self, id: &str) -> Result<VmState>;
}
