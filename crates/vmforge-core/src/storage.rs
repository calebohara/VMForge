//! Disk image operations via `qemu-img`. Phase 1 needs qcow2 creation;
//! storage-engineer extends this with resize, snapshots, and clones.

use crate::error::{Error, Result};
use serde::Deserialize;
use std::path::Path;

/// The `qemu-img` binary to invoke. `VMFORGE_QEMU_IMG` wins (CI/test mock seam);
/// otherwise resolve to an ABSOLUTE path via the shared D3 resolver — same as
/// the qemu-system path — so a Finder-launched `.app` with an empty inherited
/// `PATH` (and any "Locate QEMU…" override) still finds qemu-img. Falls back to
/// the bare name only if resolution fails.
fn qemu_img_bin() -> std::ffi::OsString {
    if let Some(v) = std::env::var_os("VMFORGE_QEMU_IMG") {
        return v;
    }
    crate::qemu_resolve::resolve_qemu_binary("qemu-img")
        .map(|p| p.into_os_string())
        .unwrap_or_else(|| "qemu-img".into())
}

/// One internal qcow2 snapshot, as reported by
/// `qemu-img info --output=json` under `.snapshots[]`. Field names verified on
/// qemu-img 11.0.1. Extra fields (icount, vm-clock-sec, …) are ignored.
#[derive(Debug, Clone, Deserialize)]
pub struct Qcow2Snapshot {
    /// qcow2 numeric snapshot id (assigned by qemu-img; not our id).
    pub id: String,
    /// The snapshot tag — equals our `Snapshot.id.to_string()`.
    pub name: String,
    #[serde(rename = "date-sec")]
    pub date_sec: i64,
    #[serde(rename = "date-nsec")]
    pub date_nsec: i64,
    #[serde(rename = "vm-state-size")]
    pub vm_state_size: u64,
    #[serde(rename = "vm-clock-nsec")]
    pub vm_clock_nsec: u64,
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

// ---------------------------------------------------------------------------
// Pure argv builders (Vec<String>). Snapshot tags are our `Snapshot.id` as a
// string. These take a disk path + tag and produce the exact `qemu-img`
// argument vector — unit-tested without QEMU installed.
// ---------------------------------------------------------------------------

/// Argv for `qemu-img snapshot -c <tag> <disk>` (create an offline snapshot).
pub fn snapshot_create_args(disk: &Path, tag: &str) -> Vec<String> {
    vec![
        "snapshot".into(),
        "-c".into(),
        tag.to_string(),
        disk.display().to_string(),
    ]
}

/// Argv for `qemu-img snapshot -a <tag> <disk>` (apply/restore a snapshot).
pub fn snapshot_apply_args(disk: &Path, tag: &str) -> Vec<String> {
    vec![
        "snapshot".into(),
        "-a".into(),
        tag.to_string(),
        disk.display().to_string(),
    ]
}

/// Argv for `qemu-img snapshot -d <tag> <disk>` (delete a snapshot).
pub fn snapshot_delete_args(disk: &Path, tag: &str) -> Vec<String> {
    vec![
        "snapshot".into(),
        "-d".into(),
        tag.to_string(),
        disk.display().to_string(),
    ]
}

/// Argv for `qemu-img info --output=json [-U] <disk>`. `force_share` adds `-U`
/// (`--force-share`) for safe reads while QEMU may hold the image open RW.
pub fn info_json_args(disk: &Path, force_share: bool) -> Vec<String> {
    let mut a: Vec<String> = vec!["info".into(), "--output=json".into()];
    if force_share {
        a.push("-U".into());
    }
    a.push(disk.display().to_string());
    a
}

/// Argv for `qemu-img convert -O qcow2 <src> <dst>` (full/deep copy). The
/// result is flattened — no backing, snapshots not carried.
pub fn convert_args(src: &Path, dst: &Path) -> Vec<String> {
    vec![
        "convert".into(),
        "-O".into(),
        "qcow2".into(),
        src.display().to_string(),
        dst.display().to_string(),
    ]
}

/// Argv for a linked overlay:
/// `qemu-img create -f qcow2 --backing <parent> --backing-format qcow2 <child>`.
/// Long flags ONLY — `-F`/`-B` are deprecated in qemu-img > 10.0.
pub fn linked_overlay_args(child: &Path, parent_backing: &str) -> Vec<String> {
    vec![
        "create".into(),
        "-f".into(),
        "qcow2".into(),
        "--backing".into(),
        parent_backing.to_string(),
        "--backing-format".into(),
        "qcow2".into(),
        child.display().to_string(),
    ]
}

// ---------------------------------------------------------------------------
// Async runners. Each spawns `qemu-img` (honoring the VMFORGE_QEMU_IMG seam)
// and mirrors `create_qcow2`'s error mapping: spawn failure → QemuNotFound,
// non-zero exit → Process with the captured stderr.
// ---------------------------------------------------------------------------

/// Run `qemu-img` with the given argv, mapping errors like `create_qcow2`.
/// `op` names the operation for the error message.
async fn run_qemu_img(op: &str, args: Vec<String>) -> Result<std::process::Output> {
    let out = tokio::process::Command::new(qemu_img_bin())
        .args(args)
        .output()
        .await
        .map_err(|e| Error::QemuNotFound(format!("qemu-img: {e}")))?;
    if !out.status.success() {
        return Err(Error::Process(format!(
            "qemu-img {op} failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        )));
    }
    Ok(out)
}

/// Create an offline (disk-only) snapshot via `qemu-img snapshot -c`.
pub async fn snapshot_create_offline(disk: &Path, tag: &str) -> Result<()> {
    run_qemu_img("snapshot -c", snapshot_create_args(disk, tag)).await?;
    Ok(())
}

/// Apply (restore) an offline snapshot via `qemu-img snapshot -a`. Disk-only.
pub async fn snapshot_apply_offline(disk: &Path, tag: &str) -> Result<()> {
    run_qemu_img("snapshot -a", snapshot_apply_args(disk, tag)).await?;
    Ok(())
}

/// Delete an offline snapshot via `qemu-img snapshot -d`.
pub async fn snapshot_delete_offline(disk: &Path, tag: &str) -> Result<()> {
    run_qemu_img("snapshot -d", snapshot_delete_args(disk, tag)).await?;
    Ok(())
}

/// Run `qemu-img info --output=json` and return raw stdout. `force_share`
/// adds `-U` for reads while QEMU may hold the image open RW.
pub async fn info_json(disk: &Path, force_share: bool) -> Result<String> {
    let out = run_qemu_img("info", info_json_args(disk, force_share)).await?;
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Deep-copy `src` to `dst` (flattened qcow2). Caller renames any `*.partial`.
pub async fn convert_qcow2(src: &Path, dst: &Path) -> Result<()> {
    run_qemu_img("convert", convert_args(src, dst)).await?;
    Ok(())
}

/// Create a linked CoW overlay `child` backed by `parent_backing`.
pub async fn create_linked_overlay(child: &Path, parent_backing: &str) -> Result<()> {
    run_qemu_img(
        "create (linked)",
        linked_overlay_args(child, parent_backing),
    )
    .await?;
    Ok(())
}

/// Parse `qemu-img info --output=json` stdout into the internal snapshot list.
/// A missing `snapshots` key yields an empty vec (no internal snapshots);
/// malformed JSON or a mismatched shape maps to [`Error::Serde`].
pub fn parse_info_snapshots(stdout: &str) -> Result<Vec<Qcow2Snapshot>> {
    #[derive(Deserialize)]
    struct Info {
        #[serde(default)]
        snapshots: Vec<Qcow2Snapshot>,
    }
    let info: Info = serde_json::from_str(stdout)?;
    Ok(info.snapshots)
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

    // The env seam wins verbatim; otherwise qemu_img_bin resolves (D3) to an
    // absolute path where qemu-img exists, falling back to the bare name. Either
    // way the file name is "qemu-img". Serializes through the shared env guard
    // because it mutates the process-global VMFORGE_QEMU_IMG.
    #[tokio::test]
    async fn qemu_img_bin_uses_env_seam() {
        let _g = crate::test_support::env_guard().await;
        std::env::set_var("VMFORGE_QEMU_IMG", "/opt/my-qemu-img");
        assert_eq!(qemu_img_bin(), std::ffi::OsString::from("/opt/my-qemu-img"));
        std::env::remove_var("VMFORGE_QEMU_IMG");
        let resolved = qemu_img_bin();
        assert_eq!(
            std::path::Path::new(&resolved)
                .file_name()
                .and_then(|s| s.to_str()),
            Some("qemu-img"),
            "resolved qemu-img path should end in 'qemu-img', got {resolved:?}"
        );
    }

    // ---- pure argv builders (§E offline) ----
    #[test]
    fn snapshot_create_argv() {
        let disk = PathBuf::from("/vm/disk.qcow2");
        assert_eq!(
            snapshot_create_args(&disk, "tag-1"),
            vec!["snapshot", "-c", "tag-1", "/vm/disk.qcow2"]
        );
    }

    #[test]
    fn snapshot_apply_argv() {
        let disk = PathBuf::from("/vm/disk.qcow2");
        assert_eq!(
            snapshot_apply_args(&disk, "tag-1"),
            vec!["snapshot", "-a", "tag-1", "/vm/disk.qcow2"]
        );
    }

    #[test]
    fn snapshot_delete_argv() {
        let disk = PathBuf::from("/vm/disk.qcow2");
        assert_eq!(
            snapshot_delete_args(&disk, "tag-1"),
            vec!["snapshot", "-d", "tag-1", "/vm/disk.qcow2"]
        );
    }

    #[test]
    fn info_json_argv_without_force_share() {
        let disk = PathBuf::from("/vm/disk.qcow2");
        assert_eq!(
            info_json_args(&disk, false),
            vec!["info", "--output=json", "/vm/disk.qcow2"]
        );
    }

    #[test]
    fn info_json_argv_with_force_share_adds_capital_u() {
        let disk = PathBuf::from("/vm/disk.qcow2");
        assert_eq!(
            info_json_args(&disk, true),
            vec!["info", "--output=json", "-U", "/vm/disk.qcow2"]
        );
    }

    #[test]
    fn convert_argv() {
        let src = PathBuf::from("/vm/src.qcow2");
        let dst = PathBuf::from("/vm/dst.qcow2");
        assert_eq!(
            convert_args(&src, &dst),
            vec!["convert", "-O", "qcow2", "/vm/src.qcow2", "/vm/dst.qcow2"]
        );
    }

    // ---- (§E regression guard) linked overlay uses LONG flags only ----
    #[test]
    fn linked_overlay_argv_uses_long_flags() {
        let child = PathBuf::from("/vm/child.qcow2");
        let args = linked_overlay_args(&child, "../parent/disk.qcow2");
        assert_eq!(
            args,
            vec![
                "create",
                "-f",
                "qcow2",
                "--backing",
                "../parent/disk.qcow2",
                "--backing-format",
                "qcow2",
                "/vm/child.qcow2",
            ]
        );
    }

    #[test]
    fn linked_overlay_never_emits_deprecated_short_flags() {
        // `-F` (backing-format) and `-B` (backing-file) are deprecated in
        // qemu-img > 10.0. Guard against a regression to the short forms.
        let child = PathBuf::from("/vm/child.qcow2");
        let args = linked_overlay_args(&child, "../parent/disk.qcow2");
        assert!(
            !args.iter().any(|a| a == "-F"),
            "must never emit -F: {args:?}"
        );
        assert!(
            !args.iter().any(|a| a == "-B"),
            "must never emit -B: {args:?}"
        );
        assert!(args.iter().any(|a| a == "--backing"));
        assert!(args.iter().any(|a| a == "--backing-format"));
    }

    // ---- parse_info_snapshots (captured 11.0.1 JSON fixture) ----
    #[test]
    fn parse_info_snapshots_from_fixture() {
        let fixture = include_str!("../tests/fixtures/qemu_img_info_snapshots.json");
        let snaps = parse_info_snapshots(fixture).expect("parse fixture");
        assert_eq!(snaps.len(), 2);
        assert_eq!(snaps[0].id, "1");
        assert_eq!(snaps[0].name, "550e8400-e29b-41d4-a716-446655440000");
        assert_eq!(snaps[0].date_sec, 1782342711);
        assert_eq!(snaps[0].date_nsec, 276293000);
        assert_eq!(snaps[0].vm_state_size, 0);
        assert_eq!(snaps[0].vm_clock_nsec, 0);
        assert_eq!(snaps[1].id, "2");
        assert_eq!(snaps[1].name, "6ba7b810-9dad-11d1-80b4-00c04fd430c8");
    }

    #[test]
    fn parse_info_snapshots_missing_key_is_empty() {
        let json = r#"{"virtual-size": 67108864, "format": "qcow2"}"#;
        let snaps = parse_info_snapshots(json).expect("missing key => empty");
        assert!(snaps.is_empty());
    }

    #[test]
    fn parse_info_snapshots_malformed_is_serde_error() {
        let res = parse_info_snapshots("not json {");
        assert!(matches!(res, Err(Error::Serde(_))), "got {res:?}");
    }
}
