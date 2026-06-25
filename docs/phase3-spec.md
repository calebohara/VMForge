# VMForge Phase 3 — Snapshots & Clones (locked spec)

Snapshot tree (live + offline), restore, full/linked clones. Team order:
**storage/engine (vmforge-core) → IPC (src-tauri) → frontend (src/)**.
All QEMU facts verified on qemu/qemu-img **11.0.1**.

---

## ORCHESTRATOR OVERRIDES (authoritative — win over anything below)

1. **`running_states` must stay responsive during a long QMP job.** A live
   `snapshot-save`/`snapshot-load`/`snapshot-delete` holds the VM's single QMP
   channel for the job's duration. When the 2s poll builds live status, do NOT
   block on a busy channel: in `running_states`, after the `try_wait` reaper
   check, acquire the per-VM QMP with `vm.qmp.try_lock()` (tokio) for
   `query_status`; on contention, skip the query and report `VmState::Running`
   (the process is already confirmed alive). This prevents a snapshot on one VM
   from freezing the whole library poll. `run_job` still drives the job while
   holding the QMP guard; it simply must not also be blocked-on by the poll.
2. Everything else in the spec below stands as written.

---

## A. DECISIONS LOCKED

**A1 — Snapshot tree-vs-flat + storage.** qcow2 internal snapshots are a flat
list (tag/numeric id, no parent linkage). The VMware-style tree is OUR overlay:
`#[serde(default)] snapshots: Vec<Snapshot>` embedded in `vmforge.toml` (one
atomic file via `write_config_atomic`; Phase-2 configs load with an empty
array — additive, no schema bump). qcow2 internal snapshots are authoritative
for existence+payload; our metadata adds parent links, our UUID, created_at,
live flag. **Join key: qcow2 `tag` = our `Snapshot.id` (Uuid) as
`id.to_string()`.** On read, reconcile `qemu-img info --output=json`
`.snapshots[]` ∩ metadata: metadata orphans → `present_in_qcow2=false`; qcow2
orphans → parent-less nodes. Tree degrades to flat when all `parent` are `None`.

**A2 — Live mechanism = QMP JOBS.** Use native QMP `snapshot-save` /
`snapshot-load` / `snapshot-delete` (async jobs; args `{job-id, tag, vmstate,
devices:[str]}`). NOT `human-monitor-command` savevm/loadvm/delvm. Extend
`QmpClient` (minimal): add `JobInfo`, `query_jobs()`, and `run_job()` that
queues the job then polls `query-jobs` until concluded/absent (or error → maps
to `Error::Qmp`), then `job-dismiss`. Existing `execute()` already skips
interleaved events.

**A3 — create/restore/delete state rules.**

| Op | Running / Paused | Stopped / Defined |
|---|---|---|
| create | QMP `snapshot-save` → `has_vm_state=true` | `qemu-img snapshot -c` → `has_vm_state=false` |
| delete | QMP `snapshot-delete` | `qemu-img snapshot -d` |
| restore | **REFUSED** (Phase 3) | `qemu-img snapshot -a` (disk-only) |

Refused in transient `Starting`/`Stopping`/`Error` → `Error::Config`. Live-vs-
offline routing decided below the IPC boundary, atomically under `start_lock`
(check→route→exec), never against an image QEMU holds open RW. Offline read
path (`list_snapshots` while VM may run) passes `-U`/`--force-share`.

**A4 — Full vs linked clone.** Full = `qemu-img convert -O qcow2 src dst` (deep
copy, flattened, no backing, snapshots NOT carried). Linked = `qemu-img create
-f qcow2 --backing <rel> --backing-format qcow2 child` (CoW; backing populates
child `DiskSpec.backing`). **Long flags only** — `-F` is deprecated in >10.0.
Both stopped-source-only (`reject_if_live`). Clone = brand-new VM
(`id=Uuid::new_v4()`, fresh `dir_slug` via `unique_slug`), returned as a normal
`VmConfig`. Atomicity: convert to `*.partial` then rename; write the clone's
`vmforge.toml` LAST so a crash leaves an orphan dir invisible to `load_all`.

**A5 — Linked-clone parent protection.** Before `start`/`delete`/
`restore_snapshot` of any VM, scan `load_all()` for any other config whose
`disks[].backing` resolves (relative to that VM's dir) to a disk in the target's
dir; if any dependent exists → `Error::Config("VM <name> has linked clones; …")`.
Backing paths stored **relative** (`../<parent-slug>/<disk>`, forward-slash).

**A6 — Long-op = synchronous.** Every command's Promise resolves on completion.
No progress-event channel in Phase 3. UX = indeterminate spinner. Returns are
DTOs (not `()`) to reserve a future async path.

**A7 — Restore reverts DISK ONLY.** `qemu-img snapshot -a` reverts disk; does
NOT revert RAM or `vmforge.toml`. Live RAM-restore (`snapshot-load` at prelaunch
`-S`) + config-vs-vmstate validation = explicit fast-follow, OUT of Phase-3-core.
Document in the restore confirm dialog.

**A8 — Launch-line P0.** Live snapshots target block-node-names. `args.rs` MUST
add `node-name=disk0` to the boot `-drive` (verified accepted; queryable via
`query-named-block-nodes`). The install ISO node (raw RO cdrom) is EXCLUDED from
`devices`; only writable qcow2 nodes pass. Single-disk Phase-3 scope:
`vmstate:"disk0"`, `devices:["disk0"]`. `-snapshot` is still NEVER emitted.

---

## B. CORE (vmforge-core)

### B1. `model.rs`
```rust
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    pub id: Uuid,                  // stable id AND qcow2 tag (id.to_string())
    pub name: String,
    #[serde(default)] pub parent: Option<Uuid>,   // None => top-level
    pub created_at: String,        // RFC3339 UTC
    #[serde(default)] pub has_vm_state: bool,      // true = live (RAM captured)
    #[serde(default)] pub notes: String,
    #[serde(default)] pub vm_state_size: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnapshotNode {
    #[serde(flatten)] pub meta: Snapshot,
    pub present_in_qcow2: bool,
    pub children: Vec<Uuid>,
}
// VmConfig: add  #[serde(default)] pub snapshots: Vec<Snapshot>,
```

### B2. `storage.rs`
```rust
#[derive(Debug, Clone, Deserialize)]
pub struct Qcow2Snapshot {     // qemu-img info --output=json .snapshots[] [VERIFIED 11.0.1]
    pub id: String,            // qcow2 numeric id
    pub name: String,          // == our Snapshot.id.to_string()
    #[serde(rename="date-sec")] pub date_sec: i64,
    #[serde(rename="date-nsec")] pub date_nsec: i64,
    #[serde(rename="vm-state-size")] pub vm_state_size: u64,
    #[serde(rename="vm-clock-nsec")] pub vm_clock_nsec: u64,
}
// pure argv builders -> Vec<String>:
pub fn snapshot_create_args(disk, tag)  // ["snapshot","-c",tag,disk]
pub fn snapshot_apply_args(disk, tag)   // ["snapshot","-a",tag,disk]
pub fn snapshot_delete_args(disk, tag)  // ["snapshot","-d",tag,disk]
pub fn info_json_args(disk, force_share: bool)  // ["info","--output=json",("-U")?,disk]
pub fn convert_args(src, dst)           // ["convert","-O","qcow2",src,dst]
pub fn linked_overlay_args(child, parent_backing)
//   ["create","-f","qcow2","--backing",parent_backing,"--backing-format","qcow2",child]
// async runners (mirror create_qcow2 error mapping; honor VMFORGE_QEMU_IMG seam):
pub async fn snapshot_create_offline / snapshot_apply_offline / snapshot_delete_offline(disk,tag) -> Result<()>;
pub async fn info_json(disk, force_share) -> Result<String>;
pub async fn convert_qcow2(src,dst) -> Result<()>;           // caller renames *.partial
pub async fn create_linked_overlay(child, parent_backing) -> Result<()>;
// pure parser: missing snapshots key -> Ok(vec![]); malformed -> Err(Serde)
pub fn parse_info_snapshots(stdout: &str) -> Result<Vec<Qcow2Snapshot>>;
```

### B3. `qemu/qmp.rs` (only QmpClient change)
```rust
#[derive(Debug, Clone, Deserialize)]
pub struct JobInfo { pub id: String, pub status: String, #[serde(rename="type")] pub kind: String, pub error: Option<String> }
impl QmpClient {
    pub async fn query_jobs(&mut self) -> Result<Vec<JobInfo>>;
    /// Queue cmd, poll query-jobs until job_id concluded/absent (or error),
    /// then job-dismiss. Bounded by timeout. Non-null job error -> Err(Qmp).
    pub async fn run_job(&mut self, cmd: &str, args: serde_json::Value, job_id: &str, timeout: Duration) -> Result<()>;
}
```

### B4. `qemu/args.rs`
```rust
// boot disk: append ,node-name=disk0
format!("file={},if=virtio,format=qcow2,node-name=disk0", esc(l.disk))
```
Update the existing boot-disk/comma-escape arg tests for the appended
`node-name=disk0`. Keep the `-snapshot`-never-emitted invariant (add a test).

### B5. `library.rs`
```rust
pub async fn full_clone(&self, src_id: &VmId, new_name: &str) -> Result<VmConfig>;   // convert; backing=None; snapshots cleared; toml last
pub async fn linked_clone(&self, src_id: &VmId, new_name: &str) -> Result<VmConfig>; // backing="../<parent-slug>/<disk>"
pub async fn dependents_of(&self, target_disk: &Path) -> Result<Vec<String>>;        // slugs whose backing resolves to target
// pure helpers:
//   reconcile(meta: &[Snapshot], qcow2: &[Qcow2Snapshot]) -> Vec<SnapshotNode>
//   reparent_on_delete(meta: &mut Vec<Snapshot>, removed: Uuid)  // children -> grandparent
```
Reuse `unique_slug`, `is_safe_slug`, clobber guard, `write_config_atomic`.

### B6. `qemu/engine.rs` (on QemuHypervisor)
```rust
pub async fn create_snapshot(&self, id: &str, name: &str, parent: Option<Uuid>, notes: &str) -> Result<Snapshot>;
pub async fn delete_snapshot(&self, id: &str, snapshot_id: Uuid) -> Result<()>;
pub async fn restore_snapshot(&self, id: &str, snapshot_id: Uuid) -> Result<()>;  // stopped-only (A7)
pub async fn list_snapshots(&self, id: &str) -> Result<Vec<SnapshotNode>>;
pub async fn clone_vm(&self, id: &str, new_name: &str, linked: bool) -> Result<VmConfig>;
```
- create: `start_lock` to read state + route. Live → `run_job("snapshot-save",
  {job-id:uuid, tag:uuid, vmstate:"disk0", devices:["disk0"]}, uuid, 5min)` on
  `vm.qmp` (NOT holding running lock). Offline → `snapshot_create_offline`. Then
  append `Snapshot`, persist via `save_config`. Single-disk only (else
  `Error::NotImplemented`).
- delete: route live/offline; `reparent_on_delete`; persist.
- restore: `start_lock`; `reject_if_live`; parent-protection (A5);
  `snapshot_apply_offline`.
- list: reconcile metadata vs `info_json(disk, force_share = is_running)`.
- clone: `start_lock`; `reject_if_live(src)`; delegate to library.
- start / delete (existing): add parent-protection scan (`dependents_of`).

---

## C. IPC (src-tauri/src/commands.rs)
```rust
#[derive(Debug, Clone, Serialize)]
pub struct SnapshotDto {
    pub snapshot_id: String,        // our Snapshot.id
    pub name: String,
    pub parent_id: Option<String>,
    pub created_at: String,
    pub has_vm_state: bool,
    pub vm_state_size: u64,
    pub present_in_qcow2: bool,
}
impl From<SnapshotNode> for SnapshotDto { /* ... */ }

#[tauri::command] pub async fn list_snapshots(state, id: String) -> Result<Vec<SnapshotDto>,String>;
#[tauri::command(rename_all="snake_case")] pub async fn create_snapshot(state, id: String, name: String) -> Result<SnapshotDto,String>;
#[tauri::command(rename_all="snake_case")] pub async fn restore_snapshot(state, id: String, snapshot_id: String) -> Result<(),String>;  // Uuid::parse_str
#[tauri::command(rename_all="snake_case")] pub async fn delete_snapshot(state, id: String, snapshot_id: String) -> Result<(),String>;
#[tauri::command(rename_all="snake_case")] pub async fn clone_vm(state, id: String, new_name: String, linked: bool) -> Result<VmConfigDto,String>;
```
Register all five in `lib.rs`. Every command ends `.map_err(|e| e.to_string())`.
Add `snapshot_dto_wire_shape` key-order test (mirrors `vm_config_dto_wire_shape`).

---

## D. FRONTEND (src/)

### D1. `lib/ipc.ts`
```ts
export interface Snapshot { snapshot_id: string; name: string; parent_id: string | null; created_at: string; has_vm_state: boolean; vm_state_size: number; present_in_qcow2: boolean }
export type CloneKind = "full" | "linked";
export const listSnapshots   = (id) => invoke<Snapshot[]>("list_snapshots", { id });
export const createSnapshot  = (id, name) => invoke<Snapshot>("create_snapshot", { id, name });
export const restoreSnapshot = (id, snapshot_id) => invoke<void>("restore_snapshot", { id, snapshot_id });
export const deleteSnapshot  = (id, snapshot_id) => invoke<void>("delete_snapshot", { id, snapshot_id });
export const cloneVm = (id, new_name, linked) => invoke<VmConfig>("clone_vm", { id, new_name, linked });
```
snake_case arg keys (`snapshot_id`, `new_name`, `linked`).

### D2. View/state
Add `View` kind `{ kind:"snapshots"; vmId:string }` (full view like editor/console).
Clone = dialog owned by `LibraryView` (like `DeleteVmDialog`). Breadcrumbs:
`Library → <vmName> → Snapshots`. `SnapshotsView` receives live `state` from
`useVmLibrary`.

### D3. Files
```
src/components/snapshots/{SnapshotsView,SnapshotTree,SnapshotNode,SnapshotDetail,
  TakeSnapshotDialog,RestoreSnapshotDialog,DeleteSnapshotDialog,EmptySnapshots,
  CloneVmDialog,snapshotTree.ts}.tsx
src/hooks/useSnapshots.ts        on-demand fetch + refresh (NOT polled), mountedRef cancel
src/lib/ipc.ts                   EDIT (D1)
src/lib/format.ts                EDIT snapshotDateLabel + byte size
src/components/library/QuickActions.tsx  EDIT: ⋯ DropdownMenu (Snapshots / Clone… / Rename… / Delete)
src/components/library/LibraryView.tsx   EDIT: own CloneVmDialog state, onConfirmClone
src/components/editor/HardwareEditorView.tsx EDIT: header "Snapshots" button
src/components/console/ConsoleView.tsx   EDIT: toolbar "Snapshot" → snapshots view
src/App.tsx                      EDIT: view kind, breadcrumbs, route, VmActions {onOpenSnapshots,onClone}
```
`snapshotTree.ts buildTree(snapshots)` single-level when all `parent_id` null;
cycle-safe; orphans promoted to roots; `descendantIds`.

### D4. shadcn add
`npx shadcn@latest add radio-group` (Full vs Linked). All else already installed.

### D5. Long-op UX
Refresh-after-success (no optimistic rows). `SnapshotsView` `busyOp`
("take"|"restore"|"delete"|null) disables tree+bar; in-flight button →
`<Loader2 spin>` + verb. Live take/restore keep AlertDialog open with spinner
("Saving memory state — this can take a while"); cancel disabled mid-flight.
`toast.success`/`toast.error(String(e))`. Clone dialog disables footer + "Cloning…";
on success close → library refresh shows new VM `Defined`. Clone menu item
disabled while source live (tooltip). Linked-clone consequence = persistent amber
callout. RAM badge per node from `has_vm_state`.

---

## E. TEST PLAN
Offline unit (mock qemu-img via `VMFORGE_QEMU_IMG`, env_guard): pure argv
(incl. regression guard: `linked_overlay_args` emits `--backing`+`--backing-format`,
NEVER `-F`/`-B`); `parse_info_snapshots` (real captured JSON fixture; missing
key→empty; malformed→Serde); tree model (assembly, reconcile orphans both ways,
`reparent_on_delete`, all-None→flat); library/engine w/ mock binary (`full_clone`
new id/slug, backing=None, snapshots empty, src untouched, *.partial rename,
toml-last → simulated convert failure leaves no adoptable config; `linked_clone`
backing string + `dependents_of`; parent-protection refusals on
start/delete/restore; state refusals; `[[snapshots]]` toml round-trip; Phase-2
config → empty array); `QmpClient::run_job` via scripted in-memory reader/writer
(queue→poll concluded→dismiss; error variant → Err(Qmp)). Gated `tests/boot.rs`
(`VMFORGE_ISO`, `#[ignore]`): real live snapshot-save→load/delete on a booted
guest. Vitest: `snapshotTree.test.ts`, `CloneVmDialog.test.tsx`,
`TakeSnapshotDialog.test.tsx`, `ipc.test.ts` extension.

---

## F. SEQUENCING & ACCEPTANCE
Stages: (1) primitives — model + storage + qmp + args + their unit tests;
(2) library + engine (clones, dependents, reconcile/reparent, 5 engine methods,
parent-protection in start/delete) + tests; (3) IPC; (4) frontend; (5) verify.

Green-gate at end of each stage (stages with frontend include npm):
```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --workspace
npm run typecheck
npm run build
npm test
```

Must NOT break Phase 1/2: Phase-2 `vmforge.toml` loads unchanged (new fields
`#[serde(default)]`); engine concurrency invariants (start_lock TOCTOU, reaper,
force-off) intact; `list_all`/`list_all_detailed` shapes unchanged (tree read
on demand via `list_snapshots`, never in the 2s poll); `args.rs` change is the
appended `node-name=disk0` only; `-snapshot` never emitted; `Result<T,String>`
IPC + `rename_all="snake_case"` rule preserved; no QEMU symbols in commands.rs.
