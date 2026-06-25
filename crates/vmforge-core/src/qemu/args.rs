//! Pure QEMU argv construction. No process spawning here — that keeps the
//! whole arg surface unit-testable without QEMU installed.
//!
//! Owned by hypervisor-engineer (network fragments coordinated with
//! network-engineer).

use crate::host::Accelerator;
use crate::model::VmConfig;
use std::path::Path;

/// The QMP control channel: a TCP loopback port (`-qmp tcp:127.0.0.1:<port>`).
pub struct QmpEndpoint(pub u16);

/// x86-64 OVMF (UEFI) firmware: a read-only CODE blob + a writable per-VM VARS
/// file, wired as two `-drive if=pflash` units. Required to boot Windows. When
/// [`QemuLaunch::firmware`] is `None`, the guest uses the built-in SeaBIOS.
pub struct Firmware<'a> {
    pub code: &'a Path,
    pub vars: &'a Path,
}

/// Everything needed to construct a launch command line.
pub struct QemuLaunch<'a> {
    pub config: &'a VmConfig,
    pub accel: Accelerator,
    pub disk: &'a Path,
    pub iso: Option<&'a Path>,
    /// x86-64 OVMF firmware, or `None` for the built-in SeaBIOS.
    pub firmware: Option<Firmware<'a>>,
    /// VNC display number N → host port 5900 + N.
    pub vnc_display: u16,
    pub qmp: QmpEndpoint,
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

    // x86-64 q35 machine. `-cpu host` needs hardware accel (WHPX); TCG needs a
    // concrete model.
    flag("-machine", "q35".to_string());
    let cpu = if l.accel.is_hardware() {
        "host"
    } else {
        "qemu64"
    };
    flag("-cpu", cpu.to_string());

    flag("-accel", l.accel.as_qemu_arg().to_string());
    flag("-smp", l.config.hardware.cpus.to_string());
    flag("-m", l.config.hardware.memory_mib.to_string());

    // Firmware: x86-64 OVMF (UEFI) is two pflash units — read-only CODE + a
    // writable per-VM VARS. `None` = the built-in SeaBIOS (legacy BIOS boot).
    if let Some(fw) = &l.firmware {
        flag(
            "-drive",
            format!(
                "if=pflash,format=raw,unit=0,readonly=on,file={}",
                esc(fw.code)
            ),
        );
        flag(
            "-drive",
            format!("if=pflash,format=raw,unit=1,file={}", esc(fw.vars)),
        );
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

    // Install media as a virtio CD-ROM.
    if let Some(iso) = l.iso {
        flag(
            "-drive",
            format!("file={},if=virtio,media=cdrom,format=raw", esc(iso)),
        );
        flag("-boot", "order=dc".to_string());
    }

    // Input devices for the VNC console (q35 has a built-in VGA adapter, so no
    // explicit display device is needed).
    flag("-device", "qemu-xhci,id=usb".to_string());
    flag("-device", "usb-kbd".to_string());
    flag("-device", "usb-tablet".to_string());

    // Display: built-in VNC server on loopback (noVNC bridge connects here).
    flag("-vnc", format!("127.0.0.1:{}", l.vnc_display));

    // QMP control channel: TCP loopback.
    flag(
        "-qmp",
        format!("tcp:127.0.0.1:{},server=on,wait=off", l.qmp.0),
    );

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
    use crate::model::{Hardware, NetworkConfig, PortForward, SharedFolder, VmConfig};
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
        }
    }

    /// Build with a fixed VNC display (1) and QMP port (4444); disk defaults to
    /// `/vm/disk.qcow2` unless overridden.
    fn build(
        c: &VmConfig,
        accel: Accelerator,
        disk: &Path,
        iso: Option<&Path>,
        firmware: Option<Firmware>,
        network: Vec<String>,
        prelaunch: bool,
    ) -> Vec<String> {
        build_args(&QemuLaunch {
            config: c,
            accel,
            disk,
            iso,
            firmware,
            vnc_display: 1,
            qmp: QmpEndpoint(4444),
            network,
            prelaunch,
        })
    }

    fn find_flag<'a>(args: &'a [String], key: &str) -> Option<&'a str> {
        args.iter()
            .position(|a| a == key)
            .and_then(|i| args.get(i + 1))
            .map(String::as_str)
    }

    #[test]
    fn drive_paths_with_commas_are_escaped() {
        // A comma in a path must be doubled so QEMU treats it as literal, not an
        // option separator (injection guard for the user-chosen ISO path).
        let mut c = cfg();
        c.iso = Some("/isos/weird,name.iso".into());
        let disk = PathBuf::from("/vm/di,sk.qcow2");
        let args = build(
            &c,
            Accelerator::Whpx,
            &disk,
            Some(Path::new("/isos/weird,name.iso")),
            None,
            vec![],
            false,
        );
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
        // -drive MUST carry node-name=disk0. With SeaBIOS (no pflash) the boot
        // disk is the first -drive.
        let disk = PathBuf::from("/vm/disk.qcow2");
        let args = build(&cfg(), Accelerator::Whpx, &disk, None, None, vec![], false);
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
        c.iso = Some("/iso/win.iso".into());
        c.network.port_forwards = vec![PortForward {
            host: 2222,
            guest: 22,
            udp: false,
            expose_lan: false,
        }];
        let network = crate::qemu::net::network_args(&c.network, Accelerator::Whpx).unwrap();
        let disk = PathBuf::from("/vm/disk.qcow2");
        let args = build(
            &c,
            Accelerator::Whpx,
            &disk,
            Some(Path::new("/iso/win.iso")),
            None,
            network,
            false,
        );
        assert!(
            !args.iter().any(|a| a == "-snapshot"),
            "build_args must never emit -snapshot: {args:?}"
        );
        let joined = args.join(" ");
        assert!(
            joined.contains("hostfwd=tcp:127.0.0.1:2222-:22"),
            "hostfwd must use the 127.0.0.1 loopback form: {joined}"
        );
    }

    #[test]
    fn q35_whpx_uses_host_cpu_and_tcp_qmp() {
        let disk = PathBuf::from("/vm/disk.qcow2");
        let args = build(&cfg(), Accelerator::Whpx, &disk, None, None, vec![], false);
        assert_eq!(find_flag(&args, "-machine"), Some("q35"));
        assert_eq!(find_flag(&args, "-accel"), Some("whpx"));
        assert_eq!(find_flag(&args, "-cpu"), Some("host"));
        assert_eq!(find_flag(&args, "-smp"), Some("4"));
        assert_eq!(find_flag(&args, "-m"), Some("2048"));
        assert_eq!(find_flag(&args, "-vnc"), Some("127.0.0.1:1"));
        assert_eq!(
            find_flag(&args, "-qmp"),
            Some("tcp:127.0.0.1:4444,server=on,wait=off")
        );
    }

    #[test]
    fn tcg_uses_qemu64_cpu() {
        let disk = PathBuf::from("/vm/disk.qcow2");
        let args = build(&cfg(), Accelerator::Tcg, &disk, None, None, vec![], false);
        assert_eq!(find_flag(&args, "-cpu"), Some("qemu64"));
        assert_eq!(find_flag(&args, "-accel"), Some("tcg"));
    }

    #[test]
    fn iso_attaches_virtio_cdrom() {
        let mut c = cfg();
        c.iso = Some("/iso/win.iso".into());
        let disk = PathBuf::from("/vm/disk.qcow2");
        let args = build(
            &c,
            Accelerator::Whpx,
            &disk,
            Some(Path::new("/iso/win.iso")),
            None,
            vec![],
            false,
        );
        let joined = args.join(" ");
        assert!(joined.contains("file=/iso/win.iso,if=virtio,media=cdrom,format=raw"));
        assert_eq!(find_flag(&args, "-boot"), Some("order=dc"));
    }

    #[test]
    fn port_forwards_render_hostfwd() {
        // Network fragments are built by `qemu::net::network_args` and spliced
        // in. Forwards bind loopback (127.0.0.1) by default.
        let mut c = cfg();
        c.network.port_forwards = vec![PortForward {
            host: 2222,
            guest: 22,
            udp: false,
            expose_lan: false,
        }];
        let network = crate::qemu::net::network_args(&c.network, Accelerator::Whpx).unwrap();
        let disk = PathBuf::from("/vm/disk.qcow2");
        let args = build(&c, Accelerator::Whpx, &disk, None, None, network, false);
        assert_eq!(
            find_flag(&args, "-netdev"),
            Some("user,id=net0,hostfwd=tcp:127.0.0.1:2222-:22")
        );
    }

    // ---- x86-64 firmware (OVMF / SeaBIOS) ----

    #[test]
    fn ovmf_emits_two_pflash_drives_no_bios() {
        let c = cfg();
        let disk = PathBuf::from("/vm/disk.qcow2");
        let code = PathBuf::from("/fw/edk2-x86_64-code.fd");
        let vars = PathBuf::from("/vm/OVMF_VARS.fd");
        let args = build(
            &c,
            Accelerator::Whpx,
            &disk,
            None,
            Some(Firmware {
                code: &code,
                vars: &vars,
            }),
            vec![],
            false,
        );
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
        // q35 has built-in VGA — no virtio-gpu device.
        assert!(
            !joined.contains("virtio-gpu-pci"),
            "x86 must not add a virtio-gpu device: {joined}"
        );
    }

    #[test]
    fn no_firmware_falls_back_to_seabios() {
        // No OVMF → SeaBIOS: no pflash. TCG x86 uses qemu64.
        let disk = PathBuf::from("/vm/disk.qcow2");
        let args = build(&cfg(), Accelerator::Tcg, &disk, None, None, vec![], false);
        let joined = args.join(" ");
        assert_eq!(find_flag(&args, "-machine"), Some("q35"));
        assert!(
            !joined.contains("if=pflash"),
            "SeaBIOS fallback must emit no pflash: {joined}"
        );
        assert_eq!(find_flag(&args, "-cpu"), Some("qemu64"));
    }

    // ---- virtio-9p shared folders + prelaunch -S ----

    /// Build args for a config with the given shared folders + prelaunch flag.
    fn args_with(shared_folders: Vec<SharedFolder>, prelaunch: bool) -> Vec<String> {
        let mut c = cfg();
        c.shared_folders = shared_folders;
        let disk = PathBuf::from("/vm/disk.qcow2");
        build(&c, Accelerator::Whpx, &disk, None, None, vec![], prelaunch)
    }

    #[test]
    fn shared_folder_emits_fsdev_and_device() {
        let args = args_with(
            vec![SharedFolder {
                host_path: "C:/Users/me/share".into(),
                mount_tag: "share".into(),
                read_only: false,
            }],
            false,
        );
        let joined = args.join(" ");
        assert!(
            joined.contains(
                "-fsdev local,id=fsdev0,path=C:/Users/me/share,security_model=mapped-xattr"
            ),
            "fsdev fragment missing/wrong: {joined}"
        );
        assert!(
            joined.contains("-device virtio-9p-pci,fsdev=fsdev0,mount_tag=share"),
            "9p device fragment missing/wrong: {joined}"
        );
        assert!(
            !joined.contains("readonly=on"),
            "must not emit readonly=on for a writable share: {joined}"
        );
    }

    #[test]
    fn shared_folder_readonly_appends_flag() {
        let args = args_with(
            vec![SharedFolder {
                host_path: "D:/data".into(),
                mount_tag: "ro".into(),
                read_only: true,
            }],
            false,
        );
        let joined = args.join(" ");
        assert!(
            joined.contains(
                "-fsdev local,id=fsdev0,path=D:/data,security_model=mapped-xattr,readonly=on"
            ),
            "readonly fsdev fragment missing/wrong: {joined}"
        );
    }

    #[test]
    fn shared_folder_host_path_comma_escaped() {
        let args = args_with(
            vec![SharedFolder {
                host_path: "C:/we,ird".into(),
                mount_tag: "tag".into(),
                read_only: false,
            }],
            false,
        );
        let joined = args.join(" ");
        assert!(
            joined.contains("path=C:/we,,ird,security_model=mapped-xattr"),
            "host_path comma not escaped: {joined}"
        );
    }

    #[test]
    fn multiple_shared_folders_index_fsdev_ids() {
        let args = args_with(
            vec![
                SharedFolder {
                    host_path: "C:/a".into(),
                    mount_tag: "first".into(),
                    read_only: false,
                },
                SharedFolder {
                    host_path: "C:/b".into(),
                    mount_tag: "second".into(),
                    read_only: true,
                },
            ],
            false,
        );
        let joined = args.join(" ");
        assert!(
            joined.contains("local,id=fsdev0,path=C:/a,security_model=mapped-xattr")
                && joined.contains("virtio-9p-pci,fsdev=fsdev0,mount_tag=first"),
            "folder 0 indexing wrong: {joined}"
        );
        assert!(
            joined.contains("local,id=fsdev1,path=C:/b,security_model=mapped-xattr,readonly=on")
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
        let mut c = cfg();
        c.iso = Some("/iso/win.iso".into());
        c.shared_folders = vec![SharedFolder {
            host_path: "C:/share".into(),
            mount_tag: "share".into(),
            read_only: false,
        }];
        let disk = PathBuf::from("/vm/disk.qcow2");
        let args = build(
            &c,
            Accelerator::Whpx,
            &disk,
            Some(Path::new("/iso/win.iso")),
            None,
            vec![],
            false,
        );
        assert!(
            !args.iter().any(|a| a == "-S"),
            "cold start must never emit -S: {args:?}"
        );
    }
}
