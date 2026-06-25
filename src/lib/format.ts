// Pure display formatters. No React, no IPC — safe to unit-test directly.
import type { Accelerator, VmListItem, VmState } from "@/lib/ipc";

/** Format a MiB amount as a human GiB string (e.g. 2048 -> "2 GiB"). */
export function formatMemory(memoryMib: number): string {
  if (!Number.isFinite(memoryMib) || memoryMib <= 0) return "0 MiB";
  if (memoryMib < 1024) return `${memoryMib} MiB`;
  const gib = memoryMib / 1024;
  const rounded = Math.round(gib * 100) / 100;
  return `${rounded} GiB`;
}

/** Convert MiB to GiB as a number (no unit). */
export function mibToGib(memoryMib: number): number {
  return Math.round((memoryMib / 1024) * 100) / 100;
}

/** Convert GiB to MiB as a whole number. */
export function gibToMib(gib: number): number {
  return Math.round(gib * 1024);
}

/** Human label for a lifecycle state. */
export function stateLabel(state: VmState): string {
  switch (state) {
    case "defined":
      return "Defined";
    case "starting":
      return "Starting";
    case "running":
      return "Running";
    case "paused":
      return "Paused";
    case "stopping":
      return "Stopping";
    case "stopped":
      return "Stopped";
    case "error":
      return "Error";
    default:
      return state;
  }
}

export type StateTone = "running" | "paused" | "idle" | "transitioning" | "error";

/** Semantic tone for a lifecycle state (drives badge colors). */
export function stateTone(state: VmState): StateTone {
  switch (state) {
    case "running":
      return "running";
    case "paused":
      return "paused";
    case "defined":
    case "stopped":
      return "idle";
    case "starting":
    case "stopping":
      return "transitioning";
    case "error":
      return "error";
    default:
      return "idle";
  }
}

/** Whether a state represents a live (process-backed) VM. */
export function isLive(state: VmState): boolean {
  return (
    state === "starting" ||
    state === "running" ||
    state === "paused" ||
    state === "stopping"
  );
}

/** Whether a state is mid-transition (actions should show a spinner). */
export function isTransitioning(state: VmState): boolean {
  return state === "starting" || state === "stopping";
}

/**
 * Whether a VM is suspended (Phase 5): it reports as `stopped` on the wire but
 * carries a captured live snapshot. Suspended VMs offer Resume / Discard rather
 * than a plain Start. Does not change the lifecycle state or its tone.
 */
export function isSuspended(vm: Pick<VmListItem, "state" | "suspended">): boolean {
  return vm.state === "stopped" && vm.suspended === true;
}

/** Display label for an accelerator. */
export function accelLabel(accel: Accelerator): string {
  switch (accel) {
    case "hvf":
      return "HVF";
    case "whpx":
      return "WHPX";
    case "kvm":
      return "KVM";
    case "tcg":
      return "TCG";
    default:
      return String(accel).toUpperCase();
  }
}

/** True for hardware-accelerated backends (not TCG). */
export function isHardwareAccel(accel: Accelerator): boolean {
  return accel !== "tcg";
}

/** Format a vCPU count (e.g. "2 vCPU", "4 vCPUs"). */
export function formatCpus(cpus: number): string {
  return cpus === 1 ? "1 vCPU" : `${cpus} vCPUs`;
}

/** Pull a friendly filename off a full path. */
export function basename(path: string): string {
  if (!path) return "";
  const parts = path.split(/[\\/]/);
  return parts[parts.length - 1] || path;
}

/**
 * Human label for a snapshot's RFC3339 `created_at` timestamp. Falls back to
 * the raw string when it isn't a parseable date.
 */
export function snapshotDateLabel(createdAt: string): string {
  if (!createdAt) return "";
  const ms = Date.parse(createdAt);
  if (Number.isNaN(ms)) return createdAt;
  return new Date(ms).toLocaleString(undefined, {
    year: "numeric",
    month: "short",
    day: "numeric",
    hour: "2-digit",
    minute: "2-digit",
  });
}

/**
 * Format a byte count as a human binary-unit string (e.g. 1536 -> "1.5 KiB").
 * Used for snapshot RAM (`vm_state_size`). Returns "0 B" for zero/invalid.
 */
export function formatBytes(bytes: number): string {
  if (!Number.isFinite(bytes) || bytes <= 0) return "0 B";
  const units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
  let value = bytes;
  let unit = 0;
  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024;
    unit += 1;
  }
  // No decimals for raw bytes; up to two for larger units.
  const rounded = unit === 0 ? value : Math.round(value * 100) / 100;
  return `${rounded} ${units[unit]}`;
}
