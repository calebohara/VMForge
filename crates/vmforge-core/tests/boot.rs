//! Integration: boot a real guest headless and control it over QMP.
//!
//! GATED — runs only when `VMFORGE_ISO` points at a bootable ISO, so the
//! default `cargo test` suite stays fully offline (no QEMU/VM in CI). This is
//! the Phase 1 engine proof-of-life: spawn QEMU, complete the QMP handshake,
//! observe `running`, pause→`paused`, resume→`running`, then force-kill.
//!
//! Run on this host:
//!   VMFORGE_ISO=.vmforge-data/isos/alpine-virt-3.24.1-aarch64.iso \
//!     cargo test -p vmforge-core --test boot -- --nocapture

use std::time::Duration;
use uuid::Uuid;
use vmforge_core::hypervisor::Hypervisor;
use vmforge_core::model::{DiskSpec, Hardware, NetworkConfig, VmConfig, VmState};
use vmforge_core::QemuHypervisor;

#[tokio::test]
async fn boots_real_guest_and_controls_via_qmp() {
    let Ok(iso) = std::env::var("VMFORGE_ISO") else {
        eprintln!("SKIP: set VMFORGE_ISO to a bootable ISO path to run this test");
        return;
    };

    let tmp = std::env::temp_dir().join(format!("vmforge-it-{}", Uuid::new_v4()));
    let hv = QemuHypervisor::with_library_dir(tmp.clone()).expect("build hypervisor");
    eprintln!("accelerator: {:?}", hv.accelerator());

    let config = VmConfig {
        id: Uuid::new_v4(),
        name: "it-alpine".into(),
        hardware: Hardware {
            cpus: 2,
            memory_mib: 1024,
        },
        disks: vec![DiskSpec {
            path: "disk.qcow2".into(),
            size_gib: 4,
            backing: None,
        }],
        network: NetworkConfig::default(),
        display: Default::default(),
        iso: Some(iso),
    };
    let id = config.id.to_string();

    hv.start(&config).await.expect("start VM");

    // Reaches running (QEMU starts the CPU unless -S is passed).
    let state = hv.state(&id).await.expect("query state");
    eprintln!("post-start state: {state:?}");
    assert!(
        matches!(state, VmState::Running | VmState::Starting),
        "unexpected post-start state: {state:?}"
    );

    // VNC display was allocated in range.
    let port = hv.vnc_port(&id).await.expect("vnc port");
    eprintln!("VNC host port: {port}");
    assert!((5901..=5963).contains(&port), "vnc port out of range: {port}");

    // pause -> paused
    hv.pause(&id).await.expect("pause");
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(hv.state(&id).await.expect("state after pause"), VmState::Paused);

    // resume -> running
    hv.resume(&id).await.expect("resume");
    tokio::time::sleep(Duration::from_millis(250)).await;
    assert_eq!(
        hv.state(&id).await.expect("state after resume"),
        VmState::Running
    );

    // Teardown: force-kill the QEMU process.
    hv.kill(&id).await.expect("kill");

    let _ = std::fs::remove_dir_all(&tmp);
    eprintln!("OK: engine boot + QMP control verified");
}
