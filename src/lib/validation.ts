// Pure client-side validators. These mirror (but do not replace) the
// server-side checks in `vmforge-core::library::validate_vm_name` and
// `vmforge-core::qemu::net`; the backend is authoritative. No React, no IPC —
// directly unit-testable.
import type { NetworkConfig, PortForward } from "@/lib/ipc";

/** Clamp bounds, kept in sync with the server-side clamps in `create_vm`. */
export const MIN_CPUS = 1;
export const MAX_CPUS = 64;
export const MIN_MEMORY_MIB = 256;
export const MAX_MEMORY_MIB = 1024 * 1024; // 1 TiB ceiling for the slider
export const MIN_DISK_GIB = 1;
export const MAX_DISK_GIB = 8192;
export const MAX_NAME_LEN = 100;

// ---- Phase 4 networking constants ----

/** Inclusive port range; 0 is reserved and rejected (mirrors the server). */
export const MIN_PORT = 1;
export const MAX_PORT = 65535;
/** Ports below this are privileged on most hosts — a soft (non-blocking) warning. */
export const PRIVILEGED_PORT_CEILING = 1024;
/** Upper bound on the number of port forwards a single VM may define. */
export const MAX_PORT_FORWARDS = 32;

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

// ---- Phase 4 networking validators (mirror `vmforge-core::qemu::net`) ----

/**
 * Validate a single TCP/UDP port number. Returns `null` when valid, otherwise a
 * human message. Must be a whole number in `1..=65535` (0 is rejected).
 */
export function validatePortNumber(port: number): string | null {
  if (!Number.isInteger(port)) return "Port must be a whole number.";
  if (port < MIN_PORT || port > MAX_PORT) {
    return `Port must be between ${MIN_PORT} and ${MAX_PORT}.`;
  }
  return null;
}

/**
 * Validate a single port forward in isolation (host + guest range only).
 * Duplicate detection across rows is handled by {@link validatePortForwards}.
 * Returns `null` when valid, otherwise a human message.
 */
export function validatePortForward(pf: PortForward): string | null {
  const hostErr = validatePortNumber(pf.host);
  if (hostErr) return `Host ${hostErr.charAt(0).toLowerCase()}${hostErr.slice(1)}`;
  const guestErr = validatePortNumber(pf.guest);
  if (guestErr) {
    return `Guest ${guestErr.charAt(0).toLowerCase()}${guestErr.slice(1)}`;
  }
  return null;
}

/**
 * Validate the full set of port forwards. Returns a per-row array of error
 * messages (`null` where the row is valid). Flags out-of-range host/guest
 * ports and, on the 2nd+ occurrence, a duplicate `(host, protocol)` pair —
 * TCP and UDP may share a host port; guest-port duplicates are allowed.
 * Mirrors `validate_port_forwards` in the engine.
 */
export function validatePortForwards(pfs: PortForward[]): (string | null)[] {
  const seen = new Set<string>();
  return pfs.map((pf) => {
    const rangeErr = validatePortForward(pf);
    if (rangeErr) return rangeErr;
    const proto = pf.udp ? "udp" : "tcp";
    const key = `${proto}:${pf.host}`;
    if (seen.has(key)) {
      return `Duplicate ${proto.toUpperCase()} host port ${pf.host}.`;
    }
    seen.add(key);
    return null;
  });
}

/**
 * Soft, non-blocking warnings for a set of port forwards. Currently flags host
 * ports below {@link PRIVILEGED_PORT_CEILING}, which usually require elevated
 * privileges to bind. Returns a per-row array (`null` where there is no
 * warning). These never gate validity.
 */
export function portForwardWarnings(pfs: PortForward[]): (string | null)[] {
  return pfs.map((pf) => {
    if (
      Number.isInteger(pf.host) &&
      pf.host >= MIN_PORT &&
      pf.host < PRIVILEGED_PORT_CEILING
    ) {
      return `Host ports below ${PRIVILEGED_PORT_CEILING} usually need elevated privileges to bind.`;
    }
    return null;
  });
}

// MAC: exactly six colon-separated hex octets (e.g. 52:54:00:12:34:56).
const MAC_RE = /^([0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}$/;

/**
 * Validate a MAC address. Returns `null` when valid, otherwise a human message.
 * Requires the strict 6-octet shape and rejects multicast addresses (the low
 * bit of the first octet must be clear). Mirrors `validate_mac` in the engine.
 */
export function validateMac(mac: string): string | null {
  if (!MAC_RE.test(mac)) {
    return "MAC must be six colon-separated hex octets (e.g. 52:54:00:12:34:56).";
  }
  const firstOctet = parseInt(mac.slice(0, 2), 16);
  if ((firstOctet & 0x01) !== 0) {
    return "MAC cannot be a multicast address (the first octet must be even).";
  }
  return null;
}

/**
 * Generate a locally-administered unicast MAC with the QEMU `52:54:00` OUI and
 * three CSPRNG-random low octets, lowercase. The fixed prefix is already
 * unicast, so the result always passes {@link validateMac}.
 */
export function generateMac(): string {
  const rand = new Uint8Array(3);
  crypto.getRandomValues(rand);
  const tail = Array.from(rand, (b) => b.toString(16).padStart(2, "0")).join(
    ":",
  );
  return `52:54:00:${tail}`;
}

/**
 * Normalize an edited network config for submission: trim the MAC (blank →
 * null so QEMU auto-assigns) and drop the form-only NaN placeholders an empty
 * port input leaves behind. Pure — shared by the wizard and the hardware editor.
 */
export function normalizeNetwork(net: NetworkConfig): NetworkConfig {
  const mac = net.mac && net.mac.trim() !== "" ? net.mac.trim() : null;
  const port_forwards = net.port_forwards
    .filter((pf) => Number.isInteger(pf.host) && Number.isInteger(pf.guest))
    .map((pf) => ({ ...pf }));
  return { mode: net.mode, mac, port_forwards };
}
