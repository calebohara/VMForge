//! QEMU binary resolution (Phase 6 — D3, the critical PATH fix).
//!
//! ## Why this exists
//! A Finder-launched macOS `.app` inherits an *empty* `launchctl getenv PATH`,
//! so the webview process gets only `/usr/bin:/bin:/usr/sbin:/sbin`. Homebrew
//! QEMU lives in `/opt/homebrew/bin` — disjoint from that set. Spawning QEMU by
//! the bare name `qemu-system-aarch64` therefore fails with "not found" even
//! though QEMU is installed and works from a shell. The fix (verified fatal
//! otherwise) is to **resolve QEMU to an absolute path once** and use that path
//! everywhere — probe, firmware discovery, and the actual spawn.
//!
//! ## Resolution order
//! 1. **User override** — a persisted setting (`qemu_dir` in the app config) or
//!    the `VMFORGE_QEMU_DIR` environment variable. The "Locate QEMU…" picker in
//!    the first-run gate writes the persisted setting.
//! 2. **`$PATH`** — first match via a minimal cross-platform `which`.
//! 3. **Hardcoded install prefixes** per OS (Homebrew / MacPorts / system /
//!    MSYS2 / `C:\Program Files\qemu`).
//!
//! A candidate is only accepted if `<candidate> --version` exits successfully —
//! that rejects a 0-byte placeholder a stub installer might leave behind.
//!
//! ## Testability seam
//! Everything routes through [`resolve_qemu_binary_with`], which takes the
//! environment lookups and search prefixes as parameters. The public
//! [`resolve_qemu_binary`] wires the real environment in. Unit tests drive the
//! `_with` form against a temp dir + fake executable, so CI without QEMU still
//! exercises override / PATH / prefix / missing paths.

use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

/// Environment variable holding a user-chosen directory containing the QEMU
/// binaries. Takes precedence over `$PATH` and the hardcoded prefixes.
pub const QEMU_DIR_ENV: &str = "VMFORGE_QEMU_DIR";

/// How long a `--version` / `-accel help` probe may run before the binary is
/// treated as unusable. Bounds a hung/stalled candidate (a wrapper script, a
/// binary on a wedged network mount) so it can't freeze the first-run probe or
/// a VM launch. A real `--version` returns in milliseconds.
pub(crate) const PROBE_TIMEOUT: Duration = Duration::from_secs(5);

/// Run `cmd` capturing its output, but kill it and return `None` if it does not
/// exit within `timeout`. Shared by the resolver's `--version` gate and the
/// host probe's `--version`/`-accel help` spawns so a pathological binary never
/// blocks indefinitely.
pub(crate) fn output_with_timeout(cmd: &mut Command, timeout: Duration) -> Option<Output> {
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .ok()?;
    let start = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(_)) => return child.wait_with_output().ok(),
            Ok(None) => {
                if start.elapsed() >= timeout {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(25));
            }
            Err(_) => return None,
        }
    }
}

/// Resolve a QEMU binary (e.g. `"qemu-system-aarch64"`, `"qemu-img"`) to an
/// absolute path, honoring the user override → `$PATH` → install-prefix search
/// order. Returns `None` if no candidate passes a `--version` check.
///
/// This is the single entry point used by [`crate::host::probe`],
/// [`crate::qemu::firmware`], and the launch path in `qemu::engine`.
pub fn resolve_qemu_binary(name: &str) -> Option<PathBuf> {
    resolve_qemu_binary_with(name, override_dir(), env_path(), &default_prefixes())
}

/// The user-override directory, if any: the persisted `qemu_dir` setting first,
/// then the `VMFORGE_QEMU_DIR` env var. `None` when neither is set.
fn override_dir() -> Option<PathBuf> {
    if let Some(dir) = crate::settings::qemu_dir_override() {
        return Some(dir);
    }
    std::env::var_os(QEMU_DIR_ENV)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

/// The current process `PATH`, split into directories.
fn env_path() -> Vec<PathBuf> {
    match std::env::var_os("PATH") {
        Some(p) => std::env::split_paths(&p).collect(),
        None => Vec::new(),
    }
}

/// Install prefixes (directories that hold QEMU binaries) per OS. These are the
/// fallback for the empty-`PATH` launch case (the macOS Finder D3 scenario, and
/// the equivalent on a Windows GUI launch).
fn default_prefixes() -> Vec<PathBuf> {
    if cfg!(target_os = "macos") {
        ["/opt/homebrew/bin", "/usr/local/bin", "/opt/local/bin"]
            .iter()
            .map(PathBuf::from)
            .collect()
    } else if cfg!(target_os = "windows") {
        windows_prefixes()
    } else {
        // Linux and other unix.
        ["/usr/bin", "/usr/local/bin"]
            .iter()
            .map(PathBuf::from)
            .collect()
    }
}

/// Windows QEMU install locations, derived from the environment so they hold
/// regardless of system drive letter or user profile. Covers the official
/// qemu.org installer (`%ProgramFiles%\qemu`), MSYS2, scoop, and winget shims.
/// Reads only environment variables, so it compiles (and harmlessly returns the
/// static MSYS2 entries) on non-Windows too.
fn windows_prefixes() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    let env_dir = |var: &str| {
        std::env::var_os(var)
            .filter(|v| !v.is_empty())
            .map(PathBuf::from)
    };

    // Official qemu.org installer (and ARM64 build) land under Program Files.
    for var in ["ProgramFiles", "ProgramW6432", "ProgramFiles(x86)"] {
        if let Some(p) = env_dir(var) {
            dirs.push(p.join("qemu"));
        }
    }
    // scoop (per-user) and winget shims.
    if let Some(home) = env_dir("USERPROFILE") {
        dirs.push(home.join("scoop").join("shims"));
        dirs.push(home.join("scoop").join("apps").join("qemu").join("current"));
    }
    if let Some(local) = env_dir("LOCALAPPDATA") {
        dirs.push(local.join("Microsoft").join("WinGet").join("Links"));
    }
    // MSYS2 default install roots.
    dirs.push(PathBuf::from(r"C:\msys64\mingw64\bin"));
    dirs.push(PathBuf::from(r"C:\msys64\ucrt64\bin"));
    dirs
}

/// The seam: resolve `name` against an explicit override dir, `PATH` list, and
/// prefix list. Order: override → `path_dirs` → `prefixes`. The first directory
/// holding an executable candidate that passes `--version` wins.
pub fn resolve_qemu_binary_with(
    name: &str,
    override_dir: Option<PathBuf>,
    path_dirs: Vec<PathBuf>,
    prefixes: &[PathBuf],
) -> Option<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Some(d) = override_dir {
        dirs.push(d);
    }
    dirs.extend(path_dirs);
    dirs.extend(prefixes.iter().cloned());

    for dir in dirs {
        if let Some(cand) = candidate_in(&dir, name) {
            if version_ok(&cand) {
                return Some(cand);
            }
        }
    }
    None
}

/// Form the candidate path `dir/name(.exe)` if it exists and is a file.
fn candidate_in(dir: &Path, name: &str) -> Option<PathBuf> {
    let suffixes: &[&str] = if cfg!(windows) { &["", ".exe"] } else { &[""] };
    for suffix in suffixes {
        let cand = dir.join(format!("{name}{suffix}"));
        if cand.is_file() {
            return Some(cand);
        }
    }
    None
}

/// Accept a candidate only if `<candidate> --version` exits successfully. A
/// 0-byte placeholder (or anything not actually runnable) is rejected here.
fn version_ok(bin: &Path) -> bool {
    let mut cmd = Command::new(bin);
    cmd.arg("--version");
    output_with_timeout(&mut cmd, PROBE_TIMEOUT)
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    #[cfg(unix)]
    use std::os::unix::fs::PermissionsExt;

    /// Write a fake executable that prints a QEMU-ish version line and exits 0.
    /// Used only by the unix-gated resolver tests (which rely on `chmod +x`).
    #[cfg(unix)]
    fn write_fake_bin(dir: &Path, name: &str) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        fs::write(
            &path,
            "#!/bin/sh\necho 'QEMU emulator version 11.0.1'\nexit 0\n",
        )
        .unwrap();
        #[cfg(unix)]
        {
            let mut perms = fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&path, perms).unwrap();
        }
        path
    }

    /// Write a 0-byte file (no exec bit) to model a placeholder that must be
    /// rejected by the `--version` gate. Used only by the unix-gated tests.
    #[cfg(unix)]
    fn write_empty_file(dir: &Path, name: &str) -> PathBuf {
        fs::create_dir_all(dir).unwrap();
        let path = dir.join(name);
        fs::write(&path, b"").unwrap();
        path
    }

    #[cfg(unix)]
    #[test]
    fn override_dir_wins_over_path_and_prefix() {
        let tmp =
            std::env::temp_dir().join(format!("vmforge-resolve-override-{}", std::process::id()));
        let over = tmp.join("override");
        let path = tmp.join("path");
        let prefix = tmp.join("prefix");
        let want = write_fake_bin(&over, "qemu-system-x86_64");
        write_fake_bin(&path, "qemu-system-x86_64");
        write_fake_bin(&prefix, "qemu-system-x86_64");

        let got = resolve_qemu_binary_with(
            "qemu-system-x86_64",
            Some(over.clone()),
            vec![path],
            &[prefix],
        );
        assert_eq!(got.as_deref(), Some(want.as_path()));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[cfg(unix)]
    #[test]
    fn path_used_when_no_override() {
        let tmp = std::env::temp_dir().join(format!("vmforge-resolve-path-{}", std::process::id()));
        let path = tmp.join("path");
        let prefix = tmp.join("prefix");
        let want = write_fake_bin(&path, "qemu-img");
        write_fake_bin(&prefix, "qemu-img");

        let got = resolve_qemu_binary_with("qemu-img", None, vec![path], &[prefix]);
        assert_eq!(got.as_deref(), Some(want.as_path()));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[cfg(unix)]
    #[test]
    fn prefix_fallback_when_path_empty() {
        let tmp =
            std::env::temp_dir().join(format!("vmforge-resolve-prefix-{}", std::process::id()));
        let prefix = tmp.join("prefix");
        let want = write_fake_bin(&prefix, "qemu-system-aarch64");

        let got = resolve_qemu_binary_with("qemu-system-aarch64", None, Vec::new(), &[prefix]);
        assert_eq!(got.as_deref(), Some(want.as_path()));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[cfg(unix)]
    #[test]
    fn missing_binary_resolves_to_none() {
        let tmp =
            std::env::temp_dir().join(format!("vmforge-resolve-missing-{}", std::process::id()));
        let empty = tmp.join("empty");
        fs::create_dir_all(&empty).unwrap();
        let got = resolve_qemu_binary_with("qemu-img", None, vec![empty.clone()], &[empty]);
        assert_eq!(got, None);
        let _ = fs::remove_dir_all(&tmp);
    }

    /// On Windows, `candidate_in` must find `name.exe` when asked for the bare
    /// `name` (a constructed absolute path does NOT consult PATHEXT, so the
    /// resolver appends `.exe` itself). Tests the file-discovery branch without
    /// needing a runnable binary (no `--version` here).
    #[cfg(windows)]
    #[test]
    fn candidate_appends_exe_on_windows() {
        let tmp = std::env::temp_dir().join(format!("vmforge-resolve-exe-{}", std::process::id()));
        fs::create_dir_all(&tmp).unwrap();
        let exe = tmp.join("qemu-system-x86_64.exe");
        fs::write(&exe, b"MZ").unwrap();
        let got = candidate_in(&tmp, "qemu-system-x86_64");
        assert_eq!(got.as_deref(), Some(exe.as_path()));
        let _ = fs::remove_dir_all(&tmp);
    }

    #[test]
    fn windows_prefixes_nonempty_and_cover_known_roots() {
        // Pure env-derived; the MSYS2 statics are always present (env vars for
        // Program Files / scoop may be absent when run off-Windows).
        let dirs = windows_prefixes();
        assert!(!dirs.is_empty());
        let joined = dirs
            .iter()
            .map(|d| d.display().to_string())
            .collect::<Vec<_>>()
            .join(";");
        assert!(
            joined.contains("msys64"),
            "expected an MSYS2 entry: {joined}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn zero_byte_placeholder_rejected_by_version_gate() {
        let tmp = std::env::temp_dir().join(format!("vmforge-resolve-stub-{}", std::process::id()));
        let dir = tmp.join("dir");
        // A 0-byte (non-runnable) file must fail the --version check → None.
        write_empty_file(&dir, "qemu-img");
        let got = resolve_qemu_binary_with("qemu-img", None, vec![dir.clone()], &[]);
        assert_eq!(got, None);
        let _ = fs::remove_dir_all(&tmp);
    }
}
