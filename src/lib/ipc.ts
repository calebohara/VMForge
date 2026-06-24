// Typed wrappers over the Tauri IPC command surface. These mirror the Rust
// structs in `vmforge-core` / `src-tauri/src/commands.rs` — keep them in sync.
import { invoke } from "@tauri-apps/api/core";

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
  warnings: string[];
}

export type VmState =
  | "defined"
  | "starting"
  | "running"
  | "paused"
  | "stopping"
  | "stopped"
  | "error";

export interface VmDescriptor {
  id: string;
  name: string;
  vnc_port: number;
}

export interface NewVmRequest {
  name: string;
  cpus: number;
  memory_mib: number;
  disk_gib: number;
  iso: string | null;
}

export const probeHost = () => invoke<HostCapabilities>("probe_host");
export const createAndStartVm = (req: NewVmRequest) =>
  invoke<VmDescriptor>("create_and_start_vm", { req });
export const openConsole = (id: string) => invoke<number>("open_console", { id });
export const vmState = (id: string) => invoke<VmState>("vm_state", { id });
export const powerOff = (id: string) => invoke<void>("power_off", { id });
export const forceOff = (id: string) => invoke<void>("force_off", { id });
export const pauseVm = (id: string) => invoke<void>("pause_vm", { id });
export const resumeVm = (id: string) => invoke<void>("resume_vm", { id });
