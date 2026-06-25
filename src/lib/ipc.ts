// Typed wrappers over the Tauri IPC command surface. These mirror the Rust
// structs in `vmforge-core` / `src-tauri/src/commands.rs` — keep them in sync.
//
// Wire casing is snake_case everywhere (serde): `VmState` lowercase,
// `NetworkMode` kebab-case. Per ORCHESTRATOR OVERRIDE #3, `invoke` arg keys are
// passed snake_case to avoid Tauri camel/snake conversion ambiguity.
import { invoke } from "@tauri-apps/api/core";

// ---- Host capabilities (Phase 1, unchanged) ----

export type Accelerator = "hvf" | "whpx" | "kvm" | "tcg";

export interface QemuBinary {
  name: string;
  present: boolean;
  version: string | null;
}

export interface HostCapabilities {
  os: string;
  arch: string;
  preferred_accelerator: Accelerator;
  available_accelerators: Accelerator[];
  hardware_accelerated: boolean;
  qemu_img: QemuBinary;
  system_binaries: QemuBinary[];
  network: NetworkCapabilities;
  warnings: string[];
}

// ---- Lifecycle state (Phase 1, unchanged) ----

export type VmState =
  | "defined"
  | "starting"
  | "running"
  | "paused"
  | "stopping"
  | "stopped"
  | "error";

// ---- Phase 2 config / library types (mirror serde, snake_case) ----

export type NetworkMode = "user" | "bridged" | "host-only";

export interface PortForward {
  host: number;
  guest: number;
  udp: boolean;
  /**
   * When true, the forward binds all interfaces (`0.0.0.0`), exposing the
   * guest port to the LAN. Defaults to false (loopback-only, `127.0.0.1`).
   */
  expose_lan: boolean;
}

export interface Hardware {
  cpus: number;
  memory_mib: number;
}

export interface Disk {
  path: string;
  size_gib: number;
  backing: string | null;
}

export interface NetworkConfig {
  mode: NetworkMode;
  mac: string | null;
  port_forwards: PortForward[];
}

/**
 * A virtio-9p shared folder (Phase 5). Mirrors `SharedFolderDto` in
 * `src-tauri/src/commands.rs` — snake_case wire keys.
 */
export interface SharedFolder {
  /** Absolute host directory; must exist (validated server-side). */
  host_path: string;
  /** 9p `mount_tag` used by the guest to mount this folder. */
  mount_tag: string;
  read_only: boolean;
}

export interface VmConfig {
  id: string;
  name: string;
  hardware: Hardware;
  disks: Disk[];
  network: NetworkConfig;
  iso: string | null;
  /** virtio-9p shared folders (Phase 5). Optional for serde back-compat. */
  shared_folders?: SharedFolder[];
  /**
   * Derived suspend flag (Phase 5): true when the VM has a captured live
   * snapshot. A suspended VM reports `state === "stopped"` on the wire.
   */
  suspended?: boolean;
}

export interface VmListItem {
  id: string;
  name: string;
  state: VmState;
  accelerator: Accelerator;
  emulated: boolean;
  cpus: number;
  memory_mib: number;
  iso: string | null;
  /**
   * Derived suspend flag (Phase 5). When true with `state === "stopped"` the
   * VM is suspended (resume or discard), not plainly stopped.
   */
  suspended?: boolean;
}

export interface CreateVmRequest {
  name: string;
  hardware: Hardware;
  disk_gib: number;
  network?: NetworkConfig | null;
  iso?: string | null;
}

export interface UpdateVmRequest {
  name: string;
  hardware: Hardware;
  network?: NetworkConfig | null;
  iso?: string | null;
  /** virtio-9p shared folders (Phase 5). Serde-optional for back-compat. */
  shared_folders?: SharedFolder[];
}

// ---- Phase 3 snapshot / clone types (mirror SnapshotDto, snake_case) ----

export interface Snapshot {
  snapshot_id: string;
  name: string;
  /** Parent snapshot id (`null` => top-level). */
  parent_id: string | null;
  created_at: string;
  /** True when RAM was captured (a live snapshot). */
  has_vm_state: boolean;
  vm_state_size: number;
  /** False when our metadata references a snapshot missing from the qcow2. */
  present_in_qcow2: boolean;
}

export type CloneKind = "full" | "linked";

// ---- Phase 4 networking capability types (mirror serde, snake_case) ----

export interface ModeCapability {
  mode: NetworkMode;
  available: boolean;
  requires_elevation: boolean;
  /** Empty when `available`; otherwise the per-OS needs-permission reason. */
  reason: string;
}

export interface NetworkCapabilities {
  modes: ModeCapability[];
  /** True when forwards bind loopback by default (per-forward LAN opt-in). */
  port_forward_loopback_only: boolean;
}

// ---- Phase 1 wrappers (kept) ----

export const probeHost = () => invoke<HostCapabilities>("probe_host");
/**
 * Persist the user's "Locate QEMU…" directory override (Phase 6 — D3). The
 * first-run gate calls this with the picked directory, then re-probes (no app
 * restart) so the resolver picks up the new location. An empty string clears
 * the override.
 */
export const setQemuDir = (dir: string) =>
  invoke<void>("set_qemu_dir", { dir });
export const openConsole = (id: string) => invoke<number>("open_console", { id });
export const vmState = (id: string) => invoke<VmState>("vm_state", { id });
export const powerOff = (id: string) => invoke<void>("power_off", { id });
export const forceOff = (id: string) => invoke<void>("force_off", { id });
export const pauseVm = (id: string) => invoke<void>("pause_vm", { id });
export const resumeVm = (id: string) => invoke<void>("resume_vm", { id });

// ---- Phase 5 suspend/restore wrappers (distinct from pause/resume = QMP cont) ----

/** Suspend a running VM: capture live snapshot then terminate the process. */
export const suspendVm = (id: string) => invoke<void>("suspend_vm", { id });
/** Restore a suspended VM: relaunch with `-S`, `snapshot-load`, then `cont`. */
export const restoreVm = (id: string) => invoke<void>("restore_vm", { id });
/**
 * Discard a VM's captured suspend state, returning it to plain stopped (the
 * escape hatch when a suspended VM should not be resumed).
 */
export const discardSuspend = (id: string) =>
  invoke<void>("discard_suspend", { id });

// ---- Phase 2 library/lifecycle wrappers (snake_case arg keys) ----

export const createVm = (req: CreateVmRequest) =>
  invoke<VmConfig>("create_vm", { req });
export const listVms = () => invoke<VmListItem[]>("list_vms");
export const getVm = (id: string) => invoke<VmConfig>("get_vm", { id });
export const updateVm = (id: string, req: UpdateVmRequest) =>
  invoke<VmConfig>("update_vm", { id, req });
export const deleteVm = (id: string, delete_disks: boolean) =>
  invoke<void>("delete_vm", { id, delete_disks });
export const startVm = (id: string) => invoke<void>("start_vm", { id });
export const renameVm = (id: string, new_name: string) =>
  invoke<VmConfig>("rename_vm", { id, new_name });

// ---- Phase 3 snapshot / clone wrappers (snake_case arg keys) ----

export const listSnapshots = (id: string) =>
  invoke<Snapshot[]>("list_snapshots", { id });
export const createSnapshot = (id: string, name: string) =>
  invoke<Snapshot>("create_snapshot", { id, name });
export const restoreSnapshot = (id: string, snapshot_id: string) =>
  invoke<void>("restore_snapshot", { id, snapshot_id });
export const deleteSnapshot = (id: string, snapshot_id: string) =>
  invoke<void>("delete_snapshot", { id, snapshot_id });
export const cloneVm = (id: string, new_name: string, linked: boolean) =>
  invoke<VmConfig>("clone_vm", { id, new_name, linked });

// ---- Phase 4 networking capability wrapper ----

export const networkCapabilities = () =>
  invoke<NetworkCapabilities>("network_capabilities");
