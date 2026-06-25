# VMForge

A **Windows** desktop app for creating, configuring, running, snapshotting, and
managing **x86-64** virtual machines through a GUI — in the spirit of VMware
Workstation Pro.

VMForge is a **management/orchestration layer over QEMU**, not a hypervisor.
QEMU does the virtualization; VMForge handles config, process supervision, a
polished UI, and an embedded guest console.

> **Platform:** VMForge targets **Windows on x86-64** only. It uses **WHPX**
> (Windows Hypervisor Platform) for acceleration, falling back to **TCG**
> software emulation. The engine is portable Rust and can be built/unit-tested
> on a unix dev box, but Windows is the only supported and shipped platform. See
> [`docs/windows-only-spec.md`](docs/windows-only-spec.md).

## Stack

- **Tauri 2** (Rust) shell — thin: window + IPC only.
- **`vmforge-core`** (Rust crate) — the engine: `Hypervisor` trait, QEMU impl,
  QMP client (TCP loopback), process supervisor, disk/network/config managers.
- **React + TypeScript + Tailwind v4 + shadcn/ui** frontend.
- Drives **QEMU** via QMP (control), `qemu-img` (disks), and CLI args
  (q35 + OVMF/SeaBIOS). Console via VNC + noVNC.

## Prerequisites

- **Windows 10/11 (x86-64)** to run the app.
- **QEMU for Windows** — install from <https://qemu.weilnetz.de> (lands in
  `C:\Program Files\qemu`); VMForge auto-detects it (or use "Locate QEMU…").
  Enable the **Windows Hypervisor Platform** feature for hardware acceleration.
- **Rust** ≥ 1.94, **Node** ≥ 20.19, **npm** to build.

## Develop

```bash
npm install
npm run tauri dev            # run the app (on Windows)
cargo test -p vmforge-core   # core tests (run on a unix dev box; no VM needed)
npm run build                # frontend build
cargo build --workspace
```

## Status

- **Phase 0 (foundations) — complete.** Host probe, workspace scaffold,
  host-capability IPC + first-run UI.
- **Phase 1 (vertical slice) — functional & verified.** ISO → launch (HVF) →
  QMP lifecycle (running/pause/resume/shutdown/kill) → embedded noVNC console
  over a Rust VNC↔WebSocket bridge → power off. Verified by a gated integration
  test that boots Alpine aarch64 headless and drives it over QMP, plus the
  console RFB handshake through the bridge.
- **Phase 2 (library + persistence) — functional & verified.** VM library
  dashboard, New-VM wizard, hardware editor (edit while stopped), and
  `vmforge.toml` persistence (directory-scanned library, atomic writes,
  slug-addressed dirs). Live status via `list_vms` polling with a natural-exit
  reaper. Adversarially reviewed; 9 confirmed findings fixed (2 critical IPC
  arg-key bugs, a reaper race, path-traversal + `-drive` injection hardening).
- **Phase 3 (snapshots & clones) — functional & verified.** Snapshot tree
  (metadata overlay in `vmforge.toml`), **live** snapshots via QMP
  `snapshot-save`/`-delete` jobs and **offline** via `qemu-img`, disk-only
  restore, and **full + linked clones** with linked-parent protection. Verified
  by gated real-host tests (actual `qemu-img` snapshot/clone + a live QMP
  `snapshot-save` on a booted guest). Adversarially reviewed; 10 confirmed
  findings fixed (snapshot TOCTOU/lost-update, a multi-disk regression, clone
  path-traversal hardening).
- **Phase 4 (networking modes) — functional & verified.** NAT **port-forwarding**
  (host→guest, **bound to `127.0.0.1` by default**, `expose_lan` opt-in for LAN),
  per-VM **MAC** (validated, auto-generated), and the **bridged/host-only**
  abstraction with host **capability detection** + a "needs elevated permissions"
  UX — privileged bring-up deferred (rejected cleanly at launch, never a silent
  NAT fallback). Verified by a gated real-host test (QEMU actually binds the
  forwarded host port). Adversarially reviewed; 5 confirmed findings fixed
  (a high-severity editor-lockout for privileged-mode VMs, MAC arg-injection
  hardening, a port-cap validity inversion, TS↔Rust type-sync).
- **Phase 5 (workstation niceties, verifiable subset) — functional & verified.**
  virtio-9p **shared folders** (host dir → guest mount, localhost-safe
  validation) and **Suspend/Resume** (live snapshot save + `-S`/`snapshot-load`
  restore — this also lands Phase 3's deferred live restore). Suspend/Resume is
  **accelerator-gated**: refused on Apple-Silicon **HVF** (where QEMU's
  `snapshot-load` crashes — a verified ARM hardware-accel limitation), working
  under **TCG**. Verified by gated real-host tests (9p device accepted by QEMU;
  TCG suspend→resume round-trip). SPICE / USB redirect / drag-and-drop deferred
  (not feasible in the Tauri webview/sandbox). Adversarially reviewed; 5
  confirmed findings fixed (a missing `discard_suspend` IPC command, a
  resume-time vmstate leak, console nav-on-failure).
- **Phase 6 (distribution, verifiable subset + config) — functional & verified.**
  macOS `.app`/`.dmg` packaging; the **D3 PATH fix** — a Finder-launched `.app`
  inherits an empty `PATH` and would never find Homebrew QEMU, so QEMU is
  resolved to an absolute path for probe + firmware + spawn (with a "Locate
  QEMU…" override); a first-run **QEMU-required gate**; and signing/notarization
  + **auto-update** + a 3-OS release CI workflow written **as code but inert**
  (need your certs/keys/release host). Verified: `tauri build` produces a valid
  `VMForge.app`; gated real-host tests pass through the resolver. Adversarially
  reviewed; 5 confirmed findings fixed (the headline: `qemu-img` bypassed the
  resolver, breaking a Finder-launched app despite a passing gate). The macOS
  `.dmg` window-layout step needs a GUI session (CI / a real desktop), not this
  headless sandbox.

- **Windows readiness (engine verified; full app pending a Windows host).**
  Closed the gaps a Windows-readiness audit found so VMForge can boot an x86_64
  ISO — including a **UEFI Windows ISO**. New: **x86_64 OVMF firmware** discovery
  + per-VM writable NVRAM (`-drive if=pflash` unit 0/1), a **guest-architecture
  selector** (create-time; a foreign arch auto-downgrades to TCG and is flagged
  "emulated" in the UI), broadened **Windows QEMU resolution** (`.exe`,
  `%ProgramFiles%\qemu`, MSYS2/scoop/winget) + firmware discovery for the
  qemu.org installer layout, a **WHPX/Hyper-V/VBS** fallback warning, and QMP
  TCP bind-failure mapping. `vmforge-core` now **cross-compiles green to
  `x86_64-pc-windows-gnu`**, and a gated real-host test boots an **x86_64 guest
  under TCG with OVMF** end-to-end on Apple Silicon. Adversarially reviewed (9
  confirmed findings fixed). Still unverified on real Windows: the Tauri shell
  (WebView2 packaging), WHPX hardware accel, and a full Windows-guest boot —
  these need a Windows host / the CI matrix. See
  [`docs/windows-readiness-spec.md`](docs/windows-readiness-spec.md).

Run the app with `npm run tauri dev`, then Browse to an ISO and **Create &
start**. See [`CLAUDE.md`](CLAUDE.md) for architecture, conventions, pinned
versions, the phased roadmap, and the agent team.

## License

MIT OR Apache-2.0
