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
    /// Pre-built `-netdev`/`-device` network fragments (from
    /// [`crate::qemu::net::network_args`]). Spliced verbatim into the argv. The
    /// engine builds (and validates) these BEFORE spawn so an unavailable mode
    /// is rejected up front; `build_args` itself stays infallible.
    pub network: Vec<String>,
}

/// Build the full QEMU argument vector (everything after the binary name).
pub fn build_args(l: &QemuLaunch) -> Vec<String> {
    let mut a: Vec<String> = Vec::new();
    let mut push = |s: String| a.push(s);
    let mut flag = |k: &str, v: String| {
        push(k.to_string());
        push(v);
    };
    // QEMU option-list values are comma-delimited, so a literal comma in a path
    // is an option separator (injection). Escape by doubling commas — QEMU's
    // documented rule. Applied to every `file=` we interpolate.
    let esc = |p: &std::path::Path| p.display().to_string().replace(',', ",,");

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
            let cpu = if l.accel.is_hardware() {
                "host"
            } else {
                "qemu64"
            };
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

    // Boot disk (virtio-blk). `node-name=disk0` names the block node so live
    // QMP snapshot jobs can target it (vmstate/devices). `-snapshot` is never
    // emitted — that would discard writes on shutdown.
    flag(
        "-drive",
        format!(
            "file={},if=virtio,format=qcow2,node-name=disk0",
            esc(l.disk)
        ),
    );

    // Install media as a virtio CD-ROM. `-cdrom` defaults to if=ide, which the
    // aarch64 `virt` machine does not have — so attach explicitly via virtio.
    if let Some(iso) = l.iso {
        flag(
            "-drive",
            format!("file={},if=virtio,media=cdrom,format=raw", esc(iso)),
        );
        flag("-boot", "order=dc".to_string());
    }

    // Graphics + input so the VNC console renders output and accepts clicks.
    // aarch64 `virt` has no default display adapter; x86 q35 has built-in VGA.
    if l.guest_arch == "aarch64" {
        flag("-device", "virtio-gpu-pci".to_string());
    }
    flag("-device", "qemu-xhci,id=usb".to_string());
    flag("-device", "usb-kbd".to_string());
    flag("-device", "usb-tablet".to_string());

    // Display: built-in VNC server on loopback (noVNC bridge connects here).
    flag("-vnc", format!("127.0.0.1:{}", l.vnc_display));

    // QMP control channel.
    match &l.qmp {
        QmpEndpoint::UnixSocket(p) => {
            flag("-qmp", format!("unix:{},server=on,wait=off", p.display()))
        }
        QmpEndpoint::Tcp(port) => flag("-qmp", format!("tcp:127.0.0.1:{port},server=on,wait=off")),
    }

    // Networking: pre-built `-netdev`/`-device` fragments. The engine builds and
    // validates these via `qemu::net::network_args` BEFORE spawn (rejecting any
    // unavailable mode), so `build_args` just splices them in and stays
    // infallible. Port-forward binding + MAC handling live in that module.
    // Appended after the closures release their borrow of `a`.
    a.extend(l.network.iter().cloned());

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
            schema_version: 1,
            dir_slug: "test-vm".into(),
            hardware: Hardware {
                cpus: 4,
                memory_mib: 2048,
            },
            disks: vec![],
            network: NetworkConfig::default(),
            display: Default::default(),
            iso: None,
            metadata: Default::default(),
            snapshots: Vec::new(),
        }
    }

    fn find_flag<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
        args.iter()
            .position(|a| a == key)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    }

    #[test]
    fn drive_paths_with_commas_are_escaped() {
        // A comma in a path must be doubled so QEMU treats it as literal, not
        // an option separator (injection guard for the user-chosen ISO path).
        let mut c = cfg();
        c.iso = Some("/isos/weird,name.iso".into());
        let disk = PathBuf::from("/vm/di,sk.qcow2");
        let sock = PathBuf::from("/vm/qmp.sock");
        let args = build_args(&QemuLaunch {
            config: &c,
            accel: Accelerator::Hvf,
            guest_arch: "aarch64",
            disk: &disk,
            iso: Some(Path::new("/isos/weird,name.iso")),
            firmware: Some(Path::new("/fw/x.fd")),
            vnc_display: 1,
            qmp: QmpEndpoint::UnixSocket(&sock),
            network: vec![],
        });
        let joined = args.join(" ");
        assert!(
            joined.contains("file=/isos/weird,,name.iso,if=virtio,media=cdrom,format=raw"),
            "iso comma not escaped: {joined}"
        );
        assert!(
            joined.contains("file=/vm/di,,sk.qcow2,if=virtio,format=qcow2,node-name=disk0"),
            "disk comma not escaped or node-name missing: {joined}"
        );
    }

    #[test]
    fn boot_drive_has_node_name_disk0() {
        // Live QMP snapshot jobs target the named block node, so the boot
        // -drive MUST carry node-name=disk0.
        let c = cfg();
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
            network: vec![],
        });
        assert_eq!(
            find_flag(&args, "-drive"),
            Some("file=/vm/disk.qcow2,if=virtio,format=qcow2,node-name=disk0")
        );
    }

    #[test]
    fn build_args_never_emits_snapshot_flag() {
        // `-snapshot` makes QEMU write to a throwaway temp overlay, discarding
        // all disk writes on power-off. VMForge must NEVER emit it. Guard with
        // an ISO + port forwards present to exercise the broadest arg set.
        let mut c = cfg();
        c.iso = Some("/iso/alpine.iso".into());
        c.network.port_forwards = vec![PortForward {
            host: 2222,
            guest: 22,
            udp: false,
            expose_lan: false,
        }];
        let disk = PathBuf::from("/vm/disk.qcow2");
        let sock = PathBuf::from("/vm/qmp.sock");
        // Build real network fragments so the broadest arg set is exercised.
        let network =
            crate::qemu::net::network_args(&c.network, Accelerator::Hvf, "macos").unwrap();
        let args = build_args(&QemuLaunch {
            config: &c,
            accel: Accelerator::Hvf,
            guest_arch: "aarch64",
            disk: &disk,
            iso: Some(Path::new("/iso/alpine.iso")),
            firmware: Some(Path::new("/fw/x.fd")),
            vnc_display: 1,
            qmp: QmpEndpoint::UnixSocket(&sock),
            network,
        });
        assert!(
            !args.iter().any(|a| a == "-snapshot"),
            "build_args must never emit -snapshot: {args:?}"
        );
        // Audit: the hostfwd substring uses the loopback (127.0.0.1) form.
        let joined = args.join(" ");
        assert!(
            joined.contains("hostfwd=tcp:127.0.0.1:2222-:22"),
            "hostfwd must use the 127.0.0.1 loopback form: {joined}"
        );
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
            network: vec![],
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
            network: vec![],
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
            network: vec![],
        });
        let joined = args.join(" ");
        assert!(joined.contains("file=/iso/alpine.iso,if=virtio,media=cdrom,format=raw"));
        assert_eq!(find_flag(&args, "-boot"), Some("order=dc"));
    }

    #[test]
    fn port_forwards_render_hostfwd() {
        // Network fragments are now built by `qemu::net::network_args` and spliced
        // into the argv. Port forwards bind loopback (127.0.0.1) by default
        // (decision A1 — sanctioned migration from the old empty-host form).
        let mut c = cfg();
        c.network.port_forwards = vec![PortForward {
            host: 2222,
            guest: 22,
            udp: false,
            expose_lan: false,
        }];
        let disk = PathBuf::from("/vm/disk.qcow2");
        let sock = PathBuf::from("/vm/qmp.sock");
        let network =
            crate::qemu::net::network_args(&c.network, Accelerator::Hvf, "macos").unwrap();
        let args = build_args(&QemuLaunch {
            config: &c,
            accel: Accelerator::Hvf,
            guest_arch: "aarch64",
            disk: &disk,
            iso: None,
            firmware: Some(Path::new("/fw/x.fd")),
            vnc_display: 1,
            qmp: QmpEndpoint::UnixSocket(&sock),
            network,
        });
        assert_eq!(
            find_flag(&args, "-netdev"),
            Some("user,id=net0,hostfwd=tcp:127.0.0.1:2222-:22")
        );
    }
}
