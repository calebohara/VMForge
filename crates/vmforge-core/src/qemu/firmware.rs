//! UEFI firmware discovery for the aarch64 `virt` machine, which has no
//! built-in BIOS. x86 guests use the built-in SeaBIOS and need none.

use std::path::{Path, PathBuf};

const AARCH64_CODE: &str = "edk2-aarch64-code.fd";

/// Locate the aarch64 UEFI code blob. Searches near the QEMU binary, then a
/// few well-known install prefixes. Returns `None` if not found.
pub fn find_aarch64_uefi(qemu_system_bin: &str) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();

    // 1. <bin_dir>/../share/qemu (relative to the resolved binary).
    if let Some(bin) = which(qemu_system_bin) {
        if let Some(prefix) = bin.parent().and_then(Path::parent) {
            dirs.push(prefix.join("share").join("qemu"));
        }
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

/// Minimal cross-platform `which`: first match for `bin` on `PATH`.
fn which(bin: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    let exe_suffixes: &[&str] = if cfg!(windows) { &["", ".exe"] } else { &[""] };
    for dir in std::env::split_paths(&path) {
        for suffix in exe_suffixes {
            let cand = dir.join(format!("{bin}{suffix}"));
            if cand.is_file() {
                return Some(cand);
            }
        }
    }
    None
}
