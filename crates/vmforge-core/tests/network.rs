//! GATED real-host Phase 4 integration: a real QEMU launch actually binds the
//! NAT host-forward port on 127.0.0.1. No guest OS needed — QEMU's user-net
//! (slirp) opens the host listening socket at launch, so a loopback connect
//! succeeds once the VM is up. Skips unless `VMFORGE_REAL=1`, so the default CI
//! suite stays offline (unit tests cover the arg construction).
//!
//!   VMFORGE_REAL=1 cargo test -p vmforge-core --test network -- --nocapture

use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};
use uuid::Uuid;
use vmforge_core::hypervisor::Hypervisor;
use vmforge_core::model::{DiskSpec, Hardware, NetworkConfig, NetworkMode, PortForward, VmConfig};
use vmforge_core::QemuHypervisor;

fn free_high_port() -> u16 {
    TcpListener::bind(("127.0.0.1", 0))
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

#[tokio::test]
async fn nat_port_forward_binds_host_port() {
    if std::env::var("VMFORGE_REAL").is_err() {
        eprintln!("SKIP: set VMFORGE_REAL=1 to run the real NAT port-forward bind test");
        return;
    }
    let host_port = free_high_port();
    let tmp = std::env::temp_dir().join(format!("vmforge-net-it-{}", Uuid::new_v4()));
    let hv = QemuHypervisor::with_library_dir(tmp.clone()).expect("hv");

    let cfg = VmConfig {
        id: Uuid::new_v4(),
        name: "Net Fwd".into(),
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
        network: NetworkConfig {
            mode: NetworkMode::User,
            mac: None,
            port_forwards: vec![PortForward {
                host: host_port,
                guest: 22,
                udp: false,
                expose_lan: false, // default: bind 127.0.0.1 only
            }],
        },
        display: Default::default(),
        // No ISO: boots to the UEFI shell. QEMU still binds the hostfwd socket
        // at launch, which is exactly what we're verifying.
        iso: None,
        metadata: Default::default(),
        snapshots: Vec::new(),
    };

    let created = hv.create_vm(cfg).await.expect("create_vm");
    let id = created.id.to_string();
    hv.start(&created).await.expect("start");

    // slirp binds the forward at launch; a loopback connect should succeed
    // shortly after QMP is up. Retry briefly.
    let deadline = Instant::now() + Duration::from_secs(5);
    let mut connected = false;
    while Instant::now() < deadline {
        if TcpStream::connect(("127.0.0.1", host_port)).is_ok() {
            connected = true;
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }

    hv.kill(&id).await.expect("kill");
    let _ = std::fs::remove_dir_all(&tmp);

    assert!(
        connected,
        "NAT host-forward should bind 127.0.0.1:{host_port}"
    );
    eprintln!("OK: NAT port-forward bound 127.0.0.1:{host_port} (real QEMU)");
}
