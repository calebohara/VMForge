//! Domain model: VM configuration and lifecycle state.
//!
//! These types are serialized to each VM's `vmforge.toml` (storage-engineer
//! owns persistence), consumed by the engine, and surfaced to the UI via
//! IPC. Keep them serde-stable and platform-neutral — no raw paths baked
//! to one OS.

use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type VmId = Uuid;

/// VM lifecycle. Transitions are driven by the engine and reflected from
/// QMP `query-status`/events. See hypervisor-engineer's state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VmState {
    Defined,
    Starting,
    Running,
    Paused,
    Stopping,
    Stopped,
    Error,
}

/// Full persisted configuration for one VM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmConfig {
    pub id: VmId,
    pub name: String,
    #[serde(default)]
    pub hardware: Hardware,
    #[serde(default)]
    pub disks: Vec<DiskSpec>,
    #[serde(default)]
    pub network: NetworkConfig,
    #[serde(default)]
    pub display: DisplayConfig,
    /// Path to boot/install ISO, if any.
    #[serde(default)]
    pub iso: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hardware {
    pub cpus: u32,
    pub memory_mib: u32,
}

impl Default for Hardware {
    fn default() -> Self {
        Self {
            cpus: 2,
            memory_mib: 4096,
        }
    }
}

/// A virtual disk. `backing` set => linked clone (qemu-img -b).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiskSpec {
    /// qcow2 path, relative to the VM directory.
    pub path: String,
    pub size_gib: u32,
    #[serde(default)]
    pub backing: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkMode {
    /// User-mode NAT (`-netdev user`). Zero privileges. MVP default.
    #[default]
    User,
    /// Bridged to a host interface. Needs elevated permissions.
    Bridged,
    /// Host-only network. Needs elevated permissions.
    HostOnly,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct NetworkConfig {
    #[serde(default)]
    pub mode: NetworkMode,
    #[serde(default)]
    pub mac: Option<String>,
    /// Host->guest port forwards (NAT mode).
    #[serde(default)]
    pub port_forwards: Vec<PortForward>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortForward {
    pub host: u16,
    pub guest: u16,
    #[serde(default)]
    pub udp: bool,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct DisplayConfig {
    /// Host VNC port the noVNC bridge connects to (assigned at launch).
    #[serde(default)]
    pub vnc_port: Option<u16>,
}

/// Lightweight view for the library/dashboard list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmSummary {
    pub id: VmId,
    pub name: String,
    pub state: VmState,
}
