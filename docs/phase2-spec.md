# VMForge Phase 2 — Implementation Spec (locked)

Persistence (`vmforge.toml` + library) → split create/start → live-status merge →
New-VM wizard → hardware editor. Sequential teams: **storage → IPC → frontend**.

---

## ORCHESTRATOR OVERRIDES (authoritative — take precedence over anything below)

1. **VNC start serialization uses a dedicated `start_lock: tokio::sync::Mutex<()>`,
   NOT the `running` registry lock.** Hold `start_lock` across display-pick → spawn →
   QMP-connect so two concurrent `start_vm` calls can't pick the same VNC display.
   Do NOT hold the `running` lock across the (up-to-15s) QMP connect — that would block
   `list_vms`/`running_states`/`state` polling and freeze the UI. The `running` lock is
   taken only for short insert/lookup/remove operations.
2. **`emulated` = `false` for all Phase 2 VMs.** There is no `guest_arch` field on
   `VmConfig` today (Phase 1 assumes guest arch == host arch). Add `emulated` to
   `VmSummary` and always set it `false` for now. Do NOT invent a `guest_arch` schema
   field. `accelerator` = the host's preferred accelerator.
3. **Pass snake_case arg keys from JS `invoke` calls** to avoid Tauri camel/snake
   conversion ambiguity: `invoke("delete_vm", { id, delete_disks })` and
   `invoke("rename_vm", { id, new_name })`. Rust params stay `delete_disks` / `new_name`.
4. **`VmConfig` has no `Default`.** Adding `schema_version`/`dir_slug`/`metadata` breaks
   every literal `VmConfig { .. }` construction (in `commands.rs`, `tests/boot.rs`). Fix
   all literal sites in Stage 0. Do not derive `Default` for `VmConfig`.

---

## A. DECISIONS LOCKED

1. **Library index = directory scan, no index file.** Library = subdirs under
   `~/VMForge/` each with a `vmforge.toml`. `list` = `read_dir(root)` → skip
   non-dirs/dotfiles → parse each `vmforge.toml`. A malformed config is skipped with
   `tracing::warn`, never fatal. No top-level index file in Phase 2.
2. **Identity = stable `Uuid`; directory = sanitized slug.**
   - `VmConfig.id: Uuid` is the one identity. Running registry stays keyed by
     `id.to_string()`. QMP socket stays `runtime_dir()/{id}.sock` (sockaddr_un limit).
   - On-disk dir = `~/VMForge/<slug>/`, `slug = slugify(name)` with collision suffixes
     (`-2`, `-3`). Slug persisted in `vmforge.toml` as `dir_slug`.
3. **Rename = metadata only.** `id` never editable. Rename changes `name` (+ `updated_at`)
   only; the directory does NOT move (`dir_slug` immutable for the VM's life). Safe even
   while running.
4. **Create-vs-start = hard split.** `create_vm` persists (dir + toml + qcow2), does NOT
   launch. `start_vm` loads config by id and launches. A VM can exist `Defined` forever.
5. **`create_and_start_vm` is REMOVED** from `commands.rs`, `lib.rs`, `ipc.ts`. Wizard's
   "create & start" = `createVm(req)` then `startVm(id)`.
6. **Polling, not events, in Phase 2.** Frontend reads state via one batched `list_vms`
   on a 2000ms interval (paused on `visibilitychange`). Events deferred to Phase 3.
7. **Editable only while stopped.** `update_vm` allowed only when effective state ∈
   `{Defined, Stopped}`; otherwise `Error::Config`. `delete_vm` likewise rejected while
   live. **Disk add/resize is OUT of scope for Phase 2** (`update_vm` cannot change
   `disks`). Editable: `name`, `hardware.cpus`, `hardware.memory_mib`, `network.*`, `iso`.
8. **Two pre-existing defects fixed in Phase 2:**
   - **Natural-exit reaper:** every registry state-read first `try_wait()`s; if the
     process exited, report `Stopped`, remove the registry entry, delete its QMP socket.
   - **VNC display TOCTOU:** serialize via `start_lock` (see override #1).
9. **Wire casing = snake_case everywhere.** `VmState` lowercase, `NetworkMode` kebab-case
   (`"host-only"`). Forbidden to add `rename_all="camelCase"` on any boundary type.
   Enforced by a JSON round-trip test.
10. **Error contract = `Result<T, String>` at IPC (unchanged).** Store maps missing
    `vmforge.toml` → `Error::VmNotFound` (stable message). Frontend never branches on
    error text (toast + reload).
11. **`VmSummary` gains `accelerator: Accelerator` + `emulated: bool`** (derived
    server-side, not persisted). See override #2.
12. **`DisplayConfig.vnc_port` becomes `#[serde(skip)]`** (runtime-only).
13. **`VmConfig` gains `schema_version: u32` (default 1), `dir_slug: String`,
    `metadata: VmMetadata`** — additive, serde-default. See override #4.
14. **App-restart = no re-attach.** Registry empty after restart; VMs list `Stopped`/
    `Defined`. Orphan QEMU/sockets not reaped in Phase 2 (documented).
15. **Atomic writes:** `vmforge.toml` via temp-file + rename.
16. **Vitest stood up** (none today).

---

## B. CORE (`vmforge-core`) changes

### B.1 File / module list
| File | Change |
|---|---|
| `src/model.rs` | add `VmMetadata`; add `VmConfig.metadata`/`.schema_version`/`.dir_slug`; `#[serde(skip)]` on `DisplayConfig.vnc_port`; add `accelerator`/`emulated` to `VmSummary` |
| `src/paths.rs` | add `vm_config_path`, `CONFIG_FILENAME`; `vm_dir(library, slug)` now slug-addressed |
| `src/library.rs` | NEW — `Library` store, `slugify`, `validate_vm_name`, `is_runtime_safe_edit` |
| `src/storage.rs` | add `VMFORGE_QEMU_IMG` env seam (CI mock) |
| `src/qemu/engine.rs` | hold a `Library`; add `list_all`/`running_states`/`running_state`; reaper in state-reads; `start_lock` serialized VNC pick; `start` uses `dir_slug` not `name` |
| `src/lib.rs` | `pub mod library;` |
| `Cargo.toml` | add `toml = "0.8"`; dev-dep `tempfile = "3"` |

### B.2 `model.rs` additions (serde-stable, snake_case)
```rust
fn default_schema_version() -> u32 { 1 }

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VmMetadata {
    #[serde(default)] pub created_at: Option<String>, // RFC3339 UTC
    #[serde(default)] pub updated_at: Option<String>,
    #[serde(default)] pub notes: String,
    #[serde(default)] pub os_hint: Option<String>,
}
// VmConfig — add:
//   #[serde(default = "default_schema_version")] pub schema_version: u32,
//   #[serde(default)] pub dir_slug: String,
//   #[serde(default)] pub metadata: VmMetadata,
// DisplayConfig.vnc_port: #[serde(skip)] pub vnc_port: Option<u16>,
// VmSummary — add: pub accelerator: Accelerator, pub emulated: bool,
```
`Accelerator` lives in `host.rs` — `use crate::host::Accelerator` in model.rs.

### B.3 `paths.rs`
```rust
pub const CONFIG_FILENAME: &str = "vmforge.toml";
pub fn vm_config_path(library: &Path, slug: &str) -> PathBuf {
    vm_dir(library, slug).join(CONFIG_FILENAME)
}
```
`engine.rs` start() must change `vm_dir(&self.library_dir, &config.name)` →
`vm_dir(&self.library_dir, &config.dir_slug)`.

### B.4 `library.rs` (NEW) — public surface (all async fns return `Result<T>`)
```rust
pub struct Library { root: PathBuf }
impl Library {
    pub fn open_default() -> Result<Self>;     // root = paths::library_dir()
    pub fn new(root: PathBuf) -> Self;
    pub fn root(&self) -> &Path;
    pub fn to_toml(config: &VmConfig) -> Result<String>;     // pure
    pub fn from_toml(s: &str) -> Result<VmConfig>;           // pure
    pub async fn create_vm(&self, config: VmConfig) -> Result<VmConfig>; // assign slug+timestamps, mkdir, atomic write, create qcow2, NO launch; refuse if config dir already has a vmforge.toml
    pub async fn save_config(&self, config: &VmConfig) -> Result<()>;    // atomic overwrite, bump updated_at, no disk touch
    pub async fn load_config(&self, id: &VmId) -> Result<VmConfig>;      // scan, match id; missing => VmNotFound
    pub async fn list_vms(&self) -> Result<Vec<VmSummary>>;              // all persisted as Defined; skip malformed (warn); absent root => []
    pub async fn load_all(&self) -> Result<Vec<VmConfig>>;
    pub async fn delete_vm(&self, id: &VmId, delete_disks: bool) -> Result<()>; // delete_disks=true removes dir; false removes only toml; unknown => VmNotFound
    pub async fn rename_vm(&self, id: &VmId, new_name: &str) -> Result<VmConfig>; // validate, rewrite name+updated_at; id+slug unchanged
}
pub fn validate_vm_name(name: &str) -> Result<()>; // reject empty, separators, . / .., control chars, Windows-reserved (CON/NUL/PRN/AUX/COM#/LPT#), trailing dot/space
pub fn slugify(name: &str) -> String;              // lowercase, spaces→-, strip non [a-z0-9-_], collapse repeats, never empty (fallback "vm"); collision suffixing done inside create_vm
pub fn is_runtime_safe_edit(old: &VmConfig, new: &VmConfig) -> bool; // true iff only name/metadata differ
```
State-merge lives on `QemuHypervisor` (B.6); the library only produces `Defined` summaries.

### B.5 `storage.rs`
```rust
fn qemu_img_bin() -> String { std::env::var("VMFORGE_QEMU_IMG").unwrap_or_else(|_| "qemu-img".into()) }
// create_qcow2: Command::new(qemu_img_bin()); stays idempotent-on-existence.
```

### B.6 `engine.rs`
```rust
// QemuHypervisor gains:  library: Library,  start_lock: tokio::sync::Mutex<()>
impl QemuHypervisor {
    pub async fn running_state(&self, id: &str) -> Option<VmState>;            // reaps; None if not in registry
    pub async fn running_states(&self) -> HashMap<String, VmState>;           // one lock, reaps exited (remove entry + delete socket)
    pub async fn list_all(&self) -> Result<Vec<VmSummary>>;                   // library Defined ∪ running overlay; each summary carries accelerator+emulated(false)
    // persistence passthroughs so commands.rs stays thin + applies live-state guards:
    pub async fn create_vm(&self, config: VmConfig) -> Result<VmConfig>;
    pub async fn get_config(&self, id: &str) -> Result<VmConfig>;
    pub async fn update_config(&self, id: &str, updated: VmConfig) -> Result<VmConfig>; // reject if live
    pub async fn delete(&self, id: &str, delete_disks: bool) -> Result<()>;             // reject if live; also remove stale socket
    pub async fn rename(&self, id: &str, new_name: &str) -> Result<VmConfig>;
}
```
`list_all` algorithm: library `list_vms()` (state=Defined, accelerator=self.accel,
emulated=false) → overlay `running_states()` by id → include running-but-not-persisted
defensively. `start` holds `start_lock` across display-pick→spawn→QMP-connect.

### B.7 `vmforge.toml` schema
```toml
id = "5f1c0e2a-..."
name = "Alpine VM"
dir_slug = "alpine-vm"
iso = "/path/alpine.iso"
schema_version = 1
[hardware]
cpus = 2
memory_mib = 2048
[[disks]]
path = "disk.qcow2"
size_gib = 8
[network]
mode = "user"
[display]            # vnc_port skipped
[metadata]
created_at = "2026-06-24T17:40:00Z"
updated_at = "2026-06-24T17:40:00Z"
notes = ""
```

---

## C. IPC (`src-tauri`) changes

`AppState { hv: Arc<QemuHypervisor> }` unchanged; `QemuHypervisor` owns the `Library`.
`commands.rs` calls one `hv` method per command, no business logic.

`lib.rs` `generate_handler!`: REMOVE `create_and_start_vm`; ADD `create_vm, list_vms,
get_vm, update_vm, delete_vm, start_vm, rename_vm`. Keep `probe_host, open_console,
vm_state, power_off, force_off, pause_vm, resume_vm`.

### C.2 DTOs (Deserialize req / Serialize resp, snake_case)
```rust
#[derive(Debug,Clone,Deserialize,Serialize)] pub struct HardwareDto { pub cpus: u32, pub memory_mib: u32 }
#[derive(Debug,Clone,Deserialize,Serialize)] pub struct DiskDto { pub path: String, pub size_gib: u32, #[serde(default)] pub backing: Option<String> }
#[derive(Debug,Clone,Deserialize,Serialize)] pub struct NetworkDto { pub mode: NetworkMode, #[serde(default)] pub mac: Option<String>, #[serde(default)] pub port_forwards: Vec<PortForward> }
#[derive(Debug,Deserialize)] pub struct CreateVmRequest { pub name: String, pub hardware: HardwareDto, pub disk_gib: u32, #[serde(default)] pub network: Option<NetworkDto>, #[serde(default)] pub iso: Option<String> }
#[derive(Debug,Deserialize)] pub struct UpdateVmRequest { pub name: String, pub hardware: HardwareDto, #[serde(default)] pub network: Option<NetworkDto>, #[serde(default)] pub iso: Option<String> }
#[derive(Debug,Serialize)] pub struct VmConfigDto { pub id: String, pub name: String, pub hardware: HardwareDto, pub disks: Vec<DiskDto>, pub network: NetworkDto, pub iso: Option<String> }
#[derive(Debug,Serialize)] pub struct VmListItem { pub id: String, pub name: String, pub state: VmState, pub accelerator: Accelerator, pub emulated: bool, pub cpus: u32, pub memory_mib: u32, pub iso: Option<String> }
```

### C.3 Command signatures
```rust
#[tauri::command] pub async fn create_vm(state: State<'_,AppState>, req: CreateVmRequest) -> Result<VmConfigDto,String>;
#[tauri::command] pub async fn list_vms(state: State<'_,AppState>) -> Result<Vec<VmListItem>,String>;
#[tauri::command] pub async fn get_vm(state: State<'_,AppState>, id: String) -> Result<VmConfigDto,String>;
#[tauri::command] pub async fn update_vm(state: State<'_,AppState>, id: String, req: UpdateVmRequest) -> Result<VmConfigDto,String>;
#[tauri::command] pub async fn delete_vm(state: State<'_,AppState>, id: String, delete_disks: bool) -> Result<(),String>;
#[tauri::command] pub async fn start_vm(state: State<'_,AppState>, id: String) -> Result<(),String>;
#[tauri::command] pub async fn rename_vm(state: State<'_,AppState>, id: String, new_name: String) -> Result<VmConfigDto,String>;
// unchanged: probe_host, open_console, vm_state, power_off, force_off, pause_vm, resume_vm
```
Behavior: `create_vm` → `id=Uuid::new_v4()`, clamp `cpus.max(1)`/`memory_mib.max(256)`/
`disk_gib.max(1)`, one disk `disk.qcow2`, empty iso → None, network None →
`NetworkConfig::default()`, delegate to `hv.create_vm`. `start_vm` loads then
`hv.start`. `update_vm`/`delete_vm` reject if live. `rename_vm` allowed while running.
All commands end `.map_err(|e| e.to_string())`. `From<VmConfig> for VmConfigDto` +
`VmSummary`+config → `VmListItem` join (commands.rs fetches `load_all` to fill
cpus/memory_mib/iso; sort by `name.to_lowercase()`).

---

## D. FRONTEND (`src/`) changes

### D.1 `src/lib/ipc.ts` types (snake_case, mirror serde)
```ts
export type NetworkMode = "user" | "bridged" | "host-only";
export interface PortForward { host: number; guest: number; udp: boolean }
export interface Hardware { cpus: number; memory_mib: number }
export interface Disk { path: string; size_gib: number; backing: string | null }
export interface NetworkConfig { mode: NetworkMode; mac: string | null; port_forwards: PortForward[] }
export interface VmConfig { id: string; name: string; hardware: Hardware; disks: Disk[]; network: NetworkConfig; iso: string | null }
export interface VmListItem { id: string; name: string; state: VmState; accelerator: Accelerator; emulated: boolean; cpus: number; memory_mib: number; iso: string | null }
export interface CreateVmRequest { name: string; hardware: Hardware; disk_gib: number; network?: NetworkConfig | null; iso?: string | null }
export interface UpdateVmRequest { name: string; hardware: Hardware; network?: NetworkConfig | null; iso?: string | null }
```
### D.2 wrappers (snake_case arg keys — override #3)
```ts
export const createVm = (req: CreateVmRequest) => invoke<VmConfig>("create_vm", { req });
export const listVms  = () => invoke<VmListItem[]>("list_vms");
export const getVm    = (id: string) => invoke<VmConfig>("get_vm", { id });
export const updateVm = (id: string, req: UpdateVmRequest) => invoke<VmConfig>("update_vm", { id, req });
export const deleteVm = (id: string, delete_disks: boolean) => invoke<void>("delete_vm", { id, delete_disks });
export const startVm  = (id: string) => invoke<void>("start_vm", { id });
export const renameVm = (id: string, new_name: string) => invoke<VmConfig>("rename_vm", { id, new_name });
// keep: probeHost, openConsole, vmState, powerOff, forceOff, pauseVm, resumeVm
// REMOVE: VmDescriptor, NewVmRequest, createAndStartVm
```
### D.3 files
```
src/App.tsx                              rewrite: view machine + AppShell
src/lib/format.ts                        NEW (MiB→GiB, state label/color, accel label)
src/lib/validation.ts                    NEW (pure validators)
src/hooks/useHostCaps.ts                 NEW (probe_host once)
src/hooks/useVmLibrary.ts                NEW (list_vms 2s poll, visibility pause, refresh())
src/components/ui/*                       shadcn-generated
src/components/layout/AppShell.tsx        NEW (titlebar + AccelBadge + breadcrumb + Toaster)
src/components/library/{LibraryView,VmCard,EmptyLibrary,QuickActions,DeleteVmDialog}.tsx  NEW
src/components/wizard/NewVmWizard.tsx     NEW (+ steps/{StepBasics,StepCpuMemory,StepStorage,StepNetwork,StepReview}.tsx)
src/components/editor/HardwareEditorView.tsx  NEW (+ tabs/{ProcessorsTab,MemoryTab,NetworkTab}.tsx) — NO disk tab (Phase 3)
src/components/common/{StatusBadge,AccelBadge,LimitField,IsoPicker,Field}.tsx  NEW
src/components/VncConsole.tsx             UNCHANGED (reused)
```
### D.4 shadcn add (non-interactive; if a component fails, create it manually)
```
npx shadcn@latest add button input label card badge dialog select slider tabs alert alert-dialog dropdown-menu tooltip separator sonner skeleton scroll-area
```
### D.5 view/state model
```ts
type View = { kind:"library" } | { kind:"wizard" } | { kind:"editor"; vmId:string } | { kind:"console"; vmId:string; wsPort:number };
```
Boot: `probe_host` → `list_vms` → library. `caps` once via `useHostCaps`; `vms` via
`useVmLibrary` (2s poll, pause on hidden, `refresh()` after actions); errors via `sonner`
toast. QuickActions state-aware (defined/stopped→Start/Edit/Delete; running→Open
console/Shutdown/Force off/Pause; paused→Resume/Open console/Force off;
starting/stopping→disabled+spinner; error→Force off/Delete). Edit disabled while live
(tooltip). Wizard "Create & start" = createVm→startVm→openConsole→console. Bridged/
host-only shown disabled-with-explanation (Phase 4). Surface host core/RAM headroom.
Console view reuses Phase-1 UX (StatusBadge + Pause/Resume/Shutdown/Force off + VncConsole).

---

## E. TEST PLAN

### E.1 Rust offline unit tests (`cargo test -p vmforge-core`, no QEMU)
Pure: (1) toml_round_trip full VmConfig; (2) toml_back_compat (minimal toml, defaults
fill, schema_version==1); (3) json_ipc_round_trip — serialize each DTO + VmState/
NetworkMode/Accelerator, snapshot exact field names + enum strings (`"host-only"`,
`"hvf"`, `"running"`); (4) display_vnc_port_not_serialized; (5) validate_vm_name_rejects;
(6) slugify_cases; (7) is_runtime_safe_edit.
Library fs (`tempfile::TempDir`): (8) create_then_load; (9) index_list_scan (junk file +
dotfile ignored); (10) list_skips_malformed; (11) save_config_bumps_updated_at + disks
untouched; (12) delete_vm_removes_dir (both delete_disks values) + unknown→VmNotFound;
(13) rename_keeps_id_and_slug; (14) create_vm_refuses_clobber; (15) slug_collision_suffix.
qemu-img: (16) create_qcow2_args (keep); (17) create_vm_idempotent_disk (pre-create file
untouched).
Engine: (18) list_all_defined_with_empty_registry; (19) reaper_drops_exited_process
(spawn `/bin/true` or `sleep 0` into registry path; after exit running_states reports
Stopped + removes entry + deletes socket; no QEMU).

### E.2 Gated boot test (`tests/boot.rs`, VMFORGE_ISO-gated)
Add: `create_vm` (persist, no launch) → new `QemuHypervisor::with_library_dir(same root)`
→ `list_all` shows VM `Defined` → `start_vm` by id → `Running` → `kill`. Default CI stays
offline.

### E.3 Vitest (stand up vitest + @testing-library/react + jsdom + jest-dom first; add
`"test": "vitest run"` script): validation.ts; ipc.ts wrappers call invoke with exact
command + arg shape; useVmLibrary renders states from mocked list_vms + pauses on hidden;
HardwareEditorView disables fields when state≠stopped; DeleteVmDialog confirmation.

---

## F. SEQUENCING & ACCEPTANCE

Green-gate (run after each stage):
```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test -p vmforge-core
cargo check --workspace
npm run typecheck
npm run build
```

- **Stage 0 — model + paths + deps.** model.rs, paths.rs, core Cargo.toml. Fix every
  literal `VmConfig { .. }` (commands.rs, tests/boot.rs) for the 3 new fields. Done when
  fmt/clippy/check/test green and Phase-1 sites compile.
- **Stage 1 — storage.** library.rs (new) + `pub mod library;` + storage env seam +
  engine (Library handle, list_all/running_states/running_state, reaper, start_lock,
  dir_slug in start). Done when gates green PLUS tests E.1 #1–19 pass.
- **Stage 2 — IPC.** commands.rs (DTOs, 7 commands, mappers, remove create_and_start_vm/
  NewVmRequest/VmDescriptor) + lib.rs handler. Done when check/clippy/fmt green +
  json_ipc_round_trip matches + core tests still green.
- **Stage 3 — frontend.** ipc.ts + format/validation + hooks + shadcn add + components +
  App rewrite + Vitest. Done when typecheck/build green + Vitest passing + console path
  works identically to Phase 1.

### Must NOT break from Phase 1
`Hypervisor::start(&self, &VmConfig)` signature unchanged; registry keyed by
`id.to_string()`; `create_qcow2` idempotent-on-existence; VNC display range 5901–5963 and
`5900+display`; VNC bridge reuse in `open_console`; QMP socket at `runtime_dir()/{id}.sock`;
`commands.rs` no business logic; Phase-1 console UX preserved (relocated into console view);
gated `tests/boot.rs` keeps passing.
