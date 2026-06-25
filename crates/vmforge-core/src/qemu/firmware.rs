//! x86-64 UEFI (OVMF) firmware discovery.
//!
//! q35 has a built-in SeaBIOS (legacy BIOS) and needs nothing to boot a legacy
//! ISO. UEFI guests — notably **Windows 11**, which requires UEFI — need OVMF: a
//! read-only CODE blob plus a writable VARS template, wired as two
//! `-drive if=pflash` units. When OVMF is absent the engine falls back to
//! SeaBIOS (and logs that UEFI-only guests won't boot).
//!
//! `qemu_bin` is the ALREADY-RESOLVED absolute QEMU path; firmware is searched
//! relative to it (`<install>/share/...`), which covers the qemu.org Windows
//! installer layout (exe in the install root) and a `bin/`+`share/` prefix
//! layout alike.

use std::path::{Path, PathBuf};

/// x86-64 OVMF (CODE, VARS) filename pairs, most-preferred first. Searched as
/// pairs — never mix a 4M CODE with a non-4M VARS. QEMU's own builds ship the
/// `edk2-*` names; distro OVMF packages use the `OVMF_*` names.
const X86_PAIRS: &[(&str, &str)] = &[
    ("edk2-x86_64-code.fd", "edk2-i386-vars.fd"),
    ("OVMF_CODE_4M.fd", "OVMF_VARS_4M.fd"),
    ("OVMF_CODE.fd", "OVMF_VARS.fd"),
];

/// x86-64 OVMF firmware: a read-only CODE blob + a writable VARS template. The
/// engine copies `vars_template` to a per-VM writable `OVMF_VARS.fd` before
/// launch (NVRAM must be writable and per-VM).
pub struct X86Firmware {
    pub code: PathBuf,
    pub vars_template: PathBuf,
}

/// `share` directories that may hold QEMU firmware, relative to the resolved
/// binary. Search BOTH the exe's own parent (qemu.org Windows installer: the exe
/// sits in the install root with firmware under `<root>\share\`) and its
/// grandparent (a `bin/`+`share/` prefix layout).
fn firmware_dirs(qemu_bin: &Path) -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let bin_dir = qemu_bin.parent();
    let prefix = bin_dir.and_then(Path::parent);
    for base in [bin_dir, prefix].into_iter().flatten() {
        let share = base.join("share");
        dirs.push(share.clone());
        dirs.push(share.join("qemu"));
        dirs.push(share.join("edk2"));
        dirs.push(share.join("OVMF"));
    }
    dirs
}

/// Locate x86-64 OVMF firmware (CODE + VARS, from the same dir, paired). Returns
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

    #[test]
    fn no_ovmf_pair_is_none_for_seabios_fallback() {
        // CODE present but no VARS → not a usable pair → None (→ SeaBIOS). The
        // fake install is self-contained (no absolute dirs are searched).
        let (tmp, bin) = fake_install(&["edk2-x86_64-code.fd"]);
        assert!(find_x86_64_uefi(&bin).is_none());
        let _ = fs::remove_dir_all(&tmp);
    }
}
