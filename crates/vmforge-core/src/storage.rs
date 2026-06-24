//! Disk image operations via `qemu-img`. Phase 1 needs qcow2 creation;
//! storage-engineer extends this with resize, snapshots, and clones.

use crate::error::{Error, Result};
use std::path::Path;

/// The `qemu-img` binary to invoke. Overridable via `VMFORGE_QEMU_IMG` so CI
/// (and unit tests) can substitute a mock without a real QEMU install.
fn qemu_img_bin() -> String {
    std::env::var("VMFORGE_QEMU_IMG").unwrap_or_else(|_| "qemu-img".into())
}

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
    let out = tokio::process::Command::new(qemu_img_bin())
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

    // ---- (16) argv for qemu-img create (keep) ----
    #[test]
    fn create_args() {
        let p = PathBuf::from("/vm/disk.qcow2");
        assert_eq!(
            create_qcow2_args(&p, 20),
            vec!["create", "-f", "qcow2", "/vm/disk.qcow2", "20G"]
        );
    }

    // ---- (17, storage side) idempotent on existence: never invokes the
    // binary when the file already exists. The existence short-circuit runs
    // before the env seam is even read, so no env coordination is needed.
    #[tokio::test]
    async fn create_qcow2_idempotent_skips_binary() {
        let tmp = tempfile::tempdir().unwrap();
        let disk = tmp.path().join("disk.qcow2");
        tokio::fs::write(&disk, b"EXISTING").await.unwrap();

        let res = create_qcow2(&disk, 8).await;
        assert!(res.is_ok(), "idempotent create must skip the binary");
        let contents = tokio::fs::read(&disk).await.unwrap();
        assert_eq!(&contents, b"EXISTING", "existing disk must be untouched");
    }

    // The env seam resolves the binary name. Serializes through the shared env
    // guard because it mutates the process-global VMFORGE_QEMU_IMG.
    #[tokio::test]
    async fn qemu_img_bin_uses_env_seam() {
        let _g = crate::test_support::env_guard().await;
        std::env::set_var("VMFORGE_QEMU_IMG", "/opt/my-qemu-img");
        assert_eq!(qemu_img_bin(), "/opt/my-qemu-img");
        std::env::remove_var("VMFORGE_QEMU_IMG");
        assert_eq!(qemu_img_bin(), "qemu-img");
    }
}
