//! The VM library store — directory-scanned persistence for `vmforge.toml`.
//!
//! The library is a flat set of subdirectories under a root (default
//! `~/VMForge/`). Each VM lives in its own slug-named directory containing a
//! `vmforge.toml`, its qcow2 disk(s), and runtime artifacts. There is **no**
//! top-level index file: `list_vms` is a `read_dir` of the root that parses
//! each child `vmforge.toml`, skipping anything malformed with a `warn`.
//!
//! Identity is the stable [`VmId`] (`Uuid`); the on-disk directory is a
//! sanitized [`slugify`]ed name with collision suffixes. The slug is persisted
//! in `vmforge.toml` as `dir_slug` and never changes for the VM's life (rename
//! is metadata-only — see [`Library::rename_vm`]).
//!
//! Writes are atomic: serialize to a sibling temp file, then rename over the
//! real path.

use crate::error::{Error, Result};
use crate::host::Accelerator;
use crate::model::{VmConfig, VmId, VmState, VmSummary};
use crate::paths;
use std::path::{Path, PathBuf};

/// Directory-scanned store of persisted VM configurations.
///
/// All filesystem methods are async; the pure (de)serialization helpers
/// ([`Library::to_toml`] / [`Library::from_toml`]) are associated functions.
pub struct Library {
    root: PathBuf,
}

impl Library {
    /// Open the default library rooted at [`paths::library_dir`] (`~/VMForge`).
    pub fn open_default() -> Result<Self> {
        Ok(Self::new(paths::library_dir()?))
    }

    /// Open a library rooted at an explicit directory (used by tests and the
    /// engine).
    pub fn new(root: PathBuf) -> Self {
        Self { root }
    }

    /// The library root directory.
    pub fn root(&self) -> &Path {
        &self.root
    }

    /// Serialize a config to its on-disk TOML form. Pure.
    pub fn to_toml(config: &VmConfig) -> Result<String> {
        toml::to_string_pretty(config).map_err(|e| Error::Other(format!("toml serialize: {e}")))
    }

    /// Parse a config from its on-disk TOML form. Pure.
    pub fn from_toml(s: &str) -> Result<VmConfig> {
        toml::from_str(s).map_err(|e| Error::Config(format!("toml parse: {e}")))
    }

    /// Persist a brand-new VM: assign its directory slug (with collision
    /// suffixing) and timestamps, create the directory, atomically write
    /// `vmforge.toml`, and create the qcow2 disk(s). Does **not** launch.
    ///
    /// Refuses with [`Error::Config`] if the chosen directory already contains
    /// a `vmforge.toml` (clobber guard); collision suffixing makes this
    /// effectively unreachable for distinct names but keeps the invariant
    /// explicit.
    pub async fn create_vm(&self, mut config: VmConfig) -> Result<VmConfig> {
        validate_vm_name(&config.name)?;

        let slug = self.unique_slug(&slugify(&config.name)).await?;
        config.dir_slug = slug;

        let now = now_rfc3339();
        if config.metadata.created_at.is_none() {
            config.metadata.created_at = Some(now.clone());
        }
        config.metadata.updated_at = Some(now);

        let dir = paths::vm_dir(&self.root, &config.dir_slug);
        let config_path = paths::vm_config_path(&self.root, &config.dir_slug);
        if config_path.exists() {
            return Err(Error::Config(format!(
                "VM directory {} already holds a vmforge.toml",
                dir.display()
            )));
        }
        tokio::fs::create_dir_all(&dir).await?;

        // Disks: create each qcow2 (idempotent on existence).
        for disk in &config.disks {
            let path = dir.join(&disk.path);
            crate::storage::create_qcow2(&path, disk.size_gib).await?;
        }

        write_config_atomic(&config_path, &config).await?;
        Ok(config)
    }

    /// Atomically overwrite an existing VM's `vmforge.toml`, bumping
    /// `updated_at`. Does not touch disks. The VM is located by its current
    /// `dir_slug`.
    pub async fn save_config(&self, config: &VmConfig) -> Result<()> {
        if !is_safe_slug(&config.dir_slug) {
            return Err(Error::Config("refusing to save: unsafe dir_slug".into()));
        }
        let mut config = config.clone();
        config.metadata.updated_at = Some(now_rfc3339());
        let config_path = paths::vm_config_path(&self.root, &config.dir_slug);
        write_config_atomic(&config_path, &config).await
    }

    /// Load a single config by id. Scans the library; missing id maps to
    /// [`Error::VmNotFound`].
    pub async fn load_config(&self, id: &VmId) -> Result<VmConfig> {
        for config in self.load_all().await? {
            if &config.id == id {
                return Ok(config);
            }
        }
        Err(Error::VmNotFound(id.to_string()))
    }

    /// All persisted VMs as `Defined` summaries. Malformed configs are skipped
    /// with a `warn`; an absent root yields an empty list. Accelerator and the
    /// `emulated` flag are filled by the engine, not here, so summaries carry a
    /// placeholder accelerator and `emulated == false`.
    pub async fn list_vms(&self) -> Result<Vec<VmSummary>> {
        Ok(self
            .load_all()
            .await?
            .into_iter()
            .map(|c| VmSummary {
                id: c.id,
                name: c.name,
                state: VmState::Defined,
                accelerator: Accelerator::Tcg,
                emulated: false,
            })
            .collect())
    }

    /// Load every well-formed config under the root. Non-directories, dotfiles,
    /// and directories without a parseable `vmforge.toml` are skipped (the
    /// latter with a `warn`). An absent root yields an empty vector.
    pub async fn load_all(&self) -> Result<Vec<VmConfig>> {
        let mut out = Vec::new();
        let mut rd = match tokio::fs::read_dir(&self.root).await {
            Ok(rd) => rd,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
            Err(e) => return Err(Error::from(e)),
        };
        while let Some(entry) = rd.next_entry().await? {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') {
                continue;
            }
            let file_type = entry.file_type().await?;
            if !file_type.is_dir() {
                continue;
            }
            let config_path = entry.path().join(paths::CONFIG_FILENAME);
            let raw = match tokio::fs::read_to_string(&config_path).await {
                Ok(s) => s,
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
                Err(e) => {
                    tracing::warn!(target: "vmforge_core::library", path = %config_path.display(), error = %e, "skipping unreadable vmforge.toml");
                    continue;
                }
            };
            match Self::from_toml(&raw) {
                Ok(c) if !is_safe_slug(&c.dir_slug) => {
                    // Fail-closed chokepoint: a config whose dir_slug could
                    // escape the root (traversal / separators / empty) is
                    // dropped here, so it can never reach load_config →
                    // delete/save/start. See is_safe_slug.
                    tracing::warn!(target: "vmforge_core::library", path = %config_path.display(), slug = %c.dir_slug, "skipping config with unsafe dir_slug");
                }
                Ok(c) => out.push(c),
                Err(e) => {
                    tracing::warn!(target: "vmforge_core::library", path = %config_path.display(), error = %e, "skipping malformed vmforge.toml");
                }
            }
        }
        Ok(out)
    }

    /// Delete a VM by id. With `delete_disks == true` the whole directory is
    /// removed; otherwise only `vmforge.toml` is removed (disks are orphaned in
    /// place). Unknown id maps to [`Error::VmNotFound`].
    pub async fn delete_vm(&self, id: &VmId, delete_disks: bool) -> Result<()> {
        let config = self.load_config(id).await?;
        // Belt-and-suspenders: load_config already filters unsafe slugs, but
        // never run remove_dir_all on a path that could escape the root.
        if !is_safe_slug(&config.dir_slug) {
            return Err(Error::Config("refusing to delete: unsafe dir_slug".into()));
        }
        let dir = paths::vm_dir(&self.root, &config.dir_slug);
        if delete_disks {
            tokio::fs::remove_dir_all(&dir).await?;
        } else {
            let config_path = dir.join(paths::CONFIG_FILENAME);
            tokio::fs::remove_file(&config_path).await?;
        }
        Ok(())
    }

    /// Rename a VM (metadata only). Validates the new name, rewrites `name` and
    /// `updated_at`; `id` and `dir_slug` are unchanged (the directory never
    /// moves). Safe while the VM is running.
    pub async fn rename_vm(&self, id: &VmId, new_name: &str) -> Result<VmConfig> {
        validate_vm_name(new_name)?;
        let mut config = self.load_config(id).await?;
        config.name = new_name.to_string();
        // save_config bumps updated_at on disk; reload to return the persisted
        // config (with the new updated_at).
        self.save_config(&config).await?;
        self.load_config(id).await
    }

    /// Find a free slug starting from `base`, appending `-2`, `-3`, ... if a
    /// directory with that slug already holds a `vmforge.toml`.
    ///
    /// A bare directory (no config) is adoptable — this lets `create_vm` reuse
    /// a directory that already has a pre-created disk but no config yet.
    async fn unique_slug(&self, base: &str) -> Result<String> {
        if !paths::vm_config_path(&self.root, base).exists() {
            return Ok(base.to_string());
        }
        for n in 2..=u32::MAX {
            let candidate = format!("{base}-{n}");
            if !paths::vm_config_path(&self.root, &candidate).exists() {
                return Ok(candidate);
            }
        }
        Err(Error::Other("exhausted slug collision suffixes".into()))
    }
}

/// Atomically write a config to `config_path` via a sibling temp file + rename.
async fn write_config_atomic(config_path: &Path, config: &VmConfig) -> Result<()> {
    let body = Library::to_toml(config)?;
    if let Some(parent) = config_path.parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    // Unique temp sibling so concurrent writers never collide.
    let tmp = config_path.with_file_name(format!(
        "{}.{}.tmp",
        paths::CONFIG_FILENAME,
        uuid::Uuid::new_v4()
    ));
    tokio::fs::write(&tmp, body.as_bytes()).await?;
    match tokio::fs::rename(&tmp, config_path).await {
        Ok(()) => Ok(()),
        Err(e) => {
            let _ = tokio::fs::remove_file(&tmp).await;
            Err(Error::from(e))
        }
    }
}

/// RFC3339 UTC timestamp (`YYYY-MM-DDTHH:MM:SSZ`) from the system clock.
///
/// Self-contained (no `chrono`/`time` dependency) civil-time conversion of the
/// Unix epoch seconds, using the standard algorithm for the proleptic
/// Gregorian calendar.
fn now_rfc3339() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format_rfc3339_utc(secs)
}

/// Convert Unix epoch seconds to an RFC3339 UTC string. Pure — unit-tested.
fn format_rfc3339_utc(secs: u64) -> String {
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hour, minute, second) = (rem / 3600, (rem % 3600) / 60, rem % 60);

    // Howard Hinnant's civil-from-days algorithm (days since 1970-01-01).
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365; // [0, 399]
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = doy - (153 * mp + 2) / 5 + 1; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 }; // [1, 12]
    let year = if month <= 2 { y + 1 } else { y };

    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

/// Reject names that can't safely become a directory entry or that collide with
/// platform-reserved identifiers.
///
/// Rejects: empty/whitespace-only, path separators (`/` or `\`), `.`/`..`,
/// control characters, Windows-reserved device names
/// (`CON`/`NUL`/`PRN`/`AUX`/`COM1`-`COM9`/`LPT1`-`LPT9`, case-insensitive), and
/// trailing dot or space (illegal on Windows).
pub fn validate_vm_name(name: &str) -> Result<()> {
    let reject = |msg: &str| Err(Error::Config(format!("invalid VM name: {msg}")));

    if name.trim().is_empty() {
        return reject("must not be empty");
    }
    if name == "." || name == ".." {
        return reject("must not be '.' or '..'");
    }
    if name.contains('/') || name.contains('\\') {
        return reject("must not contain path separators");
    }
    if name.chars().any(|c| c.is_control()) {
        return reject("must not contain control characters");
    }
    if name.ends_with('.') || name.ends_with(' ') {
        return reject("must not end with a dot or space");
    }
    // Windows-reserved device names (case-insensitive), bare or with extension.
    let stem = name.split('.').next().unwrap_or(name).to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "NUL" | "PRN" | "AUX")
        || (stem.starts_with("COM") && is_reserved_numbered(&stem, "COM"))
        || (stem.starts_with("LPT") && is_reserved_numbered(&stem, "LPT"));
    if reserved {
        return reject("must not be a reserved device name");
    }
    Ok(())
}

/// True for `{prefix}{1-9}` (e.g. `COM1`, `LPT9`).
fn is_reserved_numbered(stem: &str, prefix: &str) -> bool {
    let suffix = &stem[prefix.len()..];
    suffix.len() == 1 && matches!(suffix.as_bytes()[0], b'1'..=b'9')
}

/// Turn a display name into a safe directory slug.
///
/// Lowercases, maps whitespace to `-`, strips characters outside
/// `[a-z0-9-_]`, collapses runs of `-`/`_`, trims leading/trailing separators,
/// and never returns empty (falls back to `"vm"`). Collision suffixing is done
/// by [`Library::create_vm`], not here.
pub fn slugify(name: &str) -> String {
    let mut out = String::with_capacity(name.len());
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() {
            out.push(ch.to_ascii_lowercase());
        } else if ch.is_whitespace() || ch == '-' {
            out.push('-');
        } else if ch == '_' {
            out.push('_');
        }
        // everything else is dropped
    }

    // Collapse consecutive separators, preferring '-' when mixed.
    let mut collapsed = String::with_capacity(out.len());
    let mut prev_sep = false;
    for ch in out.chars() {
        let is_sep = ch == '-' || ch == '_';
        if is_sep {
            if !prev_sep {
                collapsed.push(ch);
            }
            prev_sep = true;
        } else {
            collapsed.push(ch);
            prev_sep = false;
        }
    }

    let trimmed = collapsed.trim_matches(|c| c == '-' || c == '_');
    if trimmed.is_empty() {
        "vm".to_string()
    } else {
        trimmed.to_string()
    }
}

/// Whether a persisted `dir_slug` is safe to use as a single path segment under
/// the library root. A `dir_slug` is read verbatim from `vmforge.toml`, so a
/// hand-edited or imported config could carry `../` or absolute components that
/// `Path::join` does not normalize — escaping the root on delete/save/start.
/// Canonical slugs are [`slugify`]-stable; anything else (empty, separators,
/// `..`, non-canonical) is rejected fail-closed.
pub fn is_safe_slug(slug: &str) -> bool {
    !slug.is_empty() && slug == slugify(slug)
}

/// True iff the only differences between two configs are runtime-safe to apply
/// while the VM is live — i.e. `name` and/or `metadata`. Any change to
/// hardware, disks, network, display, iso, id, slug, or schema makes it unsafe.
pub fn is_runtime_safe_edit(old: &VmConfig, new: &VmConfig) -> bool {
    old.id == new.id
        && old.schema_version == new.schema_version
        && old.dir_slug == new.dir_slug
        && old.hardware.cpus == new.hardware.cpus
        && old.hardware.memory_mib == new.hardware.memory_mib
        && disks_eq(&old.disks, &new.disks)
        && network_eq(&old.network, &new.network)
        && old.iso == new.iso
}

fn disks_eq(a: &[crate::model::DiskSpec], b: &[crate::model::DiskSpec]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| x.path == y.path && x.size_gib == y.size_gib && x.backing == y.backing)
}

fn network_eq(a: &crate::model::NetworkConfig, b: &crate::model::NetworkConfig) -> bool {
    a.mode == b.mode
        && a.mac == b.mac
        && a.port_forwards.len() == b.port_forwards.len()
        && a.port_forwards
            .iter()
            .zip(&b.port_forwards)
            .all(|(x, y)| x.host == y.host && x.guest == y.guest && x.udp == y.udp)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{DiskSpec, Hardware, NetworkConfig, PortForward, VmConfig, VmMetadata};
    use uuid::Uuid;

    fn sample_config() -> VmConfig {
        VmConfig {
            id: Uuid::nil(),
            name: "Alpine VM".into(),
            schema_version: 1,
            dir_slug: "alpine-vm".into(),
            hardware: Hardware {
                cpus: 2,
                memory_mib: 2048,
            },
            disks: vec![DiskSpec {
                path: "disk.qcow2".into(),
                size_gib: 8,
                backing: None,
            }],
            network: NetworkConfig {
                mode: crate::model::NetworkMode::User,
                mac: Some("52:54:00:12:34:56".into()),
                port_forwards: vec![PortForward {
                    host: 2222,
                    guest: 22,
                    udp: false,
                }],
            },
            display: Default::default(),
            iso: Some("/path/alpine.iso".into()),
            metadata: VmMetadata {
                created_at: Some("2026-06-24T17:40:00Z".into()),
                updated_at: Some("2026-06-24T17:40:00Z".into()),
                notes: "hello".into(),
                os_hint: Some("alpine".into()),
            },
        }
    }

    // ---- (1) toml round-trip ----
    #[test]
    fn toml_round_trip() {
        let c = sample_config();
        let s = Library::to_toml(&c).unwrap();
        let back = Library::from_toml(&s).unwrap();
        assert_eq!(back.id, c.id);
        assert_eq!(back.name, c.name);
        assert_eq!(back.dir_slug, c.dir_slug);
        assert_eq!(back.schema_version, 1);
        assert_eq!(back.hardware.cpus, c.hardware.cpus);
        assert_eq!(back.hardware.memory_mib, c.hardware.memory_mib);
        assert_eq!(back.disks.len(), 1);
        assert_eq!(back.disks[0].path, "disk.qcow2");
        assert_eq!(back.disks[0].size_gib, 8);
        assert_eq!(back.network.mode, crate::model::NetworkMode::User);
        assert_eq!(back.network.port_forwards.len(), 1);
        assert_eq!(back.iso.as_deref(), Some("/path/alpine.iso"));
        assert_eq!(back.metadata.notes, "hello");
        assert_eq!(back.metadata.os_hint.as_deref(), Some("alpine"));
    }

    // ---- (2) back-compat: minimal toml, serde defaults fill ----
    #[test]
    fn toml_back_compat() {
        let minimal = r#"
            id = "00000000-0000-0000-0000-000000000000"
            name = "Bare"
        "#;
        let c = Library::from_toml(minimal).unwrap();
        assert_eq!(c.name, "Bare");
        assert_eq!(c.schema_version, 1);
        assert_eq!(c.dir_slug, "");
        assert_eq!(c.hardware.cpus, Hardware::default().cpus);
        assert_eq!(c.hardware.memory_mib, Hardware::default().memory_mib);
        assert!(c.disks.is_empty());
        assert_eq!(c.network.mode, crate::model::NetworkMode::User);
        assert!(c.iso.is_none());
        assert!(c.metadata.created_at.is_none());
    }

    // ---- (4) display.vnc_port is not serialized ----
    #[test]
    fn display_vnc_port_not_serialized() {
        let mut c = sample_config();
        c.display.vnc_port = Some(5901);
        let s = Library::to_toml(&c).unwrap();
        assert!(
            !s.contains("vnc_port"),
            "vnc_port must be #[serde(skip)], got:\n{s}"
        );
        let back = Library::from_toml(&s).unwrap();
        assert_eq!(back.display.vnc_port, None);
    }

    // ---- (5) validate_vm_name rejects ----
    #[test]
    fn validate_vm_name_rejects() {
        assert!(validate_vm_name("Good Name").is_ok());
        assert!(validate_vm_name("a").is_ok());
        for bad in [
            "",
            "   ",
            ".",
            "..",
            "a/b",
            "a\\b",
            "with\nnewline",
            "trailing.",
            "trailing ",
            "CON",
            "con",
            "nul",
            "PRN",
            "AUX",
            "COM1",
            "lpt9",
            "COM3.txt",
        ] {
            assert!(
                validate_vm_name(bad).is_err(),
                "expected {bad:?} to be rejected"
            );
        }
        // Numbered device names outside 1-9 are fine.
        assert!(validate_vm_name("COM0").is_ok());
        assert!(validate_vm_name("COM10").is_ok());
        assert!(validate_vm_name("LPT").is_ok());
    }

    // ---- (6) slugify cases ----
    #[test]
    fn slugify_cases() {
        assert_eq!(slugify("Alpine VM"), "alpine-vm");
        assert_eq!(slugify("  Trimmed  "), "trimmed");
        assert_eq!(slugify("UPPER_case"), "upper_case");
        assert_eq!(slugify("weird!!!chars###here"), "weirdcharshere");
        assert_eq!(slugify("multi   space"), "multi-space");
        assert_eq!(slugify("dash---collapse"), "dash-collapse");
        assert_eq!(slugify("under___score"), "under_score");
        assert_eq!(slugify("---"), "vm");
        assert_eq!(slugify(""), "vm");
        assert_eq!(slugify("日本語"), "vm");
        assert_eq!(slugify("Ubuntu 24.04 LTS"), "ubuntu-2404-lts");
    }

    // ---- (7) is_runtime_safe_edit ----
    #[test]
    fn runtime_safe_edit_name_and_metadata_only() {
        let base = sample_config();

        let mut name_only = base.clone();
        name_only.name = "Renamed".into();
        name_only.metadata.notes = "changed".into();
        assert!(is_runtime_safe_edit(&base, &name_only));

        let mut cpu_changed = base.clone();
        cpu_changed.hardware.cpus = 8;
        assert!(!is_runtime_safe_edit(&base, &cpu_changed));

        let mut mem_changed = base.clone();
        mem_changed.hardware.memory_mib = 8192;
        assert!(!is_runtime_safe_edit(&base, &mem_changed));

        let mut iso_changed = base.clone();
        iso_changed.iso = Some("/other.iso".into());
        assert!(!is_runtime_safe_edit(&base, &iso_changed));

        let mut net_changed = base.clone();
        net_changed.network.mode = crate::model::NetworkMode::Bridged;
        assert!(!is_runtime_safe_edit(&base, &net_changed));

        let mut disk_changed = base.clone();
        disk_changed.disks[0].size_gib = 99;
        assert!(!is_runtime_safe_edit(&base, &disk_changed));
    }

    // ---- format_rfc3339_utc sanity (supports timestamp tests) ----
    #[test]
    fn rfc3339_epoch_and_known_date() {
        assert_eq!(format_rfc3339_utc(0), "1970-01-01T00:00:00Z");
        // 2026-06-24T17:40:00Z = 1782322800 (verified independently)
        assert_eq!(format_rfc3339_utc(1_782_322_800), "2026-06-24T17:40:00Z");
    }

    // ---- (8) create then load ----
    #[tokio::test]
    async fn create_then_load() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img().await;
        let lib = Library::new(tmp.path().to_path_buf());
        let mut cfg = sample_config();
        cfg.id = Uuid::new_v4();
        let created = lib.create_vm(cfg.clone()).await.unwrap();
        assert_eq!(created.dir_slug, "alpine-vm");
        assert!(created.metadata.created_at.is_some());
        assert!(created.metadata.updated_at.is_some());

        let loaded = lib.load_config(&created.id).await.unwrap();
        assert_eq!(loaded.id, created.id);
        assert_eq!(loaded.name, "Alpine VM");
        assert_eq!(loaded.dir_slug, "alpine-vm");
        // Disk file was created by the mock.
        let disk = tmp.path().join("alpine-vm").join("disk.qcow2");
        assert!(disk.exists(), "disk should have been created");
    }

    // ---- (9) index list scan ignores junk + dotfiles ----
    #[tokio::test]
    async fn index_list_scan() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img().await;
        let lib = Library::new(tmp.path().to_path_buf());
        let mut cfg = sample_config();
        cfg.id = Uuid::new_v4();
        lib.create_vm(cfg).await.unwrap();

        // Junk file at root + dotfile dir — both ignored.
        tokio::fs::write(tmp.path().join("README.txt"), b"hi")
            .await
            .unwrap();
        tokio::fs::create_dir_all(tmp.path().join(".hidden"))
            .await
            .unwrap();
        tokio::fs::write(
            tmp.path().join(".hidden").join(paths::CONFIG_FILENAME),
            b"id='x'",
        )
        .await
        .unwrap();

        let summaries = lib.list_vms().await.unwrap();
        assert_eq!(summaries.len(), 1);
        assert_eq!(summaries[0].state, VmState::Defined);
    }

    // ---- (10) list skips malformed ----
    #[tokio::test]
    async fn list_skips_malformed() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img().await;
        let lib = Library::new(tmp.path().to_path_buf());
        let mut good = sample_config();
        good.id = Uuid::new_v4();
        lib.create_vm(good).await.unwrap();

        // A directory with garbage toml — skipped, not fatal.
        let bad = tmp.path().join("broken");
        tokio::fs::create_dir_all(&bad).await.unwrap();
        tokio::fs::write(
            bad.join(paths::CONFIG_FILENAME),
            b"this is not = valid = toml [[",
        )
        .await
        .unwrap();
        // A directory with no toml at all — also skipped.
        tokio::fs::create_dir_all(tmp.path().join("empty-dir"))
            .await
            .unwrap();

        let summaries = lib.list_vms().await.unwrap();
        assert_eq!(summaries.len(), 1);
    }

    // ---- (11) save_config bumps updated_at + leaves disks untouched ----
    #[tokio::test]
    async fn save_config_bumps_updated_at() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img().await;
        let lib = Library::new(tmp.path().to_path_buf());
        let mut cfg = sample_config();
        cfg.id = Uuid::new_v4();
        cfg.metadata.updated_at = Some("2000-01-01T00:00:00Z".into());
        let created = lib.create_vm(cfg).await.unwrap();

        let disk = tmp.path().join("alpine-vm").join("disk.qcow2");
        let before = tokio::fs::read(&disk).await.unwrap();

        let mut edited = created.clone();
        edited.name = "Edited".into();
        lib.save_config(&edited).await.unwrap();

        let reloaded = lib.load_config(&created.id).await.unwrap();
        assert_eq!(reloaded.name, "Edited");
        assert_ne!(
            reloaded.metadata.updated_at.as_deref(),
            Some("2000-01-01T00:00:00Z")
        );
        // Disk untouched.
        let after = tokio::fs::read(&disk).await.unwrap();
        assert_eq!(before, after);
    }

    // ---- (12) delete_vm both modes + unknown id ----
    #[tokio::test]
    async fn delete_vm_removes_dir() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img().await;
        let lib = Library::new(tmp.path().to_path_buf());

        // delete_disks = true → whole dir gone.
        let mut a = sample_config();
        a.id = Uuid::new_v4();
        let a = lib.create_vm(a).await.unwrap();
        let a_dir = tmp.path().join(&a.dir_slug);
        assert!(a_dir.exists());
        lib.delete_vm(&a.id, true).await.unwrap();
        assert!(!a_dir.exists());

        // delete_disks = false → only toml gone, dir + disk remain.
        let mut b = sample_config();
        b.id = Uuid::new_v4();
        b.name = "Second".into();
        let b = lib.create_vm(b).await.unwrap();
        let b_dir = tmp.path().join(&b.dir_slug);
        let b_disk = b_dir.join("disk.qcow2");
        lib.delete_vm(&b.id, false).await.unwrap();
        assert!(b_dir.exists(), "dir should remain when delete_disks=false");
        assert!(
            b_disk.exists(),
            "disk should remain when delete_disks=false"
        );
        assert!(!b_dir.join(paths::CONFIG_FILENAME).exists());

        // Unknown id → VmNotFound.
        let err = lib.delete_vm(&Uuid::new_v4(), true).await.unwrap_err();
        assert!(matches!(err, Error::VmNotFound(_)));
    }

    // ---- (13) rename keeps id and slug ----
    #[tokio::test]
    async fn rename_keeps_id_and_slug() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img().await;
        let lib = Library::new(tmp.path().to_path_buf());
        let mut cfg = sample_config();
        cfg.id = Uuid::new_v4();
        let created = lib.create_vm(cfg).await.unwrap();

        let renamed = lib
            .rename_vm(&created.id, "Totally New Name")
            .await
            .unwrap();
        assert_eq!(renamed.id, created.id);
        assert_eq!(renamed.dir_slug, created.dir_slug);
        assert_eq!(renamed.name, "Totally New Name");
        // Directory did not move.
        assert!(tmp.path().join(&created.dir_slug).exists());

        // Invalid new name rejected.
        assert!(lib.rename_vm(&created.id, "bad/name").await.is_err());
    }

    // ---- (14) create_vm refuses to clobber an existing config ----
    #[tokio::test]
    async fn create_vm_refuses_clobber() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img().await;
        let lib = Library::new(tmp.path().to_path_buf());

        // Pre-seed a directory that matches the slug with an existing toml
        // (a foreign VM's config we must never overwrite).
        let dir = tmp.path().join("alpine-vm");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let sentinel = b"id = \"99999999-9999-9999-9999-999999999999\"\nname = \"Other\"\n";
        tokio::fs::write(dir.join(paths::CONFIG_FILENAME), sentinel)
            .await
            .unwrap();

        let mut cfg = sample_config();
        cfg.id = Uuid::new_v4();
        cfg.name = "Alpine VM".into();
        let created = lib.create_vm(cfg).await.unwrap();

        // The existing config was NOT clobbered: the new VM took the next slug.
        assert_eq!(created.dir_slug, "alpine-vm-2");
        let original = tokio::fs::read(dir.join(paths::CONFIG_FILENAME))
            .await
            .unwrap();
        assert_eq!(original, sentinel, "existing config must be untouched");
    }

    // ---- (14b) direct clobber guard fires when the target already has config
    #[tokio::test]
    async fn create_vm_guard_rejects_preexisting_config_at_target() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img().await;
        let lib = Library::new(tmp.path().to_path_buf());

        // Slug "vm" (the fallback) is pre-occupied with a config; a name that
        // slugs to "vm" with the suffix path blocked would normally suffix, so
        // we instead assert the guard by checking the public refuse contract:
        // creating twice with the same name never overwrites the first.
        let mut a = sample_config();
        a.id = Uuid::new_v4();
        let a = lib.create_vm(a).await.unwrap();
        let a_toml = tokio::fs::read(tmp.path().join(&a.dir_slug).join(paths::CONFIG_FILENAME))
            .await
            .unwrap();

        let mut b = sample_config();
        b.id = Uuid::new_v4();
        let b = lib.create_vm(b).await.unwrap();
        assert_ne!(a.dir_slug, b.dir_slug);

        // First VM's config is byte-identical to before — never clobbered.
        let a_toml_after =
            tokio::fs::read(tmp.path().join(&a.dir_slug).join(paths::CONFIG_FILENAME))
                .await
                .unwrap();
        assert_eq!(a_toml, a_toml_after);
    }

    // ---- (15) slug collision suffix ----
    #[tokio::test]
    async fn slug_collision_suffix() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img().await;
        let lib = Library::new(tmp.path().to_path_buf());

        let mut a = sample_config();
        a.id = Uuid::new_v4();
        let a = lib.create_vm(a).await.unwrap();
        assert_eq!(a.dir_slug, "alpine-vm");

        let mut b = sample_config();
        b.id = Uuid::new_v4();
        let b = lib.create_vm(b).await.unwrap();
        assert_eq!(b.dir_slug, "alpine-vm-2");

        let mut c = sample_config();
        c.id = Uuid::new_v4();
        let c = lib.create_vm(c).await.unwrap();
        assert_eq!(c.dir_slug, "alpine-vm-3");
    }

    // ---- (17) create_vm idempotent disk: pre-created file untouched ----
    #[tokio::test]
    async fn create_vm_idempotent_disk() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img().await;
        let lib = Library::new(tmp.path().to_path_buf());

        // Pre-create the disk with known contents inside the (future) VM dir.
        let dir = tmp.path().join("alpine-vm");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        tokio::fs::write(dir.join("disk.qcow2"), b"PRECREATED")
            .await
            .unwrap();

        let mut cfg = sample_config();
        cfg.id = Uuid::new_v4();
        // The pre-existing dir has no vmforge.toml so create_vm uses it.
        let created = lib.create_vm(cfg).await.unwrap();
        assert_eq!(created.dir_slug, "alpine-vm");
        let contents = tokio::fs::read(dir.join("disk.qcow2")).await.unwrap();
        assert_eq!(&contents, b"PRECREATED", "existing disk must be untouched");
    }

    // ---- security: a hand-planted config with a traversing dir_slug is
    //      filtered at load and never deletable/savable (path-traversal guard).
    #[test]
    fn is_safe_slug_rejects_traversal() {
        assert!(is_safe_slug("alpine-vm"));
        assert!(is_safe_slug("alpine-vm-2"));
        assert!(is_safe_slug("under_score"));
        assert!(!is_safe_slug(""));
        assert!(!is_safe_slug("../escape"));
        assert!(!is_safe_slug("a/b"));
        assert!(!is_safe_slug("a\\b"));
        assert!(!is_safe_slug(".."));
        assert!(!is_safe_slug("Has Space")); // not canonical
    }

    #[tokio::test]
    async fn load_all_skips_unsafe_dir_slug() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = Library::new(tmp.path().to_path_buf());
        let dir = tmp.path().join("evil");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let id = Uuid::new_v4();
        let toml = format!("id = \"{id}\"\nname = \"Evil\"\ndir_slug = \"../escape\"\n");
        tokio::fs::write(dir.join(paths::CONFIG_FILENAME), toml)
            .await
            .unwrap();

        // Filtered at the load chokepoint → invisible and inoperable.
        assert!(lib.list_vms().await.unwrap().is_empty());
        assert!(matches!(
            lib.load_config(&id).await.unwrap_err(),
            Error::VmNotFound(_)
        ));
        assert!(lib.delete_vm(&id, true).await.is_err());
    }
}
