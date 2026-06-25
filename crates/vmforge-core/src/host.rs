//! Host capability probing.
//!
//! Detects OS/arch, locates the QEMU binaries, and determines which
//! accelerator this host can actually use (HVF / WHPX / KVM, with a TCG
//! software-emulation fallback). Backs VMForge's first-run capability
//! screen. The pure parsers at the bottom are unit-tested, so the suite
//! needs no real QEMU installed.

use crate::error::Result;
use crate::model::NetworkMode;
use crate::qemu_resolve::resolve_qemu_binary;
use serde::{Deserialize, Serialize};
use std::process::Command;

/// Hardware/software accelerators VMForge understands. Pick per-host at
/// runtime — never hardcode one.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Accelerator {
    /// macOS Hypervisor.framework.
    Hvf,
    /// Windows Hypervisor Platform.
    Whpx,
    /// Linux KVM.
    Kvm,
    /// Pure software emulation — always available, always slow.
    Tcg,
}

impl Accelerator {
    /// The string QEMU expects for `-accel <x>`.
    pub fn as_qemu_arg(self) -> &'static str {
        match self {
            Self::Hvf => "hvf",
            Self::Whpx => "whpx",
            Self::Kvm => "kvm",
            Self::Tcg => "tcg",
        }
    }

    /// True for hardware accelerators (everything but TCG).
    pub fn is_hardware(self) -> bool {
        !matches!(self, Self::Tcg)
    }

    fn from_name(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "hvf" => Some(Self::Hvf),
            "whpx" => Some(Self::Whpx),
            "kvm" => Some(Self::Kvm),
            "tcg" => Some(Self::Tcg),
            _ => None,
        }
    }
}

/// A QEMU binary we looked for, with its version if present.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QemuBinary {
    pub name: String,
    pub present: bool,
    pub version: Option<String>,
}

/// Whether a single network mode can be used on this host, and if not, why.
/// `reason` is empty when `available == true`; otherwise it is the user-facing
/// explanation shared with the launch-reject path (so the UI and the engine
/// never disagree).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ModeCapability {
    pub mode: NetworkMode,
    pub available: bool,
    pub requires_elevation: bool,
    /// Empty when `available`; otherwise the per-OS needs-permission reason.
    pub reason: String,
}

/// The aggregate networking capability picture for this host. `modes` lists
/// every mode VMForge knows about; `port_forward_loopback_only` documents that
/// forwards bind loopback by default (per-forward LAN exposure is opt-in).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NetworkCapabilities {
    pub modes: Vec<ModeCapability>,
    pub port_forward_loopback_only: bool,
}

/// Everything the UI needs to explain what this host can and can't do.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HostCapabilities {
    /// `"macos"`, `"windows"`, or `"linux"`.
    pub os: String,
    /// `"aarch64"` or `"x86_64"`.
    pub arch: String,
    /// The accelerator VMForge will use by default on this host.
    pub preferred_accelerator: Accelerator,
    /// Every accelerator the native QEMU binary advertises.
    pub available_accelerators: Vec<Accelerator>,
    /// Whether the preferred accelerator is hardware (vs TCG fallback).
    pub hardware_accelerated: bool,
    pub qemu_img: QemuBinary,
    /// `qemu-system-*` binaries we probed (aarch64 + x86_64).
    pub system_binaries: Vec<QemuBinary>,
    /// Per-mode networking capabilities (user available; bridged/host-only
    /// gated behind elevated permissions in this build).
    pub network: NetworkCapabilities,
    /// Honest, user-facing limitations to surface in the UI.
    pub warnings: Vec<String>,
}

const SYSTEM_CANDIDATES: &[&str] = &["qemu-system-aarch64", "qemu-system-x86_64"];

/// Probe the current host. Never fails today, but returns [`Result`] so
/// future privileged checks can surface errors.
pub fn probe() -> Result<HostCapabilities> {
    let os = std::env::consts::OS.to_string();
    let arch = std::env::consts::ARCH.to_string();
    let mut warnings = Vec::new();

    let system_binaries: Vec<QemuBinary> = SYSTEM_CANDIDATES
        .iter()
        .map(|name| {
            let version = binary_version(name);
            QemuBinary {
                name: (*name).to_string(),
                present: version.is_some(),
                version,
            }
        })
        .collect();

    let qemu_img = {
        let version = binary_version("qemu-img");
        QemuBinary {
            name: "qemu-img".to_string(),
            present: version.is_some(),
            version,
        }
    };

    // Probe accelerators against the binary that matches the host arch.
    let native = native_system_binary(&arch);
    let available_accelerators = if system_binaries
        .iter()
        .any(|b| b.name == native && b.present)
    {
        query_accelerators(&native)
    } else {
        Vec::new()
    };

    let preferred = pick_preferred(&os, &available_accelerators);
    let hardware_accelerated = preferred.is_hardware();

    if !qemu_img.present {
        warnings
            .push("qemu-img not found — disk operations unavailable. Install QEMU.".to_string());
    }
    if system_binaries.iter().all(|b| !b.present) {
        warnings
            .push("No qemu-system-* binary found — cannot launch VMs. Install QEMU.".to_string());
    } else if !hardware_accelerated {
        warnings.push(format!(
            "No hardware accelerator available on {os}; falling back to TCG software emulation (expect reduced performance)."
        ));
    }
    if os == "macos" && arch == "aarch64" {
        warnings.push(
            "Apple Silicon: ARM64 guests are HVF-accelerated; x86/x64 guests run under TCG emulation and will be slow.".to_string(),
        );
    }
    // On Windows, WHPX being unavailable almost always means another component
    // owns the hypervisor (Hyper-V / WSL2 / memory-integrity VBS). Name the
    // usual cause instead of the generic TCG-fallback line (CLAUDE.md mandate).
    if os == "windows" && !hardware_accelerated && system_binaries.iter().any(|b| b.present) {
        warnings.push(
            "Windows Hypervisor Platform (WHPX) is unavailable — usually because Hyper-V, WSL2, or memory-integrity (VBS) is holding the virtualization stack. Enable the Windows Hypervisor Platform feature for hardware acceleration; until then VMForge runs guests under TCG (slow).".to_string(),
        );
    }

    let network = probe_network(&os);

    Ok(HostCapabilities {
        os,
        arch,
        preferred_accelerator: preferred,
        available_accelerators,
        hardware_accelerated,
        qemu_img,
        system_binaries,
        network,
        warnings,
    })
}

/// Probe networking capabilities for a host OS. User-mode NAT is always
/// available with zero privileges; bridged and host-only are gated behind
/// elevated permissions in this build (decision A2), so they are reported
/// `available == false`, `requires_elevation == true`, with the shared per-OS
/// reason (so the capability UI and the launch-reject path never drift). Port
/// forwards bind loopback by default (`port_forward_loopback_only == true`).
pub fn probe_network(os: &str) -> NetworkCapabilities {
    let user = ModeCapability {
        mode: NetworkMode::User,
        available: true,
        requires_elevation: false,
        reason: String::new(),
    };
    let elevated = |mode: NetworkMode| ModeCapability {
        mode,
        available: false,
        requires_elevation: true,
        reason: crate::qemu::net::elevated_reason(mode, os),
    };
    NetworkCapabilities {
        modes: vec![
            user,
            elevated(NetworkMode::Bridged),
            elevated(NetworkMode::HostOnly),
        ],
        port_forward_loopback_only: true,
    }
}

/// QEMU system-emulator binary name for a guest/host arch.
pub fn system_binary(arch: &str) -> &'static str {
    match arch {
        "aarch64" => "qemu-system-aarch64",
        "x86_64" => "qemu-system-x86_64",
        _ => "qemu-system-x86_64",
    }
}

fn native_system_binary(arch: &str) -> String {
    system_binary(arch).to_string()
}

/// Probe a QEMU binary's version, resolving it to an absolute path first (D3).
/// Resolving here — not invoking the bare name — is what makes the probe agree
/// with the spawn path under a Finder-launched `.app` (empty inherited `PATH`).
fn binary_version(bin: &str) -> Option<String> {
    let resolved = resolve_qemu_binary(bin)?;
    let mut cmd = Command::new(&resolved);
    cmd.arg("--version");
    let out =
        crate::qemu_resolve::output_with_timeout(&mut cmd, crate::qemu_resolve::PROBE_TIMEOUT)?;
    if !out.status.success() {
        return None;
    }
    parse_version(&String::from_utf8_lossy(&out.stdout))
}

fn query_accelerators(bin: &str) -> Vec<Accelerator> {
    let Some(resolved) = resolve_qemu_binary(bin) else {
        return Vec::new();
    };
    let mut cmd = Command::new(&resolved);
    cmd.args(["-accel", "help"]);
    match crate::qemu_resolve::output_with_timeout(&mut cmd, crate::qemu_resolve::PROBE_TIMEOUT) {
        Some(out) => parse_accelerators(&String::from_utf8_lossy(&out.stdout)),
        None => Vec::new(),
    }
}

/// Choose the host's natural hardware accelerator if QEMU advertises it,
/// otherwise fall back to TCG.
fn pick_preferred(os: &str, available: &[Accelerator]) -> Accelerator {
    let want = match os {
        "macos" => Accelerator::Hvf,
        "windows" => Accelerator::Whpx,
        "linux" => Accelerator::Kvm,
        _ => Accelerator::Tcg,
    };
    if available.contains(&want) {
        want
    } else {
        Accelerator::Tcg
    }
}

// ---- pure, unit-tested parsers ----

/// Extract a version like `11.0.1` from a `--version` first line such as
/// `"QEMU emulator version 11.0.1"` or `"qemu-img version 11.0.1 (...)"`.
fn parse_version(s: &str) -> Option<String> {
    let line = s.lines().next()?;
    let after = line.split("version").nth(1)?;
    after.split_whitespace().next().map(str::to_string)
}

/// Parse the body of `qemu-system-* -accel help`.
fn parse_accelerators(s: &str) -> Vec<Accelerator> {
    s.lines().filter_map(Accelerator::from_name).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_emulator_version() {
        assert_eq!(
            parse_version("QEMU emulator version 11.0.1\nCopyright (c) ..."),
            Some("11.0.1".to_string())
        );
    }

    #[test]
    fn parses_qemu_img_version() {
        assert_eq!(
            parse_version("qemu-img version 11.0.1 (Homebrew)"),
            Some("11.0.1".to_string())
        );
    }

    #[test]
    fn parses_accel_help() {
        let out = "Accelerators supported in QEMU binary:\nhvf\ntcg\n";
        assert_eq!(
            parse_accelerators(out),
            vec![Accelerator::Hvf, Accelerator::Tcg]
        );
    }

    #[test]
    fn prefers_hardware_when_available() {
        assert_eq!(
            pick_preferred("macos", &[Accelerator::Hvf, Accelerator::Tcg]),
            Accelerator::Hvf
        );
        assert_eq!(
            pick_preferred("linux", &[Accelerator::Kvm, Accelerator::Tcg]),
            Accelerator::Kvm
        );
    }

    #[test]
    fn falls_back_to_tcg_without_hardware() {
        assert_eq!(
            pick_preferred("macos", &[Accelerator::Tcg]),
            Accelerator::Tcg
        );
        assert_eq!(pick_preferred("windows", &[]), Accelerator::Tcg);
    }

    // ---- (E.2) network capability shape ----
    #[test]
    fn network_caps_phase4_shape() {
        let caps = probe_network("macos");
        assert!(
            caps.port_forward_loopback_only,
            "forwards must bind loopback by default"
        );
        assert_eq!(caps.modes.len(), 3, "user + bridged + host-only");

        let user = caps
            .modes
            .iter()
            .find(|m| m.mode == NetworkMode::User)
            .expect("user mode present");
        assert!(user.available, "user mode is available");
        assert!(!user.requires_elevation);
        assert!(user.reason.is_empty(), "available mode has empty reason");

        for mode in [NetworkMode::Bridged, NetworkMode::HostOnly] {
            let m = caps
                .modes
                .iter()
                .find(|m| m.mode == mode)
                .unwrap_or_else(|| panic!("{mode:?} present"));
            assert!(!m.available, "{mode:?} unavailable in this build");
            assert!(m.requires_elevation, "{mode:?} requires elevation");
            assert!(!m.reason.is_empty(), "{mode:?} reason non-empty");
            assert!(
                !m.reason.to_lowercase().contains("unsupported"),
                "{mode:?} reason must not say 'unsupported': {}",
                m.reason
            );
        }

        // On macOS the bridged reason must mention vmnet (matches net.rs).
        let bridged = caps
            .modes
            .iter()
            .find(|m| m.mode == NetworkMode::Bridged)
            .unwrap();
        assert!(
            bridged.reason.contains("vmnet"),
            "macos bridged reason must mention vmnet: {}",
            bridged.reason
        );
    }
}
