# Windows-only — implementation spec

Refocus VMForge to a single supported platform: **Windows on x86-64**. Remove all
macOS/Linux-specific code, the aarch64-guest path, and the cross-platform
accelerator/transport machinery. Keep the engine written in portable Rust so the
unit suite still runs on the macOS dev box and can be cross-compile-checked for
Windows; the only *supported/shipped* target is Windows.

## Decisions

- **D1 — Guest arch: x86-64 only.** Drop the aarch64 `virt` machine, the aarch64
  UEFI `-bios` path, the ARM CPU model, and the guest-architecture selector
  (model field, IPC DTOs, wizard step, library badge). Every VM is q35/x86-64.
- **D2 — Accelerators: WHPX + TCG only.** Remove `Hvf` and `Kvm`. `pick_preferred`
  = WHPX if QEMU advertises it, else TCG. `emulated` now means "running under
  TCG" (no hardware accel), surfaced honestly in the UI.
- **D3 — QMP transport: TCP loopback only.** Remove the Unix-socket path
  (`connect_unix`, `QmpBind::Unix`, `runtime_dir`/`qmp_socket_path`, the
  per-VM socket field). Always `-qmp tcp:127.0.0.1:<port>`. (tokio TCP is
  portable, so this still runs on the Mac dev box.)
- **D4 — Machine/firmware: q35 + OVMF, SeaBIOS fallback.** Keep `find_x86_64_uefi`
  (bin-relative `share` search — finds OVMF on both the qemu.org Windows installer
  AND Homebrew on the Mac dev box, so no hardcoded unix dirs needed). Drop
  `find_aarch64_uefi`. Firmware = `Pflash` (OVMF) or `None` (SeaBIOS).
- **D5 — Removal is Windows-only-in-spirit, portable-in-practice.** No
  macOS/Linux-*specific* code remains (HVF/KVM, Unix sockets, vmnet, /tmp & XDG,
  Homebrew/Finder, entitlements, dmg/deb/appimage, mac+linux CI). Portable
  primitives (tokio TCP, std fs, `directories`) stay, so the test suite runs on
  the Mac dev box. Real VM-boot verification happens on the user's Windows PC.
- **D6 — CI/packaging Windows-only.** `ci.yml` → a single `windows-latest` job
  (fmt + clippy + test). `release.yml` → a single `windows-latest` job
  (`nsis,updater`). `bundle.targets` → NSIS; remove macOS/linux bundle config +
  `entitlements.plist`. The updater pipeline (keypair, latest.json) is unchanged.
- **D7 — Device model unchanged this pass.** Boot disk stays virtio-blk, ISO
  stays virtio-cdrom (consistent with existing snapshot `node-name=disk0`
  targeting and the gated tests). NOTE for a follow-up: Windows *guest* installers
  need virtio drivers (virtio-win) or a SATA/IDE device model to see the disk/CD
  out of the box — tracked separately, not part of "remove macOS/Linux".
- **D8 — Suspend/resume stays TCG-gated** (refused under hardware accel) pending
  verification that QMP `snapshot-load` is stable under WHPX on a real Windows
  host — same safety principle as the old HVF gate, generalized.

## Verification

- `cargo test -p vmforge-core` runs green on the macOS dev box (portable code).
- `cargo check -p vmforge-core --target x86_64-pc-windows-gnu --tests` green.
- Frontend typecheck + vitest green; shell wire-shape tests updated.
- Gated `x86_guest` real-host test still boots an x86-64 guest (now the primary
  boot test). aarch64-specific gated tests retired/converted to x86-64.
- Windows CI job is the authoritative gate; full VM-boot testing on the user's PC.
