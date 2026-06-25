# VMForge Phase 4 — Networking (locked spec)

NAT port-forwarding (full + tested) + bridged/host-only **abstraction +
capability detection + elevated-permissions UX**; privileged bring-up deferred.
Sequential team: **core → IPC → frontend**.

## ORCHESTRATOR NOTES (authoritative)
- No overrides to the decisions below — accepted as-is.
- Impl note: `Accelerator` lives in **`crate::host`** (the B.2 sketch's
  `crate::accel` is a placeholder — use `crate::host::Accelerator`).

## A. DECISIONS LOCKED
- **A1 hostfwd bind default = `127.0.0.1`.** Emit
  `hostfwd={proto}:127.0.0.1:{host}-:{guest}`. LAN exposure is per-forward opt-in.
- **A2 bridged/host-only = abstraction + capability + UX only**; privileged
  bring-up deferred. Reasons must say *needs-permission / not-implemented*, never
  *unsupported* (vmnet is compiled in; blocker is entitlement/root).
- **A3 start with unavailable mode = REJECT** (no silent NAT fallback).
  `engine.start()` calls `net::network_args(...)?` BEFORE spawn; Bridged/HostOnly
  → `Error::Config(reason)`. Config still persists; only launch is refused.
- **A4 elevation error channel = reuse `Error::Config(String)`** (no new variant).
  Typed `NetworkBuildError` lives inside core, flattens to `Error::Config` at the
  engine boundary.
- **A5 port-forward/MAC editing = stopped, via `update_vm`** (already
  live-rejected); applies next launch.
- **A6 live `hostfwd_add`/`remove` = OUT** of Phase 4 (verified works via HMP but
  deferred). No `add/remove_port_forward` IPC commands. Keep the static formatter
  byte-compatible for a future Phase-5 reuse.
- **A7 no `set_network_mode` command** (mode is launch-time; `update_vm` owns it).
- **A8 MAC policy:** optional (blank → QEMU auto-assigns); validate strict shape
  `^([0-9A-Fa-f]{2}:){5}[0-9A-Fa-f]{2}$`; reject multicast (low bit of octet 0);
  generator uses `52:54:00:xx:xx:xx` (CSPRNG last 3 octets, lowercase);
  auto-generate on blank in the UI; store verbatim.
- **A9 `PortForward.expose_lan: bool`** (`#[serde(default)]`, additive). `true` →
  empty-host-addr form `hostfwd={proto}::{host}-:{guest}` (0.0.0.0).
- **A10 FIX editor/wizard data-wipe (blocking):** today both hardcode
  `network: { mode, mac:null, port_forwards:[] }`, and `update_vm` replaces (not
  merges) `network`, so saving silently destroys MAC + forwards. Editor/wizard
  drafts must carry the full `NetworkConfig` and round-trip it.
- **A11 capability shape:** `ModeCapability { mode: NetworkMode, available: bool,
  requires_elevation: bool, reason: String (empty when available) }` + aggregate
  `NetworkCapabilities { modes: Vec<ModeCapability>, port_forward_loopback_only:
  bool }`.

## B. CORE (crates/vmforge-core)
Files: `qemu/net.rs` (NEW), `qemu/mod.rs` (`pub mod net;`), `qemu/args.rs`
(delete inline netdev/device block, consume `QemuLaunch.network: Vec<String>`,
keep `build_args` infallible), `qemu/engine.rs` (pre-spawn `network_args?` +
busy-port log mapping), `host.rs` (capabilities), `model.rs` (`expose_lan`).

### model.rs (additive)
```rust
pub struct PortForward {
    pub host: u16,
    pub guest: u16,
    #[serde(default)] pub udp: bool,
    #[serde(default)] pub expose_lan: bool,  // default false = bind 127.0.0.1
}
```

### qemu/net.rs (pure; no I/O)
```rust
use crate::host::Accelerator;            // NOTE: host, not accel
use crate::model::{NetworkConfig, NetworkMode, PortForward};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NetworkBuildError {
    RequiresElevatedPermissions { mode: NetworkMode, reason: String },
    InvalidPortForward(String),
    InvalidMac(String),
}
// impl Display (reason verbatim / "invalid port forward: .." / mac msg) + std::error::Error

fn host_bind_addr(expose_lan: bool) -> &'static str { if expose_lan { "" } else { "127.0.0.1" } }

/// Build -netdev + -device fragments. Rejects bridged/host-only (typed error →
/// engine maps to Error::Config; NEVER silent NAT fallback). _accel reserved.
pub fn network_args(net: &NetworkConfig, _accel: Accelerator, host_os: &str)
    -> Result<Vec<String>, NetworkBuildError> {
    match net.mode {
        NetworkMode::User => user_mode_args(net),
        NetworkMode::Bridged | NetworkMode::HostOnly =>
            Err(NetworkBuildError::RequiresElevatedPermissions { mode: net.mode, reason: elevated_reason(net.mode, host_os) }),
    }
}
fn user_mode_args(net: &NetworkConfig) -> Result<Vec<String>, NetworkBuildError> {
    validate_port_forwards(&net.port_forwards).map_err(NetworkBuildError::InvalidPortForward)?;
    if let Some(mac) = &net.mac { validate_mac(mac).map_err(NetworkBuildError::InvalidMac)?; }
    let mut netdev = String::from("user,id=net0");
    for pf in &net.port_forwards {
        let proto = if pf.udp { "udp" } else { "tcp" };
        let bind = host_bind_addr(pf.expose_lan);
        netdev.push_str(&format!(",hostfwd={proto}:{bind}:{}-:{}", pf.host, pf.guest));
    }
    let device = match &net.mac {
        Some(mac) => format!("virtio-net-pci,netdev=net0,mac={mac}"),
        None => "virtio-net-pci,netdev=net0".to_string(),
    };
    Ok(vec!["-netdev".into(), netdev, "-device".into(), device])
}
/// Shared by launch-reject AND capability probe (so they never drift).
pub(crate) fn elevated_reason(mode: NetworkMode, host_os: &str) -> String { /* per-OS: macos vmnet/entitlement/root; linux TAP+CAP_NET_ADMIN; windows bridged adapter+Admin; "... not available in this build yet." */ }

// pure validators (pub):
pub fn validate_port_forwards(pfs: &[PortForward]) -> Result<(), String>;  // port 1..=65535 (reject 0); no dup (udp,host); tcp+udp may share host port
pub fn validate_mac(mac: &str) -> Result<(), String>;                       // 6 hex octets; reject multicast (octet0 & 0x01)
```
Verified emitted forms (QEMU 11.0.1): `-netdev user,id=net0,hostfwd=tcp:127.0.0.1:2222-:22`;
udp `hostfwd=udp:127.0.0.1:5353-:53`; `expose_lan` → `hostfwd=tcp::2222-:22`;
device `-device virtio-net-pci,netdev=net0,mac=52:54:00:12:34:56`.

### host.rs (capabilities)
```rust
pub struct ModeCapability { pub mode: NetworkMode, pub available: bool, pub requires_elevation: bool, pub reason: String }
pub struct NetworkCapabilities { pub modes: Vec<ModeCapability>, pub port_forward_loopback_only: bool }
pub fn probe_network(os: &str) -> NetworkCapabilities;  // user available; bridged/host-only unavailable+reason; loopback_only=true
// HostCapabilities gains: pub network: NetworkCapabilities  (additive; populate in probe())
```

### engine.rs
In `start()`, BEFORE `QemuProcess::spawn`:
```rust
let net_fragments = crate::qemu::net::network_args(&config.network, self.accel, std::env::consts::OS)
    .map_err(|e| Error::Config(e.to_string()))?;
```
Pass via `QemuLaunch.network`. In the early-exit branch, if the qemu.log tail
contains `Could not set up host forwarding rule` or `Bad host port`, map to
`Error::Config("Host port already in use or invalid; pick another or free it")`
instead of the generic QMP-timeout error.

### args.rs
Add `pub network: Vec<String>` to `QemuLaunch`; replace the inline netdev/device
block with `a.extend(l.network.iter().cloned());`. Keep `build_args` infallible.

## C. IPC (src-tauri)
```rust
#[tauri::command]
pub async fn network_capabilities() -> Result<NetworkCapabilities, String> {
    Ok(host::probe_network(std::env::consts::OS))
}
```
Register in `lib.rs`. `update_vm` unchanged (already carries `network`; DTOs
reflect `expose_lan` automatically once on the model). No `rename_all` (no
multi-word args). Tests: `network_capabilities_wire_shape` (keys
`["modes","port_forward_loopback_only"]`; `ModeCapability` keys
`["available","mode","reason","requires_elevation"]`; `mode` kebab `"host-only"`);
`port_forward_dto_carries_expose_lan`.

## D. FRONTEND (src/)
Files: `lib/ipc.ts` (EDIT: `ModeCapability`/`NetworkCapabilities` types +
`networkCapabilities()` wrapper; add `expose_lan` to `PortForward`);
`hooks/useNetworkCaps.ts` (NEW; module-level promise cache, probe once);
`lib/validation.ts` (EDIT: `validatePortNumber`, `validatePortForward(s)`,
`validateMac`, `generateMac`, `portForwardWarnings`, port constants);
`components/common/NetworkForm.tsx` (NEW, shared controlled),
`components/common/PortForwardRow.tsx` (NEW); REWRITE `editor/tabs/NetworkTab.tsx`
+ `wizard/steps/StepNetwork.tsx` to render `NetworkForm`; EDIT
`editor/HardwareEditorView.tsx` + `wizard/NewVmWizard.tsx` (draft `mode →
network: NetworkConfig`, round-trip mac+forwards — A10 fix, `networkValid` gate)
+ `wizard/steps/StepReview.tsx` (show MAC + forward count); DELETE
`components/common/NetworkModeField.tsx`. **No shadcn adds** (TCP/UDP = 2-option
Select; expose_lan = radio-group/button — all installed).

```ts
export interface PortForward { host: number; guest: number; udp: boolean; expose_lan: boolean }
export interface ModeCapability { mode: NetworkMode; available: boolean; requires_elevation: boolean; reason: string }
export interface NetworkCapabilities { modes: ModeCapability[]; port_forward_loopback_only: boolean }
export const networkCapabilities = () => invoke<NetworkCapabilities>("network_capabilities");
```
`NetworkForm` is fully controlled: `{ value: NetworkConfig; onChange; onValidityChange?;
disabled?; variant?: "editor"|"wizard"; idPrefix? }`. Mode select: User enabled;
Bridged/Host-only disabled with the capability `reason` + "requires elevated
permissions"; a legacy bridged value is shown selected with an amber reason and
NEVER auto-rewritten. MAC input + Generate/Clear + inline validate. Port-forward
section meaningful only in user mode (disabled+explained otherwise); rows via
`PortForwardRow` (host, guest, TCP/UDP, expose_lan toggle, remove); inline
range/dup errors; soft `<1024` warning (non-blocking); footer "applies next
launch"; security line driven by `port_forward_loopback_only`. Validation: range
1–65535 integer; duplicate `(host, udp)` flagged on 2nd+ row; guest dups allowed;
count cap (`MAX_PORT_FORWARDS=32`); MAC optional/shape/multicast.

## E. TEST PLAN
- **net.rs unit:** loopback-by-default; expose_lan→bind-all; udp+mac; zero
  forwards → bare `user,id=net0` (regression guard); bridged/host-only rejected
  with per-OS reason (panic if Ok — no NAT fallback); reject zero port; reject
  dup same-proto host port; allow tcp+udp same host port; reject bad/multicast
  MAC; MAC comma-injection rejected.
- **host.rs unit:** `network_caps_phase4_shape` (user available; others
  unavailable+requires_elevation+reason; bridged reason contains "vmnet" on
  macos; `port_forward_loopback_only==true`).
- **commands.rs:** `network_capabilities_wire_shape`, `port_forward_dto_carries_expose_lan`.
- **args.rs (intentional break):** update `port_forwards_render_hostfwd` to the
  `tcp:127.0.0.1:2222-:22` form; audit `build_args_never_emits_snapshot_flag`
  hostfwd substring; assert `127.0.0.1` present.
- **GATED E2E (`tests/boot.rs`, VMFORGE_ISO):** boot Alpine with a forward to a
  free high host port → guest; since stock Alpine has no sshd, drive the guest
  serial via QMP to `nc -l -p <guest>` then assert a host `127.0.0.1:<host>`
  connect echoes. Pick a free high port at runtime. Document prereqs; keep gated.
  **Offline interim (ships, deterministic):** pre-bind a port, launch a throwaway
  process echoing QEMU's `Could not set up host forwarding rule` stderr, assert
  `start()`'s log-mapping yields `Error::Config(host port in use)`.
- **Vitest:** `validation.test.ts`, `NetworkForm.test.tsx` (disabled options +
  reasons; legacy bridged not auto-rewritten; validity gating; security copy),
  `PortForwardRow.test.tsx`.

## F. SEQUENCING & ACCEPTANCE
Stages: (1) CORE (model → net.rs+tests → mod → host.rs+tests → args.rs[update
the one hostfwd test] → engine.rs); (2) IPC (`network_capabilities` + tests +
register); (3) FRONTEND (ipc.ts → validation+vitest → useNetworkCaps →
NetworkForm/PortForwardRow+vitest → rewrite NetworkTab/StepNetwork → edit
HardwareEditorView/NewVmWizard/StepReview [A10] → delete NetworkModeField).

Green-gate at each stage boundary:
```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --workspace
npm run typecheck
npm run build
npm test
```

Must NOT break Phase 1–3: the ONLY sanctioned test change is `args.rs`
`port_forwards_render_hostfwd` → `127.0.0.1` form (migrate any other old-form
assertion in the same commit). Gated boot/snapshots tests stay green
(`NetworkConfig::default()` = zero forwards → bare `user,id=net0`; RFB
proof-of-life unaffected). Mode-reject fires ONLY for Bridged/HostOnly; all
User-mode VMs boot unchanged. All `NetworkConfig`/`PortForward` fields
`#[serde(default)]` (back-compat). `HostCapabilities` gains `network` additively
— update its TS interface + any `*_wire_shape` test in the same commit.
