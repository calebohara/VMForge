//! Cross-platform filesystem layout for the VM library. Per the spec, VMs
//! live under `~/VMForge/<vm-name>/`. Never hardcode `~` or POSIX paths —
//! resolve the home dir via the `directories` crate.

use crate::error::{Error, Result};
use directories::BaseDirs;
use std::path::{Path, PathBuf};

/// The VMForge library root: `~/VMForge`.
pub fn library_dir() -> Result<PathBuf> {
    let base =
        BaseDirs::new().ok_or_else(|| Error::Other("cannot determine home directory".into()))?;
    Ok(base.home_dir().join("VMForge"))
}

/// Directory for a single VM within a library root.
pub fn vm_dir(library: &Path, name: &str) -> PathBuf {
    library.join(name)
}

/// Short runtime directory for control sockets.
///
/// Unix-domain socket paths must fit in `sockaddr_un` (~104 bytes on macOS),
/// so sockets must NOT live under the (potentially long) VM data dir. Prefer
/// `$XDG_RUNTIME_DIR` (Linux), else a short `/tmp/vmforge`.
#[cfg(unix)]
pub fn runtime_dir() -> PathBuf {
    if let Some(x) = std::env::var_os("XDG_RUNTIME_DIR") {
        return PathBuf::from(x).join("vmforge");
    }
    PathBuf::from("/tmp/vmforge")
}

/// QMP control-socket path for a VM id — kept short on purpose (see
/// [`runtime_dir`]).
#[cfg(unix)]
pub fn qmp_socket_path(id: &str) -> PathBuf {
    runtime_dir().join(format!("{id}.sock"))
}
