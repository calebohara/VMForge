//! # vmforge-core — the engine
//!
//! This crate owns *all* virtualization logic for VMForge: the
//! [`Hypervisor`](hypervisor::Hypervisor) abstraction, the QEMU
//! implementation, the QMP control client, process supervision, disk and
//! network management, and the on-disk config model.
//!
//! ## Engine boundary (sacred)
//! The Tauri shell (`src-tauri`) and the React frontend reach this crate
//! **only** through Tauri IPC commands. Nothing above this boundary ever
//! shells out to QEMU directly. Keeping it this way is what makes the
//! optional macOS-native (Virtualization.framework) backend a drop-in
//! behind the same trait. See `CLAUDE.md`.
//!
//! ## Module ownership (Phase 1 build-out)
//! - [`host`]     host capability probe — *done* (Phase 0)
//! - [`model`]    domain types / config — storage-engineer (+ all)
//! - [`hypervisor`] the trait + QEMU impl — hypervisor-engineer
//! - `qmp`        QMP client — hypervisor-engineer (Phase 1)
//! - `process`    QEMU process supervisor — hypervisor-engineer (Phase 1)
//! - `storage`    qemu-img wrapper — storage-engineer (Phase 1/3)
//! - [`library`]  directory-scanned VM library store — storage-engineer (Phase 2)
//! - `network`    netdev model — network-engineer (Phase 1/4)

pub mod console;
pub mod error;
pub mod host;
pub mod hypervisor;
pub mod library;
pub mod model;
pub mod paths;
pub mod qemu;
pub mod storage;

pub use error::{Error, Result};
pub use hypervisor::Hypervisor;
pub use library::Library;
pub use qemu::QemuHypervisor;

/// Shared test helpers. `VMFORGE_QEMU_IMG` is process-global, so any test that
/// reads or mutates it must serialize through [`test_support::env_guard`].
///
/// The lock is an async-aware [`tokio::sync::Mutex`] so its guard may be held
/// across `.await` points (clippy's `await_holding_lock` only flags the std
/// `Mutex`).
#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::LazyLock;
    use tokio::sync::{Mutex, MutexGuard};

    static ENV_LOCK: LazyLock<Mutex<()>> = LazyLock::new(|| Mutex::new(()));

    /// Acquire the process-wide lock guarding `VMFORGE_QEMU_IMG`. Hold the guard
    /// for the duration of any test that depends on that env var.
    pub async fn env_guard() -> MutexGuard<'static, ()> {
        ENV_LOCK.lock().await
    }

    /// Write a mock `qemu-img` shell script that just creates (touches) the
    /// output file, point `VMFORGE_QEMU_IMG` at it, and return the lock guard.
    /// Hold the guard until the test that needs the mock finishes.
    pub async fn mock_qemu_img() -> MutexGuard<'static, ()> {
        let guard = env_guard().await;
        let dir = std::env::temp_dir().join("vmforge-mock-bin");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mock-qemu-img.sh");
        // args: create -f qcow2 <path> <size>G  → touch <path> ($4)
        std::fs::write(&path, "#!/bin/sh\n: > \"$4\"\nexit 0\n").unwrap();
        install_mock(&path);
        std::env::set_var("VMFORGE_QEMU_IMG", &path);
        guard
    }

    /// Body of a fuller mock `qemu-img` that emulates the subcommands Phase-3
    /// clone/snapshot tests exercise:
    /// - `create -f qcow2 <path> <size>`      → touch `<path>`
    /// - `create -f qcow2 --backing .. <child>` → touch `<child>` (last arg)
    /// - `convert -O qcow2 <src> <dst>`       → copy `<src>` to `<dst>` (deep)
    /// - `snapshot -c|-a|-d <tag> <disk>`     → succeed (no-op)
    /// - `info --output=json [-U] <disk>`     → print empty-snapshots JSON
    ///
    /// Pass `convert_exit` to force the convert branch to fail with that exit
    /// code WITHOUT writing the destination (to test the toml-last invariant).
    const MOCK_FULL_BODY: &str = r#"#!/bin/sh
sub="$1"
shift
case "$sub" in
  create)
    # last positional arg is the output image (works for plain + --backing)
    out=
    for a in "$@"; do out="$a"; done
    : > "$out"
    exit 0
    ;;
  convert)
    if [ -n "$MOCK_CONVERT_FAIL" ]; then
      echo "mock convert forced failure" 1>&2
      exit "$MOCK_CONVERT_FAIL"
    fi
    # convert -O qcow2 <src> <dst>: copy src -> dst (deep copy emulation)
    # args after shift: -O qcow2 <src> <dst>
    src="$3"
    dst="$4"
    cp "$src" "$dst"
    exit 0
    ;;
  snapshot)
    exit 0
    ;;
  info)
    echo '{"format":"qcow2","virtual-size":67108864}'
    exit 0
    ;;
  *)
    exit 0
    ;;
esac
"#;

    /// Like [`mock_qemu_img`] but installs the fuller mock ([`MOCK_FULL_BODY`])
    /// that handles convert/create-backing/snapshot/info — used by the Phase-3
    /// clone and snapshot tests.
    pub async fn mock_qemu_img_full() -> MutexGuard<'static, ()> {
        let guard = env_guard().await;
        let dir = std::env::temp_dir().join("vmforge-mock-bin");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("mock-qemu-img-full.sh");
        std::fs::write(&path, MOCK_FULL_BODY).unwrap();
        install_mock(&path);
        // Clear any forced-failure flag a prior test may have set.
        std::env::remove_var("MOCK_CONVERT_FAIL");
        std::env::set_var("VMFORGE_QEMU_IMG", &path);
        guard
    }

    /// chmod +x on unix; no-op elsewhere.
    fn install_mock(path: &std::path::Path) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(path, perms).unwrap();
        }
        #[cfg(not(unix))]
        let _ = path;
    }
}
