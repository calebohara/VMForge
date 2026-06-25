# VMForge

A cross-platform desktop app for creating, configuring, running, snapshotting,
and managing virtual machines through a GUI — in the spirit of VMware
Workstation Pro.

VMForge is a **management/orchestration layer over QEMU**, not a hypervisor.
QEMU does the virtualization; VMForge handles config, process supervision, a
polished UI, and an embedded guest console. Think *virt-manager / UTM / GNOME
Boxes*.

## Stack

- **Tauri 2** (Rust) shell — thin: window + IPC only.
- **`vmforge-core`** (Rust crate) — the engine: `Hypervisor` trait, QEMU impl,
  QMP client, process supervisor, disk/network/config managers.
- **React + TypeScript + Tailwind v4 + shadcn/ui** frontend.
- Drives **QEMU** via QMP (control), `qemu-img` (disks), and CLI args
  (hardware). Console via VNC + noVNC (SPICE later).

## Prerequisites

- **QEMU** (`qemu-system-*`, `qemu-img`) — e.g. `brew install qemu`.
- **Rust** ≥ 1.94, **Node** ≥ 20.19, **npm**.
- macOS: Xcode Command Line Tools.

## Develop

```bash
npm install
npm run tauri dev            # run the app
cargo test -p vmforge-core   # core tests (no VM needed)
npm run build                # frontend build
cargo check --workspace
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

Run the app with `npm run tauri dev`, then Browse to an ISO and **Create &
start**. See [`CLAUDE.md`](CLAUDE.md) for architecture, conventions, pinned
versions, the phased roadmap, and the agent team.

## License

MIT OR Apache-2.0
