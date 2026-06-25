//! GATED real-host test for the Windows-readiness x86_64 path. Exercises the
//! ENTIRE foreign-arch launch on this host (aarch64 → x86_64 guest, which the
//! engine auto-downgrades to TCG): resolve `qemu-system-x86_64`, locate + stage
//! OVMF (`OVMF_VARS.fd` copied per-VM), build a valid q35/pflash command line,
//! boot to the OVMF UEFI shell (no ISO), and confirm QMP reports `Running`.
//!
//! Skips unless `VMFORGE_REAL=1`, so the default CI suite stays offline. Run:
//!   VMFORGE_REAL=1 cargo test -p vmforge-core --test x86_guest -- --nocapture

use std::time::Duration;
use uuid::Uuid;
use vmforge_core::hypervisor::Hypervisor;
use vmforge_core::model::{DiskSpec, Hardware, NetworkConfig, VmConfig, VmState};
use vmforge_core::QemuHypervisor;

fn x86_config(name: &str) -> VmConfig {
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
        iso: None, // boots to the OVMF UEFI shell — enough to prove the path
        metadata: Default::default(),
        snapshots: Vec::new(),
        shared_folders: Vec::new(),
    }
}

#[tokio::test]
async fn x86_64_guest_boots_under_tcg_with_ovmf() {
    if std::env::var("VMFORGE_REAL").is_err() {
        eprintln!("SKIP: set VMFORGE_REAL=1 to run the real-host x86_64 launch test");
        return;
    }
    let tmp = std::env::temp_dir().join(format!("vmforge-x86-it-{}", Uuid::new_v4()));
    let hv = QemuHypervisor::with_library_dir(tmp.clone()).expect("hv");

    let created = hv
        .create_vm(x86_config("X86 Guest"))
        .await
        .expect("create_vm");
    let id = created.id.to_string();

    // Start. This resolves qemu-system-x86_64, locates+stages OVMF, downgrades
    // to TCG (x86 guest on a non-x86 host), and brings up QMP.
    hv.start(&created).await.expect("start x86_64 guest");

    // Give QEMU+TCG a moment to come up, then confirm QMP says Running.
    tokio::time::sleep(Duration::from_millis(1500)).await;
    let state = hv.state(&id).await.expect("state");
    assert_eq!(state, VmState::Running, "x86_64 guest should be Running");

    // OVMF NVRAM must have been staged per-VM (UEFI firmware present → pflash).
    // (If the host lacks OVMF the engine falls back to SeaBIOS and this file is
    // absent — Homebrew QEMU ships OVMF, so on this host it must exist.)
    let vars = tmp.join(&created.dir_slug).join("OVMF_VARS.fd");
    assert!(
        vars.is_file(),
        "expected per-VM OVMF_VARS.fd staged at {}",
        vars.display()
    );
    // The staged NVRAM must be WRITABLE — pflash unit 1 is opened read-write, and
    // some OVMF VARS templates ship read-only (fs::copy preserves mode bits).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mode = std::fs::metadata(&vars).unwrap().permissions().mode();
        assert!(
            mode & 0o200 != 0,
            "staged OVMF_VARS.fd must be writable, got mode {mode:o}"
        );
    }

    hv.kill(&id).await.expect("kill");
    let _ = std::fs::remove_dir_all(&tmp);
    eprintln!("OK: x86_64 guest booted under TCG with OVMF (qemu-system-x86_64 + pflash)");
}
