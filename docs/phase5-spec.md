# VMForge Phase 5 — 9p Shared Folders + Suspend/Resume (locked spec)

Verifiable subset only: virtio-9p shared folders + Suspend/Resume (live
snapshot save+restore — lands Phase 3's deferred live restore). SPICE/USB/
drag-drop deferred. Sequential team: **CORE → IPC → FRONTEND**.

## ORCHESTRATOR NOTE (accepted, no overrides)
- **Key constraint (host-verified):** QMP `snapshot-load` CRASHES under HVF on
  aarch64 (`cpu_pre_load` assertion); it only works under TCG. So suspend/resume
  is **accelerator-gated**: `suspend` refuses up front on `aarch64 + HVF` with a
  clear message, so an unresumable vmstate is never captured. On this host the
  verifiable surface is the refusal + offline tests; the live round-trip is
  verified under a **TCG-gated** test. 9p shared folders are fully verifiable.

## A. DECISIONS LOCKED
- **9p form:** two-part `-fsdev local,id=fsdevN,path=<host>,security_model=mapped-xattr[,readonly=on]`
  + `-device virtio-9p-pci,fsdev=fsdevN,mount_tag=<tag>` (NOT `-virtfs` — keeps
  the user `path=` inside an option value so `esc()` comma-doubling protects it).
- **security_model = `mapped-xattr`** (privilege-free, preserves perms via
  xattrs). `passthrough` NEVER offered (needs root). read-only → `,readonly=on`.
- **Validation (two validators, NOT routed through is_safe_slug):**
  `is_safe_mount_tag` (non-empty, ≤31 bytes, no leading `-`, charset
  `[A-Za-z0-9._-]` — comma-free so NOT esc'd); `is_safe_share_path` (non-empty,
  absolute, no leading `-`; commas OK via esc); `config_shares_safe` (lexical +
  unique tags) at the `load_all` chokepoint (fail-closed, like
  `config_disks_safe`); `validate_shared_folders` (lexical + each host_path is an
  existing dir) in the engine BEFORE spawn → `Error::Config`.
- **Suspend state = `VmMetadata.suspended_snapshot: Option<Uuid>`, NOT a new
  `VmState` variant** (avoids rippling the locked wire contract — json_wire_casing,
  wire-shape tests, map_status, every `matches!(Stopped|Defined)` guard). A
  suspended VM is `Stopped` on the wire; "suspended-ness" is a **derived
  `suspended: bool`** on the DTOs.
- **Suspend tag:** fresh `Uuid::new_v4()` per suspend; qcow2 vmstate tag = that
  uuid string; stored in `metadata.suspended_snapshot`; **excluded from
  `config.snapshots[]`** (never in the tree). Singleton. QMP job-id is a separate
  ephemeral `vmforge-{uuid}`.
- **Resume = `-S` + `snapshot-load` + `cont`** via `start_inner(config,
  prelaunch_load: Option<Uuid>)`; runs on the local QmpClient before registry
  insert (lock discipline preserved). Accelerator-gated (above).
- **Edit-while-suspended REFUSED** (`update_config` guard: if
  `suspended_snapshot.is_some()` → `Error::Config`). Escape hatch
  `discard_suspend` (clear field + `snapshot_delete_offline`).
- **Invalidation:** tag missing at resume → clear field + refuse ("reset to
  stopped"); plain `start()` on suspended → refused; `delete` allowed (clears);
  clone never carries the field; successful resume consumes it.

## B. CORE (crates/vmforge-core)
Files: model.rs, qemu/args.rs, library.rs, qemu/engine.rs, hypervisor.rs.
qmp.rs/storage.rs unchanged. Fix every `VmConfig{…}`/`VmMetadata{…}`/
`QemuLaunch{…}` test literal for the new fields.

### model.rs
```rust
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SharedFolder {
    pub host_path: String,   // absolute host dir; must exist
    pub mount_tag: String,   // 9p tag
    #[serde(default)] pub read_only: bool,
}
// VmConfig: add  #[serde(default)] pub shared_folders: Vec<SharedFolder>,
// VmMetadata: add #[serde(default)] pub suspended_snapshot: Option<uuid::Uuid>,
//   (VmMetadata derives Default; field-by-field literals in library.rs full_clone/
//    linked_clone must add suspended_snapshot: None)
```

### qemu/args.rs
Add `pub prelaunch: bool` to `QemuLaunch`. In `build_args`, after the network
splice (where `flag`/`push` closures are released):
```rust
for (i, sf) in l.config.shared_folders.iter().enumerate() {
    let ro = if sf.read_only { ",readonly=on" } else { "" };
    flag("-fsdev", format!(
        "local,id=fsdev{i},path={},security_model=mapped-xattr{ro}",
        esc(std::path::Path::new(&sf.host_path))));
    flag("-device", format!("virtio-9p-pci,fsdev=fsdev{i},mount_tag={}", sf.mount_tag));
}
if l.prelaunch { push("-S".to_string()); }
```
Update all `QemuLaunch{…}` literals to add `prelaunch: false`. `build_args` stays
infallible.

### library.rs
```rust
pub fn is_safe_mount_tag(t: &str) -> bool {
    !t.is_empty() && t.len() <= 31 && !t.starts_with('-')
        && t.bytes().all(|b| b.is_ascii_alphanumeric() || matches!(b, b'.'|b'_'|b'-'))
}
pub fn is_safe_share_path(p: &str) -> bool {
    !p.is_empty() && !p.starts_with('-') && std::path::Path::new(p).is_absolute()
}
pub fn config_shares_safe(config: &VmConfig) -> bool { /* safe tag+path; unique tags (HashSet) */ }
pub fn validate_shared_folders(config: &VmConfig) -> Result<()> {
    // config_shares_safe -> Error::Config; then each host_path .is_dir() -> Error::Config
}
// load_all: add arm  Ok(c) if !config_shares_safe(&c) => { warn; skip }  (after config_disks_safe)
// is_runtime_safe_edit: add  && old.shared_folders == new.shared_folders
```

### qemu/engine.rs
Refactor `start` → `start_inner(&self, config, prelaunch_load: Option<Uuid>)`;
trait `start` = `start_inner(config, None)`. Deltas:
- top of start_inner, when `prelaunch_load.is_none()`: if
  `config.metadata.suspended_snapshot.is_some()` → `Error::Config("…suspended;
  resume or discard…")`.
- before build_args: `crate::library::validate_shared_folders(config)?`.
- `QemuLaunch{ …, prelaunch: prelaunch_load.is_some() }`.
- after connect_qmp, BEFORE registry insert: if `Some(tag) = prelaunch_load`,
  on the local `qmp`: `run_job("snapshot-load", {job-id, tag, vmstate:"disk0",
  devices:["disk0"]}, …)` then `execute("cont")`; on error kill proc + remove
  socket + return (no phantom Running).
```rust
pub async fn suspend(&self, id: &str) -> Result<()> {
    if self.host_arch == "aarch64" && self.accel.is_hardware() {
        return Err(Error::Config("suspend/resume is unavailable with hardware acceleration (HVF) on this host".into()));
    }
    // start_lock → snapshot_route(Live only, else Error::Config "not running") → drop guard
    // on vm.qmp: execute("stop"); run_job("snapshot-save", {job-id, tag, vmstate:"disk0", devices:["disk0"]})
    // re-read config; metadata.suspended_snapshot = Some(suspend_id); save_config  (persist BEFORE kill)
    // self.kill(id)   // terminate + del socket + drop entry
}
pub async fn restore_suspended(&self, id: &str) -> Result<()> {
    // load config; tag = suspended_snapshot or Error::Config("not suspended")
    // verify tag present via info_json+parse_info_snapshots; if missing → clear field + Error::Config("reset to stopped")
    // start_inner(&config, Some(tag)); then re-read, clear suspended_snapshot, save_config
}
pub async fn discard_suspend(&self, id: &str) -> Result<()> { /* best-effort snapshot_delete_offline + clear field */ }
// update_config: after reject_if_live, re-read config; if suspended_snapshot.is_some() → Error::Config("cannot edit … while suspended")
```
`list_all_detailed`/`map_status`: NO change (suspended VM is absent from registry
→ Stopped; `suspended` bool joined at IPC).

### hypervisor.rs
Add to the trait: `async fn suspend(&self, id:&str) -> Result<()>;` and
`async fn restore_suspended(&self, id:&str) -> Result<()>;` (resume = cont stays).

## C. IPC (src-tauri)
```rust
#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct SharedFolderDto { pub host_path: String, pub mount_tag: String, #[serde(default)] pub read_only: bool }
// From<SharedFolder> / From<SharedFolderDto> both ways.
// VmConfigDto: add  pub shared_folders: Vec<SharedFolderDto>,  pub suspended: bool,
// UpdateVmRequest: add  #[serde(default)] pub shared_folders: Vec<SharedFolderDto>,
// CreateVmRequest: UNCHANGED. VmListItem: add  pub suspended: bool,
// From<VmConfig> for VmConfigDto: shared_folders mapped; suspended = metadata.suspended_snapshot.is_some()
// update_vm body: config.shared_folders = req.shared_folders.into_iter().map(Into::into).collect();
// list_vms map: suspended = cfg.map(|c| c.metadata.suspended_snapshot.is_some()).unwrap_or(false)

#[tauri::command] pub async fn suspend_vm(state, id: String) -> Result<(),String>;   // hv.suspend
#[tauri::command] pub async fn restore_vm(state, id: String) -> Result<(),String>;   // hv.restore_suspended (NOT resume_vm = cont)
```
Register `suspend_vm`, `restore_vm` in lib.rs. Both single-word `id` → no
`rename_all`. **Update wire-shape tests in the SAME commit**:
`vm_list_item_wire_shape` (+`suspended` → 9 keys), `vm_config_dto_wire_shape`
(+`shared_folders`,`suspended`) + a `shared_folder_dto_wire_shape`
(`host_path,mount_tag,read_only`); `requests_parse_snake_case` parses
`UpdateVmRequest` with/without `shared_folders`.

## D. FRONTEND (src/)
ipc.ts: `VmState` UNCHANGED (7 values). Add `SharedFolder` interface; extend
`VmConfig`/`VmListItem` (`+shared_folders?/suspended`), `UpdateVmRequest`
(`+shared_folders?`); wrappers `suspendVm(id)`/`restoreVm(id)` (existing
`resumeVm` = cont stays). format.ts: `isSuspended(vm) = state==="stopped" &&
vm.suspended` (no state/tone change).

New: `components/editor/tabs/SharedFoldersTab.tsx` (controlled, mirrors
NetworkForm), `components/common/SharedFolderRow.tsx` (DirectoryPicker + tag
Input + read-only checkbox + remove + inline error), `components/common/
DirectoryPicker.tsx` (sibling of IsoPicker, `open({directory:true})`).
validation.ts: `MAX_SHARED_FOLDERS=8`, `MAX_MOUNT_TAG_LEN=31`,
`validateMountTag`, `validateSharedFolders` (dup tag, empty path),
`normalizeSharedFolders`.

Modified: HardwareEditorView (EditorDraft.shared; Shared-folders tab;
sharedValid gate; dirty; save payload `shared_folders: normalizeSharedFolders`).
StatusBadge (optional `suspended` prop → violet "Suspended" pill, no
animate-pulse). QuickActions (VmActions += onSuspend/onRestore; `suspended =
state==="stopped"&&vm.suspended`, `frozen = live||suspended`; running → add
Suspend (Moon); suspended → Resume + "Discard & stop"; Edit/Clone/Rename gated by
`frozen`; spinner on busy). ConsoleView (Suspend toolbar button when running →
onSuspend). App.tsx (actions.onSuspend = runAction suspendVm; onRestore =
restoreAndConsole mirroring startAndConsole; ConsoleView onSuspend → suspend then
backToLibrary). VmCard (pass `suspended` to StatusBadge). Wizard: skip (editor
only). Guest-mount help callout in SharedFoldersTab: `mount -t 9p -o
trans=virtio,version=9p2000.L <tag> /mnt/shared` + "Linux/Unix guests only". No
shadcn adds (new lucide icon: Moon).

## E. TEST PLAN
- **args.rs:** fsdev+device emission; readonly; host_path comma-escaped;
  multi-folder fsdevN indexing; `prelaunch_appends_dash_S` + `cold_start_never_emits_dash_S`.
- **library.rs:** is_safe_mount_tag table; is_safe_share_path (relative/leading-
  dash rejected); config_shares_safe dup-tag; load_all_skips_unsafe_share;
  runtime_safe_edit flips on shared_folders change.
- **qmp.rs:** snapshot-load run_job concludes (scripted harness).
- **engine.rs (mock qemu-img):** suspend_refused_under_hvf;
  restore_missing_suspend_state_resets; edit_refused_while_suspended;
  start_refused_while_suspended; discard_suspend_clears_field; suspended VM lists
  as Stopped + `suspended:true`.
- **Gated real-host:** 9p device-present (HVF, `VMFORGE_REAL`: boot + QMP
  `qom-list` contains virtio-9p-pci); 9p mount round-trip (HVF, `VMFORGE_ISO`,
  Alpine mount + read/write + readonly); bad-path → Error::Config; **suspend/
  resume cycle under TCG** (`-accel tcg -cpu cortex-a72`: suspend→save row→
  restore -S+load+cont→running, field cleared); suspend-negative on HVF (gate
  Error::Config); frozen-process safety (tag deleted between save/restore).
- **Vitest:** validateMountTag/validateSharedFolders/normalizeSharedFolders;
  SharedFolderRow; ipc suspendVm/restoreVm invoke names; QuickActions running→
  Suspend, suspended→Resume+Discard, Edit/Clone/Rename disabled when suspended.

## F. SEQUENCING & ACCEPTANCE
Stages: (1) CORE (model → args → library → engine → hypervisor; fix literals);
(2) IPC (SharedFolderDto + DTO extensions + suspend_vm/restore_vm + register +
wire-shape tests same commit); (3) FRONTEND.

Green-gate each stage:
```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --workspace
npm run typecheck
npm run build
npm test
```

Must NOT break Phase 1-4: `VmState` stays exactly 7 lowercase variants
(json_wire_casing/json_ipc_round_trip/maps_run_states untouched); `map_status`
unchanged; wire-shape tests for vm_list_item/vm_config_dto updated in the SAME
commit (additive `suspended`/`shared_folders` keys — the intended tripwire);
serde back-compat (new fields `#[serde(default)]`); `build_args_never_emits_snapshot_flag`
+ new `cold_start_never_emits_dash_S`; lock discipline (snapshot-load/cont on
local QmpClient before insert; suspend drops start_lock before the job); reaper/
parent-protection/snapshot guards unchanged; gated tests stay skip-by-default;
IPC `resume_vm` (= cont) UNCHANGED (new ops are suspend_vm/restore_vm).
