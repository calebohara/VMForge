//! GATED real-host Phase 3 integration: exercises the ACTUAL `qemu-img`
//! snapshot/clone paths, and (with `VMFORGE_ISO`) a live QMP `snapshot-save`/
//! `snapshot-delete` on a booted guest. Each test skips unless its env gate is
//! set, so the default CI suite stays fully offline. The unit suite mocks
//! `qemu-img`; this proves the real binary + real QMP job behavior.
//!
//!   offline:  VMFORGE_REAL=1 cargo test -p vmforge-core --test snapshots -- --nocapture
//!   live:     VMFORGE_ISO=<iso> cargo test -p vmforge-core --test snapshots -- --nocapture

use std::time::Duration;
use uuid::Uuid;
use vmforge_core::hypervisor::Hypervisor;
use vmforge_core::model::{DiskSpec, Hardware, NetworkConfig, VmConfig, VmState};
use vmforge_core::QemuHypervisor;

fn config(name: &str) -> VmConfig {
    VmConfig {
        id: Uuid::new_v4(),
        name: name.into(),
        schema_version: 1,
        dir_slug: String::new(),
        hardware: Hardware {
            cpus: 2,
            memory_mib: 1024,
        },
        disks: vec![DiskSpec {
            path: "disk.qcow2".into(),
            size_gib: 2,
            backing: None,
        }],
        network: NetworkConfig::default(),
        display: Default::default(),
        iso: None,
        metadata: Default::default(),
        snapshots: Vec::new(),
        shared_folders: Vec::new(),
        guest_arch: None,
    }
}

#[tokio::test]
async fn offline_snapshots_and_clones_real_qemu_img() {
    if std::env::var("VMFORGE_REAL").is_err() {
        eprintln!("SKIP: set VMFORGE_REAL=1 to run the real-qemu-img snapshot/clone test");
        return;
    }
    let tmp = std::env::temp_dir().join(format!("vmforge-snap-it-{}", Uuid::new_v4()));
    let hv = QemuHypervisor::with_library_dir(tmp.clone()).expect("hv");

    let created = hv
        .create_vm(config("Snap Source"))
        .await
        .expect("create_vm");
    let id = created.id.to_string();

    // Offline snapshot (VM stopped → `qemu-img snapshot -c`).
    let snap = hv
        .create_snapshot(&id, "base", None, "")
        .await
        .expect("create_snapshot");
    assert!(!snap.has_vm_state, "offline snapshot carries no vm-state");

    // It is really in the qcow2 (reconciled via `qemu-img info --output=json`).
    let nodes = hv.list_snapshots(&id).await.expect("list");
    let node = nodes
        .iter()
        .find(|n| n.meta.id == snap.id)
        .expect("snapshot listed");
    assert!(node.present_in_qcow2, "snapshot must exist in qcow2");

    // Restore (stopped, disk-only) — `qemu-img snapshot -a`.
    hv.restore_snapshot(&id, snap.id).await.expect("restore");

    // Full clone: deep copy, flattened (no backing).
    let full = hv
        .clone_vm(&id, "Full Clone", false)
        .await
        .expect("full clone");
    assert!(full.disks[0].backing.is_none(), "full clone is flattened");
    assert_ne!(full.id, created.id);

    // Linked clone: CoW overlay with a relative backing path to the parent.
    let linked = hv
        .clone_vm(&id, "Linked Clone", true)
        .await
        .expect("linked clone");
    let backing = linked.disks[0]
        .backing
        .clone()
        .expect("linked clone has a backing file");
    assert!(
        backing.contains(&created.dir_slug),
        "linked backing should reference the parent slug, got {backing}"
    );

    // Parent protection: the source now has a linked child → delete is refused.
    assert!(
        hv.delete(&id, true).await.is_err(),
        "a VM with linked children must refuse deletion"
    );

    // Deleting the snapshot itself is allowed (`qemu-img snapshot -d`).
    hv.delete_snapshot(&id, snap.id)
        .await
        .expect("delete snapshot");
    let after = hv.list_snapshots(&id).await.expect("list2");
    assert!(
        after.iter().all(|n| n.meta.id != snap.id),
        "snapshot must be gone after delete"
    );

    let _ = std::fs::remove_dir_all(&tmp);
    eprintln!("OK: offline snapshot + full/linked clone + parent-protection (real qemu-img)");
}

#[tokio::test]
async fn live_snapshot_over_qmp() {
    let Ok(iso) = std::env::var("VMFORGE_ISO") else {
        eprintln!("SKIP: set VMFORGE_ISO to run the live QMP snapshot test");
        return;
    };
    let tmp = std::env::temp_dir().join(format!("vmforge-livesnap-it-{}", Uuid::new_v4()));
    let hv = QemuHypervisor::with_library_dir(tmp.clone()).expect("hv");

    let mut cfg = config("Live Snap");
    cfg.iso = Some(iso);
    let created = hv.create_vm(cfg).await.expect("create_vm");
    let id = created.id.to_string();

    hv.start(&created).await.expect("start");
    tokio::time::sleep(Duration::from_millis(600)).await;
    assert_eq!(hv.state(&id).await.expect("state"), VmState::Running);

    // Live snapshot via the QMP `snapshot-save` job → captures vm-state.
    let snap = hv
        .create_snapshot(&id, "live1", None, "")
        .await
        .expect("live snapshot-save");
    assert!(snap.has_vm_state, "live snapshot must capture vm-state");
    eprintln!("live snapshot vm_state_size = {} bytes", snap.vm_state_size);

    let nodes = hv.list_snapshots(&id).await.expect("list");
    assert!(
        nodes
            .iter()
            .any(|n| n.meta.id == snap.id && n.present_in_qcow2),
        "live snapshot must be listed and present in qcow2"
    );

    // Live delete via the QMP `snapshot-delete` job.
    hv.delete_snapshot(&id, snap.id)
        .await
        .expect("live snapshot-delete");

    hv.kill(&id).await.expect("kill");
    let _ = std::fs::remove_dir_all(&tmp);
    eprintln!("OK: live QMP snapshot-save / list / snapshot-delete verified");
}
