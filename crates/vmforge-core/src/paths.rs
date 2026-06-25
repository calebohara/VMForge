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

/// Filename of a VM's persisted configuration within its directory.
pub const CONFIG_FILENAME: &str = "vmforge.toml";

/// Directory for a single VM within a library root.
pub fn vm_dir(library: &Path, name: &str) -> PathBuf {
    library.join(name)
}

/// Path to a VM's `vmforge.toml`, addressed by its directory slug.
pub fn vm_config_path(library: &Path, slug: &str) -> PathBuf {
    vm_dir(library, slug).join(CONFIG_FILENAME)
}
