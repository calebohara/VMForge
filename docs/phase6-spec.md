# VMForge Phase 6 — Distribution (locked spec)

Packaging + first-run QEMU UX (verifiable here) + signing/notarization +
auto-update + CI config (as code, inert/documented). Sequential team:
**build/config → first-run UX → updater/CI**. Config keys verified against
@tauri-apps/cli 2.11.3 + tauri-plugin-updater 2.10.0. No overrides.

## A. DECISIONS LOCKED
- **D1 QEMU strategy = require system QEMU, spawn arm's-length. Do NOT bundle**
  (GPLv2: bundling makes the signed installer a combined work + forces re-signing
  QEMU). First-run gate enforces presence; `linux.deb.depends` declares it.
- **D2 hardened runtime = true, NO JIT entitlements on VMForge.** TCG `MAP_JIT`
  happens inside the QEMU child (own signing identity; brew QEMU is ad-hoc +
  `com.apple.security.hypervisor` → HVF+TCG work). `allow-jit`/`disable-library-
  validation` only matter in a future bundling phase (documented, not enabled).
- **D3 PATH gotcha (CRITICAL, verified fatal):** `launchctl getenv PATH` is empty
  → a Finder-launched `.app` gets `/usr/bin:/bin:/usr/sbin:/sbin`; QEMU lives in
  `/opt/homebrew/bin` (disjoint). Fix: one `resolve_qemu_binary(name) ->
  Option<PathBuf>` used by **probe, firmware, AND spawn**; **spawn by absolute
  path**, not bare name; search order = (1) user override (persisted), (2) `$PATH`
  via `which`, (3) hardcoded prefixes (`/opt/homebrew/bin`, `/usr/local/bin`,
  `/opt/local/bin` mac; `/usr/bin` linux; `C:\\Program Files\\qemu`, MSYS2 win);
  require `--version` success (a 0-byte binary reads absent). Prepend the resolved
  bin dir to the child's PATH. Plus a "Locate QEMU…" picker writing the override.
- **D4 verified-here vs CI-only:** LOCAL = unsigned `tauri build` → arm64
  `VMForge.app` + `VMForge_0.1.0_aarch64.dmg`, launches, gate behavior incl. the
  PATH false-negative→fix, HVF from Finder. CI-only/documented = Developer-ID
  sign + notarize + staple, universal build, Windows/Linux bundles, updater e2e.
- **D5 updater = WIRED but NOT ACTIVATED.** Committed `createUpdaterArtifacts:
  false` + placeholder `pubkey` (a placeholder pubkey with `true` would break the
  local build); `check()` just toasts until activated. release.yml flips it true
  via a `--config` patch.
- **D6** release.yml on `push: tags: ['v*']`, secret-guarded/fork-safe; ci.yml
  stays the PR gate, untouched.
- **D7** replace `security.csp: null` with a real CSP (must allow
  `ws://127.0.0.1:*` for the noVNC bridge) before any signed release — tracked,
  NOT required for the local unsigned proof.
- **D8** updater = roll-forward only; private key offline backup; auto-update on
  macOS+Windows, Linux = manual download.

## B. BUILD / PACKAGING
### B.1 tauri.conf.json — `bundle` additions (key fields)
`category "DeveloperTool"`, short/longDescription, copyright, `license
"GPL-2.0-or-later"`, keep `targets "all"`, **`createUpdaterArtifacts: false`**;
`macOS { minimumSystemVersion "11.0", hardenedRuntime true, entitlements
"entitlements.plist", signingIdentity null, dmg {...} }`; `windows {
digestAlgorithm sha256, timestampUrl, certificateThumbprint null, nsis {
installMode perMachine } }`; `linux { deb { depends ["qemu-system-x86",
"qemu-system-arm","qemu-utils"] }, appimage {} }`. Add top-level `plugins.updater
{ pubkey: "PLACEHOLDER…", endpoints: ["https://github.com/OWNER/VMForge/releases/
latest/download/latest.json"], windows { installMode "passive" } }`. Keep
`security.csp: null` for now (D7 comment inline).

### B.2 src-tauri/entitlements.plist (NEW, minimal — D2)
`com.apple.security.network.client` + `.network.server` (noVNC bridge + updater
HTTPS) + `com.apple.security.files.user-selected.read-write` (ISO/disk/dir
pickers). NO `allow-jit`, NO `disable-library-validation` (no in-app JIT, no
bundled dylibs). Future-bundling-only set documented in a comment.

### B.3 deps + registration
src-tauri/Cargo.toml (desktop-only target table): `tauri-plugin-updater = "2"`,
`tauri-plugin-process = "2"`. src-tauri/src/lib.rs: add `.setup(|app| { #[cfg(desktop)]
{ use tauri::Manager; app.handle().plugin(tauri_plugin_updater::Builder::new().build())?;
app.handle().plugin(tauri_plugin_process::init())?; } Ok(()) })` before `.manage`.
capabilities/default.json permissions += `"updater:default"`, `"process:allow-restart"`.

### B.4 package.json scripts
`tauri:build`, `tauri:build:mac` (`tauri build --bundles app,dmg`),
`tauri:build:mac:universal`, `tauri:build:win`, `tauri:build:linux`,
`tauri:build:debug`, `updater:keygen` (`tauri signer generate -w .tauri/updater.key`).

### B.5 D3 engine (the critical correctness fix)
Add `resolve_qemu_binary(name: &str) -> Option<PathBuf>` (host.rs or a small
module) — override → `$PATH` (which) → prefix fallback; require `--version` ok.
Wire into: `host::probe()` (system_binaries presence/version via resolved path),
`qemu/firmware.rs` (reuse for prefix discovery), `qemu/process.rs` spawn (take an
absolute binary path), `qemu/engine.rs` start (resolve once; pass absolute path;
prepend resolved bin dir to the child `PATH` env). Keep arg construction/QMP/
snapshot behavior unchanged. The user override = a persisted setting (a simple
JSON/TOML in the app config dir, or an env `VMFORGE_QEMU_DIR`) consumed by the
resolver; the "Locate QEMU…" picker writes it. Unit-test the resolver
(override/PATH/prefix/missing) with a temp dir + fake executable.

## C. FIRST-RUN UX
No new IPC command — `probe_host` is authoritative + idempotent; the gate
re-invokes it. `src/hooks/useHostCaps.ts` MODIFY: add `refresh(): Promise<...>` +
`refreshing` (distinct from initial `loading`); `mounted` ref guards async.
NEW `src/lib/hostStatus.ts` (pure): `qemuMissing(caps)` (no system binary OR
`!qemu_img.present`), `hasHostWarnings(caps)` (`warnings.length>0 ||
!hardware_accelerated`), `nativeSystemBinaryName(arch)`, `requiredBinaries(caps)`,
`installGuide(os)` (macos `brew install qemu`; debian/fedora/arch; windows
installer + PATH). NEW `src/components/host/{QemuRequiredGate, InstallInstructions,
MissingBinaryList, HostWarningsBanner, HostProbeLoading}.tsx`.
**Gate placement:** top-level early-return in `App.tsx` BEFORE the View machine:
if `caps===null && loading` → `<HostProbeLoading/>`; if `qemuMissing(caps)` →
`<QemuRequiredGate caps error rechecking={refreshing} onRecheck={refresh}/>`
inside `<AppShell>`; else fall through to existing views. Soft banner
`<HostWarningsBanner caps/>` at top of `LibraryView` (null when clean). Gate has
Re-check (re-probe, no restart), "Locate QEMU…" picker (D3 override), and a docs
link via `tauri-plugin-opener.openUrl` (add `@tauri-apps/plugin-opener` JS dep, or
render URL as text). Copy is honest — names the actual missing binary + found
versions from caps. Vitest: hostStatus predicates, useHostCaps refresh,
QemuRequiredGate, HostWarningsBanner, App gate-wiring (missing → gate + no library;
healthy → library, no gate).

## D. AUTO-UPDATE + CI (NOT ACTIVATED)
JS deps `@tauri-apps/plugin-updater ^2` + `@tauri-apps/plugin-process ^2`.
`plugins.updater` block (§B.1) with placeholder pubkey + GitHub endpoint.
Keypair flow (off-sandbox): `tauri signer generate -w ~/.tauri/vmforge_updater.key`
→ paste `.pub` content into `pubkey`; private key → CI secret
`TAURI_SIGNING_PRIVATE_KEY`. Feed = static `latest.json` as a GitHub release asset.
NEW `src/lib/updater.ts`: `checkForUpdates()` using `check()` +
`downloadAndInstall()` progress + `relaunch()` (toasts errors — inert until
activated); bind to an AppShell "Check for updates…" menu item.
NEW `.github/workflows/release.yml` (tag `v*`, `permissions: contents: write`,
matrix macos arm64+x86_64/ubuntu-22.04/windows; `tauri-apps/tauri-action@v0` with
`releaseDraft: true`, args incl `--config src-tauri/tauri.release.conf.json
--bundles …`; env = TAURI_SIGNING_*, APPLE_* secrets placeholders; secret steps
only on tag push). NEW `src-tauri/tauri.release.conf.json` = `{ "bundle": {
"createUpdaterArtifacts": true } }` (merged over base via --config, never committed
to base). ci.yml UNCHANGED.

## E. TEST / VERIFY PLAN
Local (run here): (1) `npm run tauri:build:mac` produces `VMForge.app` +
`VMForge_0.1.0_aarch64.dmg`; (2) the green gate; (3) first-run gate Vitest; (4)
D3 resolver unit test. CI-only/documented: signing/notarize/staple, universal,
Windows/Linux, updater e2e, Gatekeeper `spctl`.

## F. SEQUENCING & ACCEPTANCE
Stages: (1) build/config + **D3 engine** (resolver in probe/firmware/process/
engine; tauri.conf bundle+plugins; entitlements; deps; lib.rs setup; capabilities;
scripts); (2) first-run UX (hostStatus, host/ components, useHostCaps.refresh,
App gate, LibraryView banner, Locate-QEMU picker, Vitest); (3) updater/CI (deps,
updater.ts + menu, release.yml, tauri.release.conf.json — all inert).

Green gate (each handoff) PLUS a successful release `npm run tauri:build:mac`
producing a macOS artifact that launches + clears the QEMU gate:
```
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check --workspace
npm run typecheck
npm run build
npm test
```

Must NOT break Phase 1-5: no new IPC command (probe_host unchanged; useHostCaps
gains refresh additively); View union untouched (gate is an early-return outside
it); engine boundary intact (D3 refactors binary RESOLUTION only — no change to
arg construction/QMP/snapshot/clone or AppState wiring; existing engine + gated
tests stay green); ci.yml PR gate unchanged (release.yml additive, off PR path);
updater inert (committed createUpdaterArtifacts:false + placeholder pubkey keeps
the local build working). Pre-release follow-ups (not local blockers): D7 real
CSP, D8 key backup/roll-forward doc.
