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

Run the app with `npm run tauri dev`, then Browse to an ISO and **Create &
start**. See [`CLAUDE.md`](CLAUDE.md) for architecture, conventions, pinned
versions, the phased roadmap, and the agent team.

## License

MIT OR Apache-2.0
