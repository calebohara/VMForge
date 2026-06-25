//! UEFI firmware discovery for the aarch64 `virt` machine, which has no
//! built-in BIOS. x86 guests use the built-in SeaBIOS and need none.

use std::path::{Path, PathBuf};

const AARCH64_CODE: &str = "edk2-aarch64-code.fd";

/// Locate the aarch64 UEFI code blob. `qemu_bin` is the ALREADY-RESOLVED
/// absolute QEMU binary path (D3) — derive `<prefix>/share/qemu` from it (no
/// second resolve / `--version` spawn), then fall back to well-known install
/// prefixes. Returns `None` if not found.
pub fn find_aarch64_uefi(qemu_bin: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    // 1. <bin_dir>/../share/qemu, relative to the already-resolved binary.
    if let Some(prefix) = qemu_bin.parent().and_then(Path::parent) {
        dirs.push(prefix.join("share").join("qemu"));
    }

    // 2. Common locations (Homebrew, system, MSYS2).
    for c in [
        "/opt/homebrew/share/qemu",
        "/opt/homebrew/opt/qemu/share/qemu",
        "/usr/local/share/qemu",
        "/usr/share/qemu",
        "/usr/share/edk2/aarch64",
    ] {
        dirs.push(PathBuf::from(c));
    }

    for d in dirs {
        let f = d.join(AARCH64_CODE);
        if f.is_file() {
            return Some(f);
        }
    }
    None
}
