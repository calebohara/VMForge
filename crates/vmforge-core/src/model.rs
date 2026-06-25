//! Domain model: VM configuration and lifecycle state.
//!
//! These types are serialized to each VM's `vmforge.toml` (storage-engineer
//! owns persistence), consumed by the engine, and surfaced to the UI via
//! IPC. Keep them serde-stable and platform-neutral — no raw paths baked
//! to one OS.

use crate::host::Accelerator;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub type VmId = Uuid;

fn default_schema_version() -> u32 {
    1
}

/// Free-form, non-engine metadata persisted alongside hardware config.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VmMetadata {
    /// RFC3339 UTC timestamp of creation.
    #[serde(default)]
    pub created_at: Option<String>,
    /// RFC3339 UTC timestamp of last config write.
    #[serde(default)]
    pub updated_at: Option<String>,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub os_hint: Option<String>,
}

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

/// One node in the VMForge snapshot tree. This is OUR overlay metadata,
/// persisted in `vmforge.toml`; qcow2 internal snapshots remain authoritative
/// for existence and payload. Join key: the qcow2 `tag` equals this snapshot's
/// `id.to_string()`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// Stable id AND qcow2 tag (`id.to_string()`).
    pub id: Uuid,
    pub name: String,
    /// Parent snapshot id; `None` => a top-level (tree-root) snapshot.
    #[serde(default)]
    pub parent: Option<Uuid>,
    /// RFC3339 UTC timestamp of creation.
    pub created_at: String,
    /// `true` when RAM/device state was captured (a live snapshot).
    #[serde(default)]
    pub has_vm_state: bool,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub vm_state_size: u64,
}

/// A reconciled snapshot node: our metadata plus presence in the qcow2 image
/// and the resolved child links. Produced on-demand by the read path, never
/// persisted.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotNode {
    #[serde(flatten)]
    pub meta: Snapshot,
    /// Whether an internal qcow2 snapshot with this tag currently exists.
    pub present_in_qcow2: bool,
    pub children: Vec<Uuid>,
}

/// Full persisted configuration for one VM.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmConfig {
    pub id: VmId,
    pub name: String,
    /// On-disk schema version of this VM's `vmforge.toml`.
    #[serde(default = "default_schema_version")]
    pub schema_version: u32,
    /// Sanitized directory slug under the library root. Immutable for the
    /// VM's life; the directory never moves on rename.
    #[serde(default)]
    pub dir_slug: String,
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
    #[serde(default)]
    pub metadata: VmMetadata,
    /// VMForge snapshot-tree overlay metadata. Phase-2 configs load with an
    /// empty array (additive, no schema bump).
    #[serde(default)]
    pub snapshots: Vec<Snapshot>,
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
    /// Runtime-only — never persisted to `vmforge.toml`.
    #[serde(skip)]
    pub vnc_port: Option<u16>,
}

/// Lightweight view for the library/dashboard list.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VmSummary {
    pub id: VmId,
    pub name: String,
    pub state: VmState,
    /// Accelerator this host will use (derived server-side, not persisted).
    pub accelerator: Accelerator,
    /// Whether the guest arch differs from the host (emulated). Always
    /// `false` in Phase 2 — no `guest_arch` field yet.
    pub emulated: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (3) Wire casing is snake/kebab/lowercase — never camelCase. This guards
    /// the JSON boundary contract (decision #9). The full DTO snapshot lives in
    /// the IPC crate; here we pin the shared enums the DTOs embed.
    #[test]
    fn json_wire_casing() {
        assert_eq!(
            serde_json::to_string(&VmState::Running).unwrap(),
            "\"running\""
        );
        assert_eq!(
            serde_json::to_string(&VmState::Defined).unwrap(),
            "\"defined\""
        );
        assert_eq!(
            serde_json::to_string(&VmState::Stopped).unwrap(),
            "\"stopped\""
        );
        assert_eq!(
            serde_json::to_string(&NetworkMode::User).unwrap(),
            "\"user\""
        );
        assert_eq!(
            serde_json::to_string(&NetworkMode::HostOnly).unwrap(),
            "\"host-only\""
        );
        assert_eq!(
            serde_json::to_string(&NetworkMode::Bridged).unwrap(),
            "\"bridged\""
        );
        assert_eq!(serde_json::to_string(&Accelerator::Hvf).unwrap(), "\"hvf\"");
        assert_eq!(serde_json::to_string(&Accelerator::Kvm).unwrap(), "\"kvm\"");
        assert_eq!(serde_json::to_string(&Accelerator::Tcg).unwrap(), "\"tcg\"");
    }

    /// (E.1 #3) json_ipc_round_trip — the IPC boundary contract. Every enum and
    /// DTO-equivalent struct that crosses Tauri IPC must serialize with exact,
    /// stable snake/kebab/lowercase field names and enum strings. The DTOs
    /// themselves live in `src-tauri`, but they are structural mirrors of these
    /// model types (HardwareDto↔Hardware, DiskDto↔DiskSpec, NetworkDto↔
    /// NetworkConfig, VmConfigDto/VmListItem fields), so pinning the model
    /// boundary here pins the wire contract. Decision #9; never camelCase.
    #[test]
    fn json_ipc_round_trip() {
        // Boundary enums — exact wire strings.
        assert_eq!(
            serde_json::to_string(&VmState::Running).unwrap(),
            "\"running\""
        );
        assert_eq!(
            serde_json::to_string(&NetworkMode::HostOnly).unwrap(),
            "\"host-only\""
        );
        assert_eq!(serde_json::to_string(&Accelerator::Hvf).unwrap(), "\"hvf\"");

        // Round-trip each enum (deserialize the wire string back).
        assert_eq!(
            serde_json::from_str::<VmState>("\"running\"").unwrap(),
            VmState::Running
        );
        assert_eq!(
            serde_json::from_str::<NetworkMode>("\"host-only\"").unwrap(),
            NetworkMode::HostOnly
        );
        assert_eq!(
            serde_json::from_str::<Accelerator>("\"hvf\"").unwrap(),
            Accelerator::Hvf
        );

        // Helper: assert an object has exactly `keys` (order-insensitive).
        fn assert_keys(v: &serde_json::Value, keys: &[&str]) {
            let obj = v.as_object().expect("expected JSON object");
            assert_eq!(obj.len(), keys.len(), "field count mismatch: {obj:?}");
            for k in keys {
                assert!(obj.contains_key(*k), "missing field {k}: {obj:?}");
            }
        }

        // Hardware (↔ HardwareDto).
        let hw = Hardware {
            cpus: 2,
            memory_mib: 2048,
        };
        assert_keys(&serde_json::to_value(&hw).unwrap(), &["cpus", "memory_mib"]);

        // DiskSpec (↔ DiskDto). `backing` is always emitted (Option, no skip).
        let disk = DiskSpec {
            path: "disk.qcow2".into(),
            size_gib: 8,
            backing: None,
        };
        assert_keys(
            &serde_json::to_value(&disk).unwrap(),
            &["path", "size_gib", "backing"],
        );

        // PortForward.
        let pf = PortForward {
            host: 2222,
            guest: 22,
            udp: false,
        };
        assert_keys(
            &serde_json::to_value(&pf).unwrap(),
            &["host", "guest", "udp"],
        );

        // NetworkConfig (↔ NetworkDto) — embeds the kebab-case mode enum.
        let net = NetworkConfig {
            mode: NetworkMode::HostOnly,
            mac: None,
            port_forwards: vec![pf],
        };
        let net_v = serde_json::to_value(&net).unwrap();
        assert_keys(&net_v, &["mode", "mac", "port_forwards"]);
        assert_eq!(net_v["mode"], "host-only");

        // VmSummary — carries the lowercase state + accelerator enums.
        let summary = VmSummary {
            id: Uuid::nil(),
            name: "x".into(),
            state: VmState::Running,
            accelerator: Accelerator::Hvf,
            emulated: false,
        };
        let sum_v = serde_json::to_value(&summary).unwrap();
        assert_keys(&sum_v, &["id", "name", "state", "accelerator", "emulated"]);
        assert_eq!(sum_v["state"], "running");
        assert_eq!(sum_v["accelerator"], "hvf");
    }

    /// VmSummary serializes with exactly its snake_case field names.
    #[test]
    fn vm_summary_json_fields() {
        let s = VmSummary {
            id: Uuid::nil(),
            name: "x".into(),
            state: VmState::Defined,
            accelerator: Accelerator::Tcg,
            emulated: false,
        };
        let v: serde_json::Value = serde_json::to_value(&s).unwrap();
        let obj = v.as_object().unwrap();
        for key in ["id", "name", "state", "accelerator", "emulated"] {
            assert!(obj.contains_key(key), "missing key {key}");
        }
        assert_eq!(obj.len(), 5);
    }
}
