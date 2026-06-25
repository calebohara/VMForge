//! GATED real-host Phase 5 integration. Skips unless `VMFORGE_REAL=1`.
//!
//! 1. 9p shared folder: a real QEMU launch ACCEPTS the -fsdev/-device 9p args
//!    (a malformed pair would make QEMU exit before QMP, failing start) and the
//!    suspend gate refuses on HVF.
//! 2. Suspend/resume round-trip: forced under TCG (HVF cannot snapshot-load),
//!    proving the engine's stop→snapshot-save→kill / relaunch -S→snapshot-load
//!    →cont path end-to-end against real QEMU + qemu-img.
//!
//!   VMFORGE_REAL=1 cargo test -p vmforge-core --test phase5 -- --nocapture

use std::time::Duration;
use uuid::Uuid;
use vmforge_core::host::Accelerator;
use vmforge_core::hypervisor::Hypervisor;
use vmforge_core::model::{DiskSpec, Hardware, NetworkConfig, SharedFolder, VmConfig, VmState};
use vmforge_core::QemuHypervisor;

fn base_config(name: &str) -> VmConfig {
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
        iso: None, // boots to UEFI shell; enough to exercise device args + vmstate
        metadata: Default::default(),
        snapshots: Vec::new(),
        shared_folders: Vec::new(),
    }
}

#[tokio::test]
async fn phase5_real_host() {
    if std::env::var("VMFORGE_REAL").is_err() {
        eprintln!("SKIP: set VMFORGE_REAL=1 to run the real Phase 5 (9p + suspend/resume) tests");
        return;
    }
    // Run sequentially: each part boots a real VM, and concurrent boots contend
    // for host resources (a flaky QMP-connect timeout otherwise).
    shared_folder_and_suspend_gate().await;
    suspend_resume_round_trip_tcg().await;
}

async fn shared_folder_and_suspend_gate() {
    let tmp = std::env::temp_dir().join(format!("vmforge-9p-it-{}", Uuid::new_v4()));
    let share = tmp.join("share");
    std::fs::create_dir_all(&share).unwrap();
    let hv = QemuHypervisor::with_library_dir(tmp.clone()).expect("hv");

    let mut cfg = base_config("Share Host");
    cfg.shared_folders = vec![SharedFolder {
        host_path: share.to_string_lossy().into_owned(),
        mount_tag: "hostshare".into(),
        read_only: false,
    }];

    let created = hv.create_vm(cfg).await.expect("create_vm");
    let id = created.id.to_string();
    // If the -fsdev/-device 9p args were malformed, QEMU would exit before QMP
    // and start() would error — so a clean start IS the device-accepted proof.
    hv.start(&created).await.expect("start with 9p share");
    assert!(matches!(
        hv.state(&id).await.expect("state"),
        VmState::Running | VmState::Starting
    ));

    // Suspend must be refused under HVF (accelerator gate).
    if hv.accelerator() == Accelerator::Hvf {
        let err = hv
            .suspend(&id)
            .await
            .expect_err("suspend must refuse on HVF");
        assert!(
            err.to_string().contains("hardware acceleration"),
            "unexpected suspend error: {err}"
        );
    }

    hv.kill(&id).await.expect("kill");
    let _ = std::fs::remove_dir_all(&tmp);
    eprintln!("OK: 9p share accepted by real QEMU; suspend gated on HVF");
}

async fn suspend_resume_round_trip_tcg() {
    let tmp = std::env::temp_dir().join(format!("vmforge-suspend-it-{}", Uuid::new_v4()));
    // Force TCG — HVF crashes on snapshot-load (the whole reason suspend is gated).
    let hv = QemuHypervisor::with_library_dir_and_accel(tmp.clone(), Accelerator::Tcg).expect("hv");

    let created = hv
        .create_vm(base_config("Suspendable"))
        .await
        .expect("create_vm");
    let id = created.id.to_string();
    hv.start(&created).await.expect("start (tcg)");
    // TCG aarch64 boot to the UEFI shell; give it a moment to reach running.
    tokio::time::sleep(Duration::from_secs(2)).await;
    assert_eq!(hv.state(&id).await.expect("state"), VmState::Running);

    // Suspend: stop + snapshot-save vmstate, persist tag, kill the process.
    hv.suspend(&id).await.expect("suspend");
    let after = hv.get_config(&id).await.expect("get_config");
    assert!(
        after.metadata.suspended_snapshot.is_some(),
        "suspend must record a vmstate tag"
    );
    // Process is gone → effective state is Stopped.
    assert_eq!(
        hv.state(&id).await.unwrap_or(VmState::Stopped),
        VmState::Stopped
    );

    // Resume: relaunch -S, snapshot-load, cont → running; tag cleared.
    hv.restore_suspended(&id).await.expect("restore_suspended");
    assert_eq!(
        hv.state(&id).await.expect("state after resume"),
        VmState::Running
    );
    let resumed = hv.get_config(&id).await.expect("get_config 2");
    assert!(
        resumed.metadata.suspended_snapshot.is_none(),
        "resume must clear the vmstate tag"
    );

    hv.kill(&id).await.expect("kill");
    let _ = std::fs::remove_dir_all(&tmp);
    eprintln!(
        "OK: TCG suspend → resume round-trip verified (snapshot-save + -S/snapshot-load/cont)"
    );
}
