//! Disk image operations via `qemu-img`. Phase 1 needs qcow2 creation;
//! storage-engineer extends this with resize, snapshots, and clones.

use crate::error::{Error, Result};
use std::path::Path;

/// Argv for `qemu-img create -f qcow2 <path> <size>G`. Pure — unit-tested.
pub fn create_qcow2_args(path: &Path, size_gib: u32) -> Vec<String> {
    vec![
        "create".into(),
        "-f".into(),
        "qcow2".into(),
        path.display().to_string(),
        format!("{size_gib}G"),
    ]
}

/// Create a qcow2 disk. Idempotent: if the file already exists, succeeds
/// without touching it (so re-launching a VM keeps its disk).
pub async fn create_qcow2(path: &Path, size_gib: u32) -> Result<()> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let out = tokio::process::Command::new("qemu-img")
        .args(create_qcow2_args(path, size_gib))
        .output()
        .await
        .map_err(|e| Error::QemuNotFound(format!("qemu-img: {e}")))?;
    if !out.status.success() {
        return Err(Error::Process(format!(
            "qemu-img create failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn create_args() {
        let p = PathBuf::from("/vm/disk.qcow2");
        assert_eq!(
            create_qcow2_args(&p, 20),
            vec!["create", "-f", "qcow2", "/vm/disk.qcow2", "20G"]
        );
    }
}
