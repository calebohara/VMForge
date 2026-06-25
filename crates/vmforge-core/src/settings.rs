//! Persisted user settings (Phase 6 — D3 user override).
//!
//! A tiny JSON file in the app config dir holding cross-session preferences.
//! Today it carries exactly one field: `qemu_dir`, the user-chosen directory
//! containing the QEMU binaries, written by the first-run gate's "Locate QEMU…"
//! picker and consumed by [`crate::qemu_resolve::resolve_qemu_binary`].
//!
//! Reads are best-effort: a missing or malformed file means "no override",
//! never an error — the resolver simply falls through to `$PATH` and the
//! hardcoded prefixes. The on-disk location can be redirected for tests via the
//! `VMFORGE_CONFIG_DIR` env var.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// Env var that redirects the settings directory (test seam; also lets a host
/// override the config location without touching `$HOME`).
const CONFIG_DIR_ENV: &str = "VMFORGE_CONFIG_DIR";

const SETTINGS_FILENAME: &str = "settings.json";

/// Cross-session user settings persisted as JSON.
#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct Settings {
    /// User-chosen directory containing the QEMU binaries (the "Locate QEMU…"
    /// override). `None` when the user has not pinned a location.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qemu_dir: Option<PathBuf>,
}

/// Directory holding the settings file. `$VMFORGE_CONFIG_DIR` if set, otherwise
/// the platform app-config dir from `ProjectDirs::from("com","vmforge","VMForge")`
/// (macOS `~/Library/Application Support/com.vmforge.VMForge`; Linux
/// `~/.config/vmforge`; Windows `%APPDATA%\vmforge\VMForge\config`). `None` if
/// neither can be resolved.
pub fn config_dir() -> Option<PathBuf> {
    if let Some(dir) = std::env::var_os(CONFIG_DIR_ENV).filter(|v| !v.is_empty()) {
        return Some(PathBuf::from(dir));
    }
    directories::ProjectDirs::from("com", "vmforge", "VMForge")
        .map(|p| p.config_dir().to_path_buf())
}

fn settings_path() -> Option<PathBuf> {
    config_dir().map(|d| d.join(SETTINGS_FILENAME))
}

/// Load persisted settings. Missing/malformed file → defaults (never errors).
pub fn load() -> Settings {
    let Some(path) = settings_path() else {
        return Settings::default();
    };
    match std::fs::read_to_string(&path) {
        Ok(s) => serde_json::from_str(&s).unwrap_or_default(),
        Err(_) => Settings::default(),
    }
}

/// Persist settings as pretty JSON, creating the config dir if needed.
pub fn save(settings: &Settings) -> crate::error::Result<()> {
    let path = settings_path().ok_or_else(|| crate::error::Error::Other("no config dir".into()))?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let json = serde_json::to_string_pretty(settings)
        .map_err(|e| crate::error::Error::Other(format!("serialize settings: {e}")))?;
    std::fs::write(&path, json)?;
    Ok(())
}

/// The persisted QEMU directory override, if any. Convenience wrapper consumed
/// by the resolver.
pub fn qemu_dir_override() -> Option<PathBuf> {
    load().qemu_dir.filter(|p| !p.as_os_str().is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Round-trip through a temp config dir via the `VMFORGE_CONFIG_DIR` seam.
    /// Serialized to avoid clobbering the process-global env between tests.
    #[test]
    fn save_then_load_round_trips_qemu_dir() {
        let tmp = std::env::temp_dir().join(format!("vmforge-settings-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::set_var(CONFIG_DIR_ENV, &tmp);

        // Empty by default.
        assert!(load().qemu_dir.is_none());

        let want = PathBuf::from("/opt/homebrew/bin");
        save(&Settings {
            qemu_dir: Some(want.clone()),
        })
        .unwrap();

        assert_eq!(load().qemu_dir, Some(want.clone()));
        assert_eq!(qemu_dir_override(), Some(want));

        std::env::remove_var(CONFIG_DIR_ENV);
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
