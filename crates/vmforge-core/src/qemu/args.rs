//! Pure QEMU argv construction. No process spawning here — that keeps the
//! whole arg surface unit-testable without QEMU installed.
//!
//! Owned by hypervisor-engineer (network fragments coordinated with
//! network-engineer).

use crate::host::Accelerator;
use crate::model::VmConfig;
use std::path::Path;

/// Where QEMU should expose its QMP control channel.
pub enum QmpEndpoint<'a> {
    /// Unix domain socket (macOS / Linux).
    UnixSocket(&'a Path),
    /// TCP loopback port (Windows; also usable anywhere).
    Tcp(u16),
}

/// Everything needed to construct a launch command line.
pub struct QemuLaunch<'a> {
    pub config: &'a VmConfig,
    pub accel: Accelerator,
    /// `"aarch64"` or `"x86_64"`.
    pub guest_arch: &'a str,
    pub disk: &'a Path,
    pub iso: Option<&'a Path>,
    /// aarch64 UEFI code blob (`edk2-aarch64-code.fd`); `None` for x86.
    pub firmware: Option<&'a Path>,
    /// VNC display number N → host port 5900 + N.
    pub vnc_display: u16,
    pub qmp: QmpEndpoint<'a>,
}

/// Build the full QEMU argument vector (everything after the binary name).
pub fn build_args(l: &QemuLaunch) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    let mut push = |s: String| a.push(s);
    let mut flag = |k: &str, v: String| {
        push(k.to_string());
        push(v);
    };

    flag("-name", l.config.name.clone());

    // Machine + CPU model depend on guest arch and accelerator.
    match l.guest_arch {
        "aarch64" => {
            flag("-machine", "virt".to_string());
            // `-cpu host` requires hardware accel; TCG needs a concrete core.
            let cpu = if l.accel == Accelerator::Hvf {
                "host"
            } else {
                "cortex-a72"
            };
            flag("-cpu", cpu.to_string());
        }
        _ => {
            flag("-machine", "q35".to_string());
            let cpu = if l.accel.is_hardware() { "host" } else { "qemu64" };
            flag("-cpu", cpu.to_string());
        }
    }

    flag("-accel", l.accel.as_qemu_arg().to_string());
    flag("-smp", l.config.hardware.cpus.to_string());
    flag("-m", l.config.hardware.memory_mib.to_string());

    // aarch64 `virt` has no built-in BIOS — UEFI firmware is required to boot.
    if let Some(fw) = l.firmware {
        flag("-bios", fw.display().to_string());
    }

    // Boot disk (virtio-blk).
    flag(
        "-drive",
        format!("file={},if=virtio,format=qcow2", l.disk.display()),
    );

    // Install media as a virtio CD-ROM. `-cdrom` defaults to if=ide, which the
    // aarch64 `virt` machine does not have — so attach explicitly via virtio.
    if let Some(iso) = l.iso {
        flag(
            "-drive",
            format!("file={},if=virtio,media=cdrom,format=raw", iso.display()),
        );
        flag("-boot", "order=dc".to_string());
    }

    // Networking: user-mode NAT (MVP). Bridged/host-only land later
    // (network-engineer). Port forwards apply in user mode.
    let mut netdev = String::from("user,id=net0");
    for pf in &l.config.network.port_forwards {
        let proto = if pf.udp { "udp" } else { "tcp" };
        netdev.push_str(&format!(",hostfwd={proto}::{}-:{}", pf.host, pf.guest));
    }
    flag("-netdev", netdev);
    let nic = match &l.config.network.mac {
        Some(mac) => format!("virtio-net-pci,netdev=net0,mac={mac}"),
        None => "virtio-net-pci,netdev=net0".to_string(),
    };
    flag("-device", nic);

    // Display: built-in VNC server on loopback (noVNC bridge connects here).
    flag("-vnc", format!("127.0.0.1:{}", l.vnc_display));

    // QMP control channel.
    match &l.qmp {
        QmpEndpoint::UnixSocket(p) => flag(
            "-qmp",
            format!("unix:{},server=on,wait=off", p.display()),
        ),
        QmpEndpoint::Tcp(port) => flag(
            "-qmp",
            format!("tcp:127.0.0.1:{port},server=on,wait=off"),
        ),
    }

    a
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Hardware, NetworkConfig, PortForward, VmConfig};
    use std::path::PathBuf;
    use uuid::Uuid;

    fn cfg() -> VmConfig {
        VmConfig {
            id: Uuid::nil(),
            name: "test-vm".into(),
            hardware: Hardware {
                cpus: 4,
                memory_mib: 2048,
            },
            disks: vec![],
            network: NetworkConfig::default(),
            display: Default::default(),
            iso: None,
        }
    }

    fn find_flag<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
        args.iter()
            .position(|a| a == key)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    }

    #[test]
    fn aarch64_hvf_uses_host_cpu_and_virt() {
        let c = cfg();
        let disk = PathBuf::from("/vm/disk.qcow2");
        let sock = PathBuf::from("/vm/qmp.sock");
        let args = build_args(&QemuLaunch {
            config: &c,
            accel: Accelerator::Hvf,
            guest_arch: "aarch64",
            disk: &disk,
            iso: None,
            firmware: Some(Path::new("/fw/edk2-aarch64-code.fd")),
            vnc_display: 1,
            qmp: QmpEndpoint::UnixSocket(&sock),
        });
        assert_eq!(find_flag(&args, "-machine"), Some("virt"));
        assert_eq!(find_flag(&args, "-accel"), Some("hvf"));
        assert_eq!(find_flag(&args, "-cpu"), Some("host"));
        assert_eq!(find_flag(&args, "-smp"), Some("4"));
        assert_eq!(find_flag(&args, "-m"), Some("2048"));
        assert_eq!(find_flag(&args, "-bios"), Some("/fw/edk2-aarch64-code.fd"));
        assert_eq!(find_flag(&args, "-vnc"), Some("127.0.0.1:1"));
        assert_eq!(
            find_flag(&args, "-qmp"),
            Some("unix:/vm/qmp.sock,server=on,wait=off")
        );
    }

    #[test]
    fn tcg_aarch64_uses_concrete_cpu() {
        let c = cfg();
        let disk = PathBuf::from("/vm/disk.qcow2");
        let args = build_args(&QemuLaunch {
            config: &c,
            accel: Accelerator::Tcg,
            guest_arch: "aarch64",
            disk: &disk,
            iso: None,
            firmware: Some(Path::new("/fw/x.fd")),
            vnc_display: 0,
            qmp: QmpEndpoint::Tcp(4444),
        });
        assert_eq!(find_flag(&args, "-cpu"), Some("cortex-a72"));
        assert_eq!(
            find_flag(&args, "-qmp"),
            Some("tcp:127.0.0.1:4444,server=on,wait=off")
        );
    }

    #[test]
    fn iso_attaches_virtio_cdrom() {
        let mut c = cfg();
        c.iso = Some("/iso/alpine.iso".into());
        let disk = PathBuf::from("/vm/disk.qcow2");
        let sock = PathBuf::from("/vm/qmp.sock");
        let args = build_args(&QemuLaunch {
            config: &c,
            accel: Accelerator::Hvf,
            guest_arch: "aarch64",
            disk: &disk,
            iso: Some(Path::new("/iso/alpine.iso")),
            firmware: Some(Path::new("/fw/x.fd")),
            vnc_display: 2,
            qmp: QmpEndpoint::UnixSocket(&sock),
        });
        let joined = args.join(" ");
        assert!(joined.contains("file=/iso/alpine.iso,if=virtio,media=cdrom,format=raw"));
        assert_eq!(find_flag(&args, "-boot"), Some("order=dc"));
    }

    #[test]
    fn port_forwards_render_hostfwd() {
        let mut c = cfg();
        c.network.port_forwards = vec![PortForward {
            host: 2222,
            guest: 22,
            udp: false,
        }];
        let disk = PathBuf::from("/vm/disk.qcow2");
        let sock = PathBuf::from("/vm/qmp.sock");
        let args = build_args(&QemuLaunch {
            config: &c,
            accel: Accelerator::Hvf,
            guest_arch: "aarch64",
            disk: &disk,
            iso: None,
            firmware: Some(Path::new("/fw/x.fd")),
            vnc_display: 1,
            qmp: QmpEndpoint::UnixSocket(&sock),
        });
        assert_eq!(
            find_flag(&args, "-netdev"),
            Some("user,id=net0,hostfwd=tcp::2222-:22")
        );
    }
}
