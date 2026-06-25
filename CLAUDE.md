# CLAUDE.md — VMForge

VMForge is a cross-platform desktop app for creating, configuring, running,
snapshotting, and managing virtual machines through a GUI — in the spirit of
VMware Workstation Pro.

## What VMForge is (and is NOT)

- **NOT** a hypervisor, VMM, or device-emulation project. We do **not** build
  KVM. **QEMU is the engine** and does the actual virtualization.
- VMForge is a **management/orchestration layer over QEMU**: config
  management, process supervision, a great UI, and a guest console. Think
  *virt-manager / UTM / GNOME Boxes*, not "build a hypervisor."
- We drive QEMU through three surfaces:
  - **QMP** (QEMU Machine Protocol — JSON over a control socket) for lifecycle
    and runtime control.
  - **`qemu-img`** for disk image create / snapshot / clone.
  - **QEMU CLI args** to define virtual hardware at launch.

## Architecture

```
Frontend (React/TS/Tailwind/shadcn, in webview)
        │  Tauri IPC commands / events
Core (Rust, crates/vmforge-core)
   Hypervisor trait → QEMU impl (VZ impl optional later)
   QMP client · process supervisor · disk mgr · network mgr · config store
        │ spawns + controls            │ display
   QEMU process ──── VNC/SPICE ─────────┘
```

- **Shell:** Tauri 2 (`src-tauri/`) — thin; window + IPC wiring only.
- **Core:** `crates/vmforge-core` — all virtualization logic lives here.
- One **QEMU process per VM**, each with its own **QMP** channel.
- **Disks:** `qcow2` (`qemu-img create -f qcow2`); snapshots via QMP (live) and
  `qemu-img snapshot` (offline); linked clones via `qemu-img create -b`.
- **Console:** MVP = QEMU built-in **VNC** (`-vnc`) rendered with **noVNC**
  over a websockify bridge. SPICE upgrade later.
- **Networking:** MVP = user-mode **NAT** (`-netdev user`), zero privileges.
  Bridged / host-only designed now, implemented later behind an elevated-
  permissions UX.
- **Config layout:** each VM is a directory `~/VMForge/<vm-name>/` with
  `vmforge.toml` (hardware + metadata), qcow2 disk(s), runtime sockets/logs.
  A top-level library index tracks all VMs.

## Working agreements (non-negotiable)

1. **Engine boundary is sacred.** The frontend NEVER shells out to QEMU. All
   engine access goes through `vmforge-core`'s `Hypervisor` trait via Tauri
   IPC (`src-tauri/src/commands.rs`). This keeps the macOS-native VZ swap
   viable. The `Hypervisor` trait lives in
   `crates/vmforge-core/src/hypervisor.rs`.
2. **Every QMP / `qemu-img` interaction is wrapped and unit-testable** with the
   real binary mocked, so the suite runs without booting VMs in CI. Pure
   parsers (e.g. `host::parse_*`) are separated and tested directly.
3. **Phase gates.** No Phase N+1 work until Phase N's slice runs and its tests
   pass. `qa-engineer` owns the gate.
4. **Version honesty.** Pin and record actual installed versions (below). If
   unsure a QEMU flag/QMP command still exists in 11.x, verify before relying
   on it.
5. **Cross-platform paths always.** No POSIX-only paths, no `/tmp`
   assumptions. QMP/control transport is abstracted: Unix socket on
   macOS/Linux (`-qmp unix:<path>,server=on,wait=off`), TCP loopback / named
   pipe on Windows (`-qmp tcp:127.0.0.1:<port>,server=on,wait=off`).
6. **Small, reviewable commits**, conventional-commit messages, one feature
   slice per branch.
7. **Surface limitations in the UI, don't hide them** (e.g. "x86 guest on
   Apple Silicon — emulated, expect reduced performance"; "Bridged networking
   requires elevated permissions").

## Pinned versions (verified on this host, 2026-06-24)

| Tool | Version | Notes |
|---|---|---|
| QEMU | **11.0.1** | Homebrew; `hvf` + `tcg` accelerators present |
| Rust / Cargo | **1.94.0** | |
| Node | **25.8.0** | Homebrew (active `node`). See toolchain note below. |
| npm | **11.11.0** | Project package manager |
| Tauri CLI | **2.11.3** | `@tauri-apps/api` 2.11.1 |
| React | 19.1 | |
| Vite | 7.x | |
| TypeScript | 5.8 | |
| Tailwind | 4.x | + shadcn/ui (new-york / neutral), `@tailwindcss/vite` |
| Key crates | serde 1.0.228 · tokio 1.52 · uuid 1.23 · async-trait 0.1.89 · thiserror 2.0 | |

**Toolchain note:** the dev host has Node 25.8.0 (Homebrew) active and a
separate nvm Node 24.14.0 (where `pnpm` lives). The project standardizes on
**npm + the active Node**. Contributors: use Node ≥ 20.19.

## Host / platform reality (this machine)

- **Host:** macOS 26.5.1 (Darwin 25.5.0), Apple **M4**, 10 cores / 24 GiB,
  `arm64`. `kern.hv_support = 1` → **HVF available**.
- **Accelerated path here:** ARM64 guests under HVF. x86/x64 guests run under
  TCG (slow) — surface this in the UI.
- **All three OSes are first-class shipping targets** (per project decision).
  BUT this is an Apple-Silicon-only dev host: **Windows and Linux cannot be
  built or boot-tested locally here.** They are written as real code behind
  the platform abstraction and validated via the **CI matrix**
  (`build-engineer` + `qa-engineer` own a 3-OS matrix; a green build means all
  three pass, not just macOS). Never imply local macOS green == Win/Linux
  verified.

## Accelerator selection (per-host, runtime)

Picked at runtime by `vmforge_core::host::probe()` — never hardcoded:
macOS → `hvf`, Windows → `whpx`, Linux → `kvm`, else/unavailable → `tcg`
(with an honest performance warning). On Windows, WHPX availability interacts
with Hyper-V / WSL2 / VBS — detect, fall back to TCG, and explain why in the UI.

## Repo layout

```
VMForge/
  Cargo.toml                 # workspace root (members, shared deps, profiles)
  package.json               # frontend (npm)
  index.html · vite.config.ts · tsconfig.json · components.json
  src/                       # React frontend (frontend-engineer, console-engineer)
    App.tsx · main.tsx · index.css · lib/utils.ts
  src-tauri/                 # Tauri shell (thin)
    Cargo.toml · tauri.conf.json
    src/main.rs · src/lib.rs · src/commands.rs   # IPC surface
  crates/vmforge-core/       # the engine (Rust)
    src/lib.rs · error.rs · host.rs · model.rs · hypervisor.rs
    # Phase 1 adds: qmp/ process/ storage/ network/ + qemu impl
```

## Commands

| Action | Command |
|---|---|
| Install frontend deps | `npm install` |
| Run app (dev) | `npm run tauri dev` |
| Build app | `npm run tauri build` |
| Frontend typecheck | `npm run typecheck` |
| Core tests | `cargo test -p vmforge-core` |
| Whole workspace check | `cargo check --workspace` (needs `dist/`; run `npm run build` first) |
| Rust fmt / lint | `cargo fmt` · `cargo clippy --workspace` |

## Roadmap (thin vertical slices — prove each before expanding)

- **Phase 0 — Foundations.** ✅ Host probe, scaffold, workspace, `CLAUDE.md`,
  agent files, host-capability IPC + first-run UI.
- **Phase 1 — Vertical slice (proof of life).** ISO → launch VM (NAT, single
  qcow2, HVF/KVM accel) → interactive embedded VNC console → clean power-off
  via QMP. One VM, near-hardcoded config OK. **Do not proceed until this works
  on the host.**
- **Phase 2 — Library + persistence.** ✅ Multiple VMs, New-VM wizard,
  `vmforge.toml` + directory-scanned library, hardware editor (edit while
  stopped), live status via `list_vms` polling (events deferred to Phase 3).
  See [`docs/phase2-spec.md`](docs/phase2-spec.md).
- **Phase 3 — Snapshots & clones.** ✅ Snapshot tree (overlay in `vmforge.toml`),
  live (QMP `snapshot-save`/`-delete` jobs) + offline (`qemu-img`) snapshots,
  disk-only restore (live RAM-restore deferred), full + linked clones with
  parent-protection. See [`docs/phase3-spec.md`](docs/phase3-spec.md).
- **Phase 4 — Networking modes.** ✅ NAT port-forwarding (full, localhost-bound
  by default, `expose_lan` opt-in), per-VM MAC, and the bridged/host-only
  abstraction + capability detection + "needs elevated permissions" UX
  (privileged bring-up deferred — rejected at launch, never silent NAT
  fallback). See [`docs/phase4-spec.md`](docs/phase4-spec.md).
- **Phase 5 — Workstation niceties.** SPICE (clipboard, dynamic res, USB
  redirect), shared folders, drag-and-drop, snapshot-at-suspend.
- **Phase 6 — Distribution.** Signed packaged builds, first-run QEMU dependency
  handling, auto-update.

## Agent team (`.claude/agents/`)

The orchestrator (lead) owns planning, the roadmap, this file, integration,
and dispatch. Specialists own tight, non-overlapping mandates:

| Agent | Owns |
|---|---|
| `hypervisor-engineer` | `Hypervisor` trait, QEMU impl, QMP client, process supervisor, lifecycle state machine. The spine. |
| `storage-engineer` | `qemu-img` wrapper, qcow2 create/resize, snapshot tree, full/linked clones, VM dir layout + config (de)serialization. |
| `network-engineer` | NAT (MVP), bridged/host-only abstractions + privileged plumbing, port forwarding, per-VM MAC/adapter, "needs elevated permissions" UX contract. |
| `frontend-engineer` | React/TS/Tailwind/shadcn: library dashboard, New-VM wizard, hardware editor, layout. Consumes IPC; never touches QEMU. |
| `console-engineer` | Embedded guest display: noVNC + websockify/VNC bridge, input forwarding, resize; SPICE upgrade path. |
| `qa-engineer` | Test strategy/harness, Rust unit tests (mock QEMU + QMP), integration boot tests (tiny Alpine, headless), Vitest, CI gates. |
| `build-engineer` | Tauri packaging/signing, cross-platform build matrix, bundling/locating QEMU binaries, reproducible dev/build scripts. |

**Team-size decision:** the full 7-agent team is defined. For early Phase 1, the
orchestrator may temporarily fold `storage-engineer` + `network-engineer` into
`hypervisor-engineer`, and `console-engineer` into `frontend-engineer`, then
split back out at Phase 3. (Current decision: keep all 7 defined; dispatch only
those a slice needs.)

### Handoff rules

- **Shared contracts live in `vmforge-core`:** `error::{Error, Result}`,
  `model::*` (config + state), `host::*`, `hypervisor::Hypervisor`. Changing a
  shared type is a cross-agent change — flag it to the orchestrator.
- **IPC is the only frontend↔core path.** New capability = a `#[tauri::command]`
  in `src-tauri/src/commands.rs` delegating to the core, plus a typed wrapper
  on the frontend. Keep TS types in sync with the Rust structs.
- **Map errors at the boundary:** core returns `Result<T, vmforge_core::Error>`;
  commands return `Result<T, String>`.
- **No agent edits another agent's primary files** without coordinating. QA may
  add tests anywhere.
- **Every slice ships with tests** and updates this file if a convention
  changes.
