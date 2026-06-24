// Pure client-side validators. These mirror (but do not replace) the
// server-side checks in `vmforge-core::library::validate_vm_name`; the backend
// is authoritative. No React, no IPC — directly unit-testable.

/** Clamp bounds, kept in sync with the server-side clamps in `create_vm`. */
export const MIN_CPUS = 1;
export const MAX_CPUS = 64;
export const MIN_MEMORY_MIB = 256;
export const MAX_MEMORY_MIB = 1024 * 1024; // 1 TiB ceiling for the slider
export const MIN_DISK_GIB = 1;
export const MAX_DISK_GIB = 8192;
export const MAX_NAME_LEN = 100;

// Windows-reserved device names (case-insensitive), mirrors the server list.
const WINDOWS_RESERVED = new Set([
  "con",
  "prn",
  "aux",
  "nul",
  "com1",
  "com2",
  "com3",
  "com4",
  "com5",
  "com6",
  "com7",
  "com8",
  "com9",
  "lpt1",
  "lpt2",
  "lpt3",
  "lpt4",
  "lpt5",
  "lpt6",
  "lpt7",
  "lpt8",
  "lpt9",
]);

// Matches ASCII control characters (0x00-0x1F and DEL 0x7F).
const CONTROL_CHARS = new RegExp("[\\x00-\\x1f\\x7f]");

/**
 * Validate a VM name. Returns `null` when valid, otherwise a human message.
 * Rejects: empty/whitespace, path separators, `.`/`..`, control chars,
 * Windows-reserved names, trailing dot/space, over-long names.
 */
export function validateVmName(name: string): string | null {
  if (name == null) return "Name is required.";
  const trimmed = name.trim();
  if (trimmed.length === 0) return "Name is required.";
  if (name.length > MAX_NAME_LEN) {
    return `Name must be ${MAX_NAME_LEN} characters or fewer.`;
  }
  if (name === "." || name === "..") return "Name cannot be “.” or “..”.";
  if (/[\\/]/.test(name)) return "Name cannot contain “/” or “\\”.";
  if (CONTROL_CHARS.test(name)) return "Name cannot contain control characters.";
  if (name.endsWith(".")) return "Name cannot end with a dot.";
  if (name.endsWith(" ")) return "Name cannot end with a space.";
  if (WINDOWS_RESERVED.has(name.trim().toLowerCase())) {
    return "Name is reserved by the operating system.";
  }
  return null;
}

/** True iff the name passes validation. */
export function isValidVmName(name: string): boolean {
  return validateVmName(name) === null;
}

/** Validate vCPU count. Returns null when valid. */
export function validateCpus(cpus: number): string | null {
  if (!Number.isInteger(cpus)) return "vCPUs must be a whole number.";
  if (cpus < MIN_CPUS) return `At least ${MIN_CPUS} vCPU is required.`;
  if (cpus > MAX_CPUS) return `At most ${MAX_CPUS} vCPUs are allowed.`;
  return null;
}

/** Validate RAM in MiB. Returns null when valid. */
export function validateMemoryMib(memoryMib: number): string | null {
  if (!Number.isFinite(memoryMib)) return "Memory must be a number.";
  if (memoryMib < MIN_MEMORY_MIB) {
    return `At least ${MIN_MEMORY_MIB} MiB is required.`;
  }
  if (memoryMib > MAX_MEMORY_MIB) return "Memory exceeds the maximum.";
  return null;
}

/** Validate disk size in GiB. Returns null when valid. */
export function validateDiskGib(diskGib: number): string | null {
  if (!Number.isFinite(diskGib)) return "Disk size must be a number.";
  if (diskGib < MIN_DISK_GIB) return `At least ${MIN_DISK_GIB} GiB is required.`;
  if (diskGib > MAX_DISK_GIB) return "Disk size exceeds the maximum.";
  return null;
}

/** Clamp a value into an inclusive range. */
export function clamp(value: number, min: number, max: number): number {
  if (Number.isNaN(value)) return min;
  return Math.min(max, Math.max(min, value));
}
