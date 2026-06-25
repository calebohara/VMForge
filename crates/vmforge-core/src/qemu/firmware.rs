//! UEFI / BIOS firmware discovery.
//!
//! - **aarch64 `virt`** has no built-in BIOS → it needs the edk2 UEFI code blob
//!   (`edk2-aarch64-code.fd`), passed via `-bios`.
//! - **x86_64 q35** has a built-in SeaBIOS (legacy BIOS) and needs nothing to
//!   boot a legacy ISO. But UEFI guests — notably **Windows 11**, which
//!   requires UEFI — need OVMF: a read-only CODE blob plus a writable VARS
//!   template, wired as two `-drive if=pflash` units. When OVMF is absent we
//!   fall back to SeaBIOS (the engine logs that Windows 11 won't boot then).
//!
//! `qemu_bin` is the ALREADY-RESOLVED absolute QEMU path (D3); we derive
//! `<prefix>/share/...` from it (no second resolve / `--version` spawn) and then
//! fall back to well-known install prefixes for Homebrew, system, MSYS2/Windows,
//! and Linux distro OVMF packages.

use std::path::{Path, PathBuf};

const AARCH64_CODE: &str = "edk2-aarch64-code.fd";

/// x86_64 OVMF (CODE, VARS) filename pairs, most-preferred first. Searched as
/// pairs — never mix a 4M CODE with a non-4M VARS. QEMU's own builds ship the
/// `edk2-*` names; distro OVMF packages use the `OVMF_*` names.
const X86_PAIRS: &[(&str, &str)] = &[
    ("edk2-x86_64-code.fd", "edk2-i386-vars.fd"),
    ("OVMF_CODE_4M.fd", "OVMF_VARS_4M.fd"),
    ("OVMF_CODE.fd", "OVMF_VARS.fd"),
];

/// x86_64 OVMF firmware: a read-only CODE blob + a writable VARS template. The
/// engine copies `vars_template` to a per-VM writable `OVMF_VARS.fd` before
/// launch (NVRAM must be writable and per-VM).
pub struct X86Firmware {
    pub code: PathBuf,
    pub vars_template: PathBuf,
}

/// Directories that may hold QEMU firmware: the resolved binary's sibling
/// `share` tree first, then well-known install prefixes across platforms.
fn firmware_dirs(qemu_bin: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    // 1. `share` trees relative to the resolved binary. Search BOTH:
    //    - the exe's own parent: the qemu.org Windows installer puts
    //      `qemu-system-*.exe` in the install ROOT with firmware under
    //      `<root>\share\` (and `<root>\share\edk2\`).
    //    - the grandparent: Homebrew / MSYS2 / Linux put the exe in
    //      `<prefix>/bin` with firmware under `<prefix>/share/qemu` etc.
    //    Include the bare `share` dir (Windows) plus the qemu/edk2/OVMF subdirs.
    let bin_dir = qemu_bin.parent();
    let prefix = bin_dir.and_then(Path::parent);
    for base in [bin_dir, prefix].into_iter().flatten() {
        let share = base.join("share");
        dirs.push(share.clone());
        dirs.push(share.join("qemu"));
        dirs.push(share.join("edk2"));
        dirs.push(share.join("OVMF"));
    }

    // 2. Common absolute locations (Homebrew / system / Linux OVMF packages).
    for c in [
        "/opt/homebrew/share/qemu",
        "/opt/homebrew/opt/qemu/share/qemu",
        "/usr/local/share/qemu",
        "/usr/share/qemu",
        "/usr/share/edk2/aarch64",
        "/usr/share/edk2/x64",
        "/usr/share/edk2-ovmf",
        "/usr/share/OVMF",
        "/usr/share/ovmf",
    ] {
        dirs.push(PathBuf::from(c));
    }
    dirs
}

/// Locate the aarch64 UEFI code blob (`-bios`). `None` if not found.
pub fn find_aarch64_uefi(qemu_bin: &Path) -> Option<PathBuf> {
    for dir in firmware_dirs(qemu_bin) {
        let f = dir.join(AARCH64_CODE);
        if f.is_file() {
            return Some(f);
        }
    }
    None
}

/// Locate x86_64 OVMF firmware (CODE + VARS, from the same dir, paired). Returns
/// `None` when no pair is found — the caller then falls back to SeaBIOS.
pub fn find_x86_64_uefi(qemu_bin: &Path) -> Option<X86Firmware> {
    for dir in firmware_dirs(qemu_bin) {
        for (code, vars) in X86_PAIRS {
            let c = dir.join(code);
            let v = dir.join(vars);
            if c.is_file() && v.is_file() {
                return Some(X86Firmware {
                    code: c,
                    vars_template: v,
                });
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    /// `<tmp>/bin/qemu-system-x86_64` with firmware under `<tmp>/share/qemu`
    /// exercises the `<bin_dir>/../share/qemu` derivation cross-platform.
    fn fake_install(files: &[&str]) -> (PathBuf, PathBuf) {
        let tmp = std::env::temp_dir().join(format!("vmforge-fw-{}", uuid::Uuid::new_v4()));
        let bin = tmp.join("bin").join("qemu-system-x86_64");
        let share = tmp.join("share").join("qemu");
        fs::create_dir_all(bin.parent().unwrap()).unwrap();
        fs::create_dir_all(&share).unwrap();
        fs::write(&bin, b"x").unwrap();
        for f in files {
            fs::write(share.join(f), b"fw").unwrap();
        }
        (tmp, bin)
    }

    #[test]
    fn finds_x86_ovmf_pair_qemu_names() {
        let (tmp, bin) = fake_install(&["edk2-x86_64-code.fd", "edk2-i386-vars.fd"]);
        let fw = find_x86_64_uefi(&bin).expect("OVMF pair found");
        assert!(fw.code.ends_with("edk2-x86_64-code.fd"));
        assert!(fw.vars_template.ends_with("edk2-i386-vars.fd"));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn finds_x86_ovmf_pair_distro_4m_names() {
        let (tmp, bin) = fake_install(&["OVMF_CODE_4M.fd", "OVMF_VARS_4M.fd"]);
        let fw = find_x86_64_uefi(&bin).expect("OVMF 4M pair found");
        assert!(fw.code.ends_with("OVMF_CODE_4M.fd"));
        assert!(fw.vars_template.ends_with("OVMF_VARS_4M.fd"));
        let _ = fs::remove_dir_all(&tmp);
    }

    // Note: a "no OVMF → None" test would be host-fragile — a dev host with
    // QEMU installed has real OVMF under an absolute fallback dir (correct on
    // Linux, where OVMF lives outside the QEMU prefix). The SeaBIOS fallback
    // path is covered deterministically by args::tests (firmware: None).

    #[test]
    fn finds_aarch64_code() {
        let (tmp, bin) = fake_install(&["edk2-aarch64-code.fd"]);
        let f = find_aarch64_uefi(&bin).expect("aarch64 code found");
        assert!(f.ends_with("edk2-aarch64-code.fd"));
        let _ = fs::remove_dir_all(&tmp);
    }
}
