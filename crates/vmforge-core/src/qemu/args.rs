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

/// Platform firmware for the guest.
pub enum Firmware<'a> {
    /// aarch64 `virt`: a single UEFI code blob passed via `-bios`.
    Bios(&'a Path),
    /// x86_64 OVMF (UEFI): a read-only CODE blob + a writable per-VM VARS file,
    /// wired as two `-drive if=pflash` units. Required to boot Windows 11.
    Pflash { code: &'a Path, vars: &'a Path },
}

/// Everything needed to construct a launch command line.
pub struct QemuLaunch<'a> {
    pub config: &'a VmConfig,
    pub accel: Accelerator,
    /// `"aarch64"` or `"x86_64"`.
    pub guest_arch: &'a str,
    pub disk: &'a Path,
    pub iso: Option<&'a Path>,
    /// Guest firmware: `Bios` (aarch64 UEFI code blob) or `Pflash` (x86 OVMF).
    /// `None` means x86 SeaBIOS (built-in legacy BIOS — no firmware args).
    pub firmware: Option<Firmware<'a>>,
    /// VNC display number N → host port 5900 + N.
    pub vnc_display: u16,
    pub qmp: QmpEndpoint<'a>,
    /// Pre-built `-netdev`/`-device` network fragments (from
    /// [`crate::qemu::net::network_args`]). Spliced verbatim into the argv. The
    /// engine builds (and validates) these BEFORE spawn so an unavailable mode
    /// is rejected up front; `build_args` itself stays infallible.
    pub network: Vec<String>,
    /// When `true`, launch QEMU paused (`-S`) so a `snapshot-load` can run on
    /// the QMP channel before the guest CPUs start (suspend/resume). Cold starts
    /// pass `false` and `-S` is never emitted.
    pub prelaunch: bool,
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

    // Firmware. aarch64 `virt` has no built-in BIOS → `-bios <code>`. x86_64
    // OVMF (UEFI, required by Windows 11) is two pflash units: read-only CODE +
    // a writable per-VM VARS. `None` on x86 = built-in SeaBIOS (legacy boot).
    match &l.firmware {
        Some(Firmware::Bios(code)) => flag("-bios", code.display().to_string()),
        Some(Firmware::Pflash { code, vars }) => {
            flag(
                "-drive",
                format!("if=pflash,format=raw,unit=0,readonly=on,file={}", esc(code)),
            );
            flag(
                "-drive",
                format!("if=pflash,format=raw,unit=1,file={}", esc(vars)),
            );
        }
        None => {}
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

    // virtio-9p shared folders (decision A): a two-part fragment per folder —
    // `-fsdev local,id=fsdevN,path=<host>,security_model=mapped-xattr[,readonly=on]`
    // names the host export (privilege-free, perms preserved via xattrs), and
    // `-device virtio-9p-pci,fsdev=fsdevN,mount_tag=<tag>` attaches it under the
    // guest mount tag. `path=` is comma-escaped via `esc` (it lives inside an
    // option-list value); `mount_tag` is validated comma-free upstream. The
    // `local` form NEVER offers `passthrough` (needs root).
    for (i, sf) in l.config.shared_folders.iter().enumerate() {
        let ro = if sf.read_only { ",readonly=on" } else { "" };
        flag(
            "-fsdev",
            format!(
                "local,id=fsdev{i},path={},security_model=mapped-xattr{ro}",
                esc(std::path::Path::new(&sf.host_path))
            ),
        );
        flag(
            "-device",
            format!("virtio-9p-pci,fsdev=fsdev{i},mount_tag={}", sf.mount_tag),
        );
    }

    // Suspend/resume prelaunch: start paused so `snapshot-load` + `cont` can run
    // on QMP before the guest executes. Cold starts never emit `-S`.
    if l.prelaunch {
        push("-S".to_string());
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
            shared_folders: Vec::new(),
            guest_arch: None,
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
            firmware: Some(Firmware::Bios(Path::new("/fw/x.fd"))),
            vnc_display: 1,
            qmp: QmpEndpoint::UnixSocket(&sock),
            network: vec![],
            prelaunch: false,
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
            firmware: Some(Firmware::Bios(Path::new("/fw/x.fd"))),
            vnc_display: 1,
            qmp: QmpEndpoint::UnixSocket(&sock),
            network: vec![],
            prelaunch: false,
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
            firmware: Some(Firmware::Bios(Path::new("/fw/x.fd"))),
            vnc_display: 1,
            qmp: QmpEndpoint::UnixSocket(&sock),
            network,
            prelaunch: false,
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
            firmware: Some(Firmware::Bios(Path::new("/fw/edk2-aarch64-code.fd"))),
            vnc_display: 1,
            qmp: QmpEndpoint::UnixSocket(&sock),
            network: vec![],
            prelaunch: false,
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
            firmware: Some(Firmware::Bios(Path::new("/fw/x.fd"))),
            vnc_display: 0,
            qmp: QmpEndpoint::Tcp(4444),
            network: vec![],
            prelaunch: false,
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
            firmware: Some(Firmware::Bios(Path::new("/fw/x.fd"))),
            vnc_display: 2,
            qmp: QmpEndpoint::UnixSocket(&sock),
            network: vec![],
            prelaunch: false,
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
            firmware: Some(Firmware::Bios(Path::new("/fw/x.fd"))),
            vnc_display: 1,
            qmp: QmpEndpoint::UnixSocket(&sock),
            network,
            prelaunch: false,
        });
        assert_eq!(
            find_flag(&args, "-netdev"),
            Some("user,id=net0,hostfwd=tcp:127.0.0.1:2222-:22")
        );
    }

    // ---- Phase 5 — virtio-9p shared folders + prelaunch -S ----

    use crate::model::SharedFolder;

    /// Build args for a config with the given shared folders + prelaunch flag,
    /// returning the full argv (aarch64/HVF, no ISO).
    fn args_with(shared_folders: Vec<SharedFolder>, prelaunch: bool) -> Vec<String> {
        let mut c = cfg();
        c.shared_folders = shared_folders;
        let disk = PathBuf::from("/vm/disk.qcow2");
        let sock = PathBuf::from("/vm/qmp.sock");
        build_args(&QemuLaunch {
            config: &c,
            accel: Accelerator::Hvf,
            guest_arch: "aarch64",
            disk: &disk,
            iso: None,
            firmware: Some(Firmware::Bios(Path::new("/fw/x.fd"))),
            vnc_display: 1,
            qmp: QmpEndpoint::UnixSocket(&sock),
            network: vec![],
            prelaunch,
        })
    }

    #[test]
    fn shared_folder_emits_fsdev_and_device() {
        let args = args_with(
            vec![SharedFolder {
                host_path: "/home/user/share".into(),
                mount_tag: "share".into(),
                read_only: false,
            }],
            false,
        );
        let joined = args.join(" ");
        assert!(
            joined.contains(
                "-fsdev local,id=fsdev0,path=/home/user/share,security_model=mapped-xattr"
            ),
            "fsdev fragment missing/wrong: {joined}"
        );
        assert!(
            joined.contains("-device virtio-9p-pci,fsdev=fsdev0,mount_tag=share"),
            "9p device fragment missing/wrong: {joined}"
        );
        // No readonly suffix when read_only == false.
        assert!(
            !joined.contains("readonly=on"),
            "must not emit readonly=on for a writable share: {joined}"
        );
    }

    #[test]
    fn shared_folder_readonly_appends_flag() {
        let args = args_with(
            vec![SharedFolder {
                host_path: "/data".into(),
                mount_tag: "ro".into(),
                read_only: true,
            }],
            false,
        );
        let joined = args.join(" ");
        assert!(
            joined.contains(
                "-fsdev local,id=fsdev0,path=/data,security_model=mapped-xattr,readonly=on"
            ),
            "readonly fsdev fragment missing/wrong: {joined}"
        );
    }

    #[test]
    fn shared_folder_host_path_comma_escaped() {
        // A comma in the host path must be doubled so QEMU treats it as literal,
        // not an option separator (injection guard, decision A).
        let args = args_with(
            vec![SharedFolder {
                host_path: "/host/we,ird".into(),
                mount_tag: "tag".into(),
                read_only: false,
            }],
            false,
        );
        let joined = args.join(" ");
        assert!(
            joined.contains("path=/host/we,,ird,security_model=mapped-xattr"),
            "host_path comma not escaped: {joined}"
        );
    }

    #[test]
    fn multiple_shared_folders_index_fsdev_ids() {
        let args = args_with(
            vec![
                SharedFolder {
                    host_path: "/a".into(),
                    mount_tag: "first".into(),
                    read_only: false,
                },
                SharedFolder {
                    host_path: "/b".into(),
                    mount_tag: "second".into(),
                    read_only: true,
                },
            ],
            false,
        );
        let joined = args.join(" ");
        assert!(
            joined.contains("local,id=fsdev0,path=/a,security_model=mapped-xattr")
                && joined.contains("virtio-9p-pci,fsdev=fsdev0,mount_tag=first"),
            "folder 0 indexing wrong: {joined}"
        );
        assert!(
            joined.contains("local,id=fsdev1,path=/b,security_model=mapped-xattr,readonly=on")
                && joined.contains("virtio-9p-pci,fsdev=fsdev1,mount_tag=second"),
            "folder 1 indexing wrong: {joined}"
        );
    }

    #[test]
    fn prelaunch_appends_dash_s() {
        let args = args_with(vec![], true);
        assert!(
            args.iter().any(|a| a == "-S"),
            "prelaunch must emit -S: {args:?}"
        );
    }

    #[test]
    fn cold_start_never_emits_dash_s() {
        // A cold start (prelaunch == false) must NEVER pause the guest with -S,
        // even with shared folders + an ISO present (broadest arg set).
        let mut c = cfg();
        c.iso = Some("/iso/alpine.iso".into());
        c.shared_folders = vec![SharedFolder {
            host_path: "/share".into(),
            mount_tag: "share".into(),
            read_only: false,
        }];
        let disk = PathBuf::from("/vm/disk.qcow2");
        let sock = PathBuf::from("/vm/qmp.sock");
        let args = build_args(&QemuLaunch {
            config: &c,
            accel: Accelerator::Hvf,
            guest_arch: "aarch64",
            disk: &disk,
            iso: Some(Path::new("/iso/alpine.iso")),
            firmware: Some(Firmware::Bios(Path::new("/fw/x.fd"))),
            vnc_display: 1,
            qmp: QmpEndpoint::UnixSocket(&sock),
            network: vec![],
            prelaunch: false,
        });
        assert!(
            !args.iter().any(|a| a == "-S"),
            "cold start must never emit -S: {args:?}"
        );
    }

    // ---- Windows-readiness: x86_64 firmware + machine ----

    #[test]
    fn x86_pflash_emits_two_pflash_drives() {
        let c = cfg();
        let disk = PathBuf::from("/vm/disk.qcow2");
        let sock = PathBuf::from("/vm/qmp.sock");
        let code = PathBuf::from("/fw/edk2-x86_64-code.fd");
        let vars = PathBuf::from("/vm/OVMF_VARS.fd");
        let args = build_args(&QemuLaunch {
            config: &c,
            accel: Accelerator::Tcg,
            guest_arch: "x86_64",
            disk: &disk,
            iso: None,
            firmware: Some(Firmware::Pflash {
                code: &code,
                vars: &vars,
            }),
            vnc_display: 1,
            qmp: QmpEndpoint::UnixSocket(&sock),
            network: vec![],
            prelaunch: false,
        });
        let joined = args.join(" ");
        assert_eq!(find_flag(&args, "-machine"), Some("q35"));
        assert!(
            joined.contains("if=pflash,format=raw,unit=0,readonly=on,file=/fw/edk2-x86_64-code.fd"),
            "missing OVMF code pflash: {joined}"
        );
        assert!(
            joined.contains("if=pflash,format=raw,unit=1,file=/vm/OVMF_VARS.fd"),
            "missing OVMF vars pflash: {joined}"
        );
        // x86 OVMF must NOT use -bios, and q35 has built-in VGA (no virtio-gpu).
        assert!(
            find_flag(&args, "-bios").is_none(),
            "x86 OVMF must not use -bios: {joined}"
        );
        assert!(
            !joined.contains("virtio-gpu-pci"),
            "x86 must not add a virtio-gpu device: {joined}"
        );
    }

    #[test]
    fn x86_no_firmware_falls_back_to_seabios() {
        // No OVMF found → SeaBIOS: no -bios, no pflash. TCG x86 uses qemu64.
        let c = cfg();
        let disk = PathBuf::from("/vm/disk.qcow2");
        let sock = PathBuf::from("/vm/qmp.sock");
        let args = build_args(&QemuLaunch {
            config: &c,
            accel: Accelerator::Tcg,
            guest_arch: "x86_64",
            disk: &disk,
            iso: None,
            firmware: None,
            vnc_display: 1,
            qmp: QmpEndpoint::UnixSocket(&sock),
            network: vec![],
            prelaunch: false,
        });
        let joined = args.join(" ");
        assert_eq!(find_flag(&args, "-machine"), Some("q35"));
        assert!(find_flag(&args, "-bios").is_none());
        assert!(
            !joined.contains("if=pflash"),
            "SeaBIOS fallback must emit no pflash: {joined}"
        );
        assert_eq!(find_flag(&args, "-cpu"), Some("qemu64"));
    }
}
