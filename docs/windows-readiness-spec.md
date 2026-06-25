# Windows-readiness — implementation spec

Closes the gaps the Windows-readiness audit found so VMForge can boot an ISO on
Windows (incl. an x86_64 UEFI Windows ISO). Engine-only where possible; the
Tauri shell still needs a real Windows host for WebView2 packaging.

## Ground truth (verified)

- `vmforge-core` **already cross-compiles** to `x86_64-pc-windows-gnu`
  (`cargo check -p vmforge-core --target x86_64-pc-windows-gnu` is green). The
  `cfg(not(unix))` TCP-QMP branch and all deps build. Blocker B1 (the engine
  half) is therefore verifiable from this host going forward.
- Only `aarch64-apple-darwin` was installed before; mingw-w64 + the gnu target
  are now present for ongoing compile-checks.

## Decisions

- **D-W1 — Guest arch is a create-time choice.** `VmConfig.guest_arch:
  Option<String>` (`#[serde(default)]`; `None` = host arch, fully back-compat).
  `"x86_64"` | `"aarch64"`. NOT editable after create — the installed OS arch is
  fixed to the disk, so the hardware editor never changes it.
- **D-W2 — Cross-arch ⇒ TCG.** When `guest_arch != host_arch`, the launch forces
  `Accelerator::Tcg` and the VM is `emulated`. Hardware accel (HVF/WHPX/KVM)
  only when guest == host. This also drives `VmSummary.emulated`/`accelerator`.
- **D-W3 — x86_64 firmware = UEFI (OVMF) when found, else SeaBIOS.** Locate an
  OVMF code blob (`edk2-x86_64-code.fd`, `OVMF_CODE_4M.fd`, `OVMF_CODE.fd`) plus
  a writable VARS template (`edk2-i386-vars.fd`, `OVMF_VARS_4M.fd`,
  `OVMF_VARS.fd`); copy the template once into the VM dir as `OVMF_VARS.fd`
  (per-VM writable) and emit `-drive if=pflash` unit0 (code, ro) + unit1 (vars).
  If no OVMF is found, fall back to built-in SeaBIOS (no firmware args) with a
  logged warning — Linux/legacy ISOs still boot; Windows 11 needs OVMF.
  aarch64 keeps `-bios <edk2-aarch64-code.fd>` (already verified booting).
- **D-W4 — Firmware modeled as an enum** in the arg layer:
  `Firmware::Bios(&Path)` | `Firmware::Pflash { code, vars }`. The engine builds
  an owned variant and borrows it into `QemuLaunch`.
- **D-W5 — Resolver Windows prefixes broadened.** Env-derived dirs
  (`%ProgramFiles%\qemu`, `%ProgramW6432%\qemu`, `%ProgramFiles(x86)%\qemu`,
  scoop, winget Links, msys64). `.exe` candidate handling already correct.
  `storage::qemu_img_bin` last-resort fallback uses `qemu-img.exe` on Windows.
- **D-W6 — QMP-over-TCP bind-failure detection.** A failed loopback bind for the
  QMP port is detected in qemu.log and mapped to a clean, actionable
  `Error::Config` instead of an opaque 15 s QMP-timeout. (The bind:0→drop→spawn
  TOCTOU window remains, microseconds on a loopback ephemeral port.)
- **D-W7 — WHPX fallback warning** elaborated on Windows: mention Hyper-V / WSL2
  / VBS as the usual reason WHPX is unavailable (per CLAUDE.md mandate).
- **D-W8 — Verification.** Keep the mac suite green; keep the
  `x86_64-pc-windows-gnu` cross-check green; add Windows-gated resolver tests
  exercising the `.exe`/prefix branch; add args tests for x86 pflash + SeaBIOS
  fallback + the cross-arch TCG downgrade.

## Touch list

- `model.rs` — `VmConfig.guest_arch` + `effective_arch(host)`; `VmSummary` doc.
- `qemu/firmware.rs` — keep `find_aarch64_uefi`; add `find_x86_64_uefi` →
  `X86Firmware { code, vars_template }`; shared share-dir search helper.
- `qemu/args.rs` — `Firmware` enum; pflash emission; cross-arch already keyed on
  `guest_arch`; tests updated.
- `qemu/engine.rs` — compute `guest_arch`/`emulated`/effective `accel`; pick bin
  by guest arch; firmware selection (aarch64 bios / x86 OVMF+vars-copy /
  SeaBIOS); QMP bind-failure error mapping; summaries reflect emulated/accel.
- `qemu_resolve.rs` — Windows prefixes; Windows-gated tests.
- `storage.rs` — `.exe` last-resort on Windows.
- `host.rs` — Windows WHPX/Hyper-V warning.
- `src-tauri/src/commands.rs` — `guest_arch` on `CreateVmRequest`,
  `VmConfigDto`, `VmListItem`; thread into `create_vm`; wire-shape tests.
- `src/lib/ipc.ts` — `guest_arch` on the mirrored types.
- `src/components/wizard/*` — arch selector in Basics (default host arch, honest
  "emulated, slow" note for non-native); Review row; thread through submit.
