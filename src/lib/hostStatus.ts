// Pure host-status predicates and copy for the Phase 6 first-run UX (spec §C).
//
// These derive everything the gate/banner need from a `HostCapabilities` probe
// result — no IPC, no React. Kept pure so they're trivially unit-testable and
// the components stay declarative. The naming/branching mirrors the Rust probe
// in `crates/vmforge-core/src/host.rs` (system binaries are `qemu-system-*`,
// disk ops need `qemu-img`).

import type { HostCapabilities, QemuBinary } from "@/lib/ipc";

/**
 * True when QEMU is missing badly enough that we cannot run VMs at all: either
 * no `qemu-system-*` binary is present, or `qemu-img` is absent (no disk ops).
 * This is the hard first-run gate condition. Treats a `null`/un-probed caps as
 * "not missing" so the loading screen — not the gate — shows during the probe.
 */
export function qemuMissing(caps: HostCapabilities | null): boolean {
  if (!caps) return false;
  const noSystemBinary = !caps.system_binaries.some((b) => b.present);
  return noSystemBinary || !caps.qemu_img.present;
}

/**
 * True when the host is usable but degraded — the engine produced warnings, or
 * no hardware accelerator is available (TCG software emulation). Drives the soft
 * banner in the library. `null` caps => no warnings (nothing probed yet).
 */
export function hasHostWarnings(caps: HostCapabilities | null): boolean {
  if (!caps) return false;
  return caps.warnings.length > 0 || !caps.hardware_accelerated;
}

/**
 * The `qemu-system-*` binary that matches a host architecture. Mirrors
 * `native_system_binary` in `host.rs` (aarch64 → aarch64; everything else →
 * x86_64, the safe default for x86/x64 hosts).
 */
export function nativeSystemBinaryName(arch: string): string {
  switch (arch) {
    case "aarch64":
    case "arm64":
      return "qemu-system-aarch64";
    case "x86_64":
    case "x64":
    case "amd64":
      return "qemu-system-x86_64";
    default:
      return "qemu-system-x86_64";
  }
}

/**
 * The QEMU binaries we require, in display order: `qemu-img` plus the host's
 * native `qemu-system-*`. Each item carries its probed presence/version so the
 * gate can name exactly what is missing and show what was found. Always returns
 * the native system binary's record even when the probe didn't list it (a
 * synthesized absent entry), so the gate never renders a blank row.
 */
export function requiredBinaries(caps: HostCapabilities): QemuBinary[] {
  const nativeName = nativeSystemBinaryName(caps.arch);
  const nativeSystem =
    caps.system_binaries.find((b) => b.name === nativeName) ??
    ({ name: nativeName, present: false, version: null } satisfies QemuBinary);
  return [caps.qemu_img, nativeSystem];
}

/** A single install command/step with optional surrounding prose. */
export interface InstallStep {
  /** A shell command to copy/run, or null for a prose-only step. */
  command: string | null;
  /** Human-readable description shown alongside the command. */
  description: string;
}

/** A named platform install guide (heading + ordered steps + a docs URL). */
export interface InstallGuide {
  /** The OS this guide targets (the `os` key the caps probe returned, or a
   * label for the per-distro variants). */
  os: string;
  /** Display heading, e.g. "macOS (Homebrew)". */
  heading: string;
  steps: InstallStep[];
  /** Canonical QEMU download/docs URL for this platform. */
  docsUrl: string;
}

const QEMU_DOWNLOAD_URL = "https://www.qemu.org/download/";

/**
 * Honest, copy-pasteable install instructions for a host OS. `os` is the value
 * the probe returned (`"macos"`, `"linux"`, `"windows"`); Linux returns a guide
 * covering the common distro package managers since the probe can't tell them
 * apart. Unknown OSes fall back to the generic download page.
 */
export function installGuide(os: string): InstallGuide {
  switch (os) {
    case "macos":
      return {
        os,
        heading: "macOS (Homebrew)",
        steps: [
          {
            command: "brew install qemu",
            description:
              "Install QEMU with Homebrew, then re-check below (no restart needed).",
          },
        ],
        docsUrl: `${QEMU_DOWNLOAD_URL}#macos`,
      };
    case "linux":
      return {
        os,
        heading: "Linux (your distribution's package manager)",
        steps: [
          {
            command:
              "sudo apt install qemu-system-x86 qemu-system-arm qemu-utils",
            description: "Debian / Ubuntu",
          },
          {
            command: "sudo dnf install qemu-system-x86 qemu-system-aarch64 qemu-img",
            description: "Fedora / RHEL",
          },
          {
            command: "sudo pacman -S qemu-full",
            description: "Arch / Manjaro",
          },
        ],
        docsUrl: `${QEMU_DOWNLOAD_URL}#linux`,
      };
    case "windows":
      return {
        os,
        heading: "Windows",
        steps: [
          {
            command: null,
            description:
              "Download and run the QEMU installer for Windows from the official site.",
          },
          {
            command: null,
            description:
              "Add the QEMU install directory (default C:\\Program Files\\qemu) to your PATH, or use “Locate QEMU…” below to point VMForge at it directly.",
          },
        ],
        docsUrl: `${QEMU_DOWNLOAD_URL}#windows`,
      };
    default:
      return {
        os,
        heading: "Install QEMU",
        steps: [
          {
            command: null,
            description:
              "Install QEMU 11.x for your platform from the official download page, then re-check below.",
          },
        ],
        docsUrl: QEMU_DOWNLOAD_URL,
      };
  }
}
