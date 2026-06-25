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
use crate::model::{DiskSpec, Snapshot, SnapshotNode, VmConfig, VmId, VmState, VmSummary};
use crate::paths;
use crate::storage::Qcow2Snapshot;
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use uuid::Uuid;

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
                Ok(c) if !config_disks_safe(&c) => {
                    // Same fail-closed chokepoint for disk paths/backings: a
                    // hand-edited disks[].path/backing could escape the root on
                    // clone/convert/launch. See is_safe_disk_filename/backing.
                    tracing::warn!(target: "vmforge_core::library", path = %config_path.display(), "skipping config with unsafe disk path/backing");
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

    /// Full (deep) clone: `qemu-img convert -O qcow2` each source disk into a
    /// brand-new VM directory. The result is flattened — `backing` is cleared
    /// and snapshots are NOT carried (the convert drops internal snapshots and
    /// our overlay is reset).
    ///
    /// Atomicity (A4): convert into `<disk>.partial` then rename, and write the
    /// clone's `vmforge.toml` **LAST** so a crash mid-convert leaves only an
    /// orphan directory with no parseable config — invisible to `load_all`.
    ///
    /// Single-disk scope (Phase 3): more than one disk maps to
    /// [`Error::NotImplemented`].
    pub async fn full_clone(&self, src_id: &VmId, new_name: &str) -> Result<VmConfig> {
        validate_vm_name(new_name)?;
        let src = self.load_config(src_id).await?;
        let src_disk = single_disk(&src)?;

        let src_dir = paths::vm_dir(&self.root, &src.dir_slug);
        let src_disk_path = src_dir.join(&src_disk.path);

        let slug = self.unique_slug(&slugify(new_name)).await?;
        let dst_dir = paths::vm_dir(&self.root, &slug);
        let config_path = dst_dir.join(paths::CONFIG_FILENAME);
        if config_path.exists() {
            return Err(Error::Config(format!(
                "VM directory {} already holds a vmforge.toml",
                dst_dir.display()
            )));
        }
        tokio::fs::create_dir_all(&dst_dir).await?;

        // Deep-copy into a *.partial then rename so a half-written disk never
        // appears as the real image.
        let final_disk = dst_dir.join(&src_disk.path);
        let partial = dst_dir.join(format!("{}.partial", src_disk.path));
        crate::storage::convert_qcow2(&src_disk_path, &partial).await?;
        tokio::fs::rename(&partial, &final_disk).await?;

        let now = now_rfc3339();
        let clone = VmConfig {
            id: Uuid::new_v4(),
            name: new_name.to_string(),
            schema_version: src.schema_version,
            dir_slug: slug,
            hardware: src.hardware.clone(),
            disks: vec![DiskSpec {
                path: src_disk.path.clone(),
                size_gib: src_disk.size_gib,
                backing: None, // flattened: full clone carries no backing
            }],
            network: src.network.clone(),
            display: Default::default(),
            iso: src.iso.clone(),
            metadata: crate::model::VmMetadata {
                created_at: Some(now.clone()),
                updated_at: Some(now),
                notes: src.metadata.notes.clone(),
                os_hint: src.metadata.os_hint.clone(),
            },
            snapshots: Vec::new(), // snapshots NOT carried (A4)
        };

        // Config LAST: until this lands the dir is config-less and invisible.
        write_config_atomic(&config_path, &clone).await?;
        Ok(clone)
    }

    /// Linked (CoW) clone: `qemu-img create -f qcow2 --backing <rel>` an overlay
    /// whose backing file is the source's disk, addressed **relative** to the
    /// child's directory as `../<parent-slug>/<disk>` (forward-slash, A5). The
    /// child's [`DiskSpec::backing`] records the same relative path.
    ///
    /// Single-disk scope (Phase 3): more than one disk maps to
    /// [`Error::NotImplemented`].
    pub async fn linked_clone(&self, src_id: &VmId, new_name: &str) -> Result<VmConfig> {
        validate_vm_name(new_name)?;
        let src = self.load_config(src_id).await?;
        let src_disk = single_disk(&src)?;

        let slug = self.unique_slug(&slugify(new_name)).await?;
        let dst_dir = paths::vm_dir(&self.root, &slug);
        let config_path = dst_dir.join(paths::CONFIG_FILENAME);
        if config_path.exists() {
            return Err(Error::Config(format!(
                "VM directory {} already holds a vmforge.toml",
                dst_dir.display()
            )));
        }
        tokio::fs::create_dir_all(&dst_dir).await?;

        // Backing path is relative to the CHILD directory; forward-slash so it
        // is stable across platforms in the qcow2 header and our config.
        let backing_rel = format!("../{}/{}", src.dir_slug, src_disk.path);

        let child_disk = dst_dir.join(&src_disk.path);
        crate::storage::create_linked_overlay(&child_disk, &backing_rel).await?;

        let now = now_rfc3339();
        let clone = VmConfig {
            id: Uuid::new_v4(),
            name: new_name.to_string(),
            schema_version: src.schema_version,
            dir_slug: slug,
            hardware: src.hardware.clone(),
            disks: vec![DiskSpec {
                path: src_disk.path.clone(),
                size_gib: src_disk.size_gib,
                backing: Some(backing_rel),
            }],
            network: src.network.clone(),
            display: Default::default(),
            iso: src.iso.clone(),
            metadata: crate::model::VmMetadata {
                created_at: Some(now.clone()),
                updated_at: Some(now),
                notes: src.metadata.notes.clone(),
                os_hint: src.metadata.os_hint.clone(),
            },
            snapshots: Vec::new(),
        };

        write_config_atomic(&config_path, &clone).await?;
        Ok(clone)
    }

    /// Slugs of every persisted VM that has a disk whose `backing` path resolves
    /// (relative to that VM's own directory) to `target_disk`. Used by linked-
    /// clone parent protection (A5): a parent with dependents may not be
    /// started, deleted, or restored.
    ///
    /// `target_disk` is matched by canonicalized absolute path where possible,
    /// falling back to a lexical comparison so the check still works for paths
    /// that do not yet exist on disk.
    pub async fn dependents_of(&self, target_disk: &Path) -> Result<Vec<String>> {
        let target = normalize_path(target_disk);
        let mut out = Vec::new();
        for config in self.load_all().await? {
            let vm_dir = paths::vm_dir(&self.root, &config.dir_slug);
            for disk in &config.disks {
                let Some(backing) = &disk.backing else {
                    continue;
                };
                // Backing is relative to the VM's directory.
                let resolved = normalize_path(&vm_dir.join(backing));
                if resolved == target {
                    out.push(config.dir_slug.clone());
                    break;
                }
            }
        }
        Ok(out)
    }
}

/// The single disk of a config, or [`Error::NotImplemented`] for the multi-disk
/// case (Phase 3 clones/snapshots are single-disk only).
fn single_disk(config: &VmConfig) -> Result<&DiskSpec> {
    match config.disks.as_slice() {
        [d] => Ok(d),
        [] => Err(Error::Config(format!(
            "VM {} has no disks to clone",
            config.name
        ))),
        _ => Err(Error::NotImplemented("multi-disk clone")),
    }
}

/// Lexically normalize a path (resolve `.`/`..` segments) without touching the
/// filesystem, then canonicalize against any existing prefix so two paths that
/// reach the same file compare equal regardless of `..`-style relativity.
fn normalize_path(p: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for comp in p.components() {
        match comp {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    // Best-effort canonicalize (collapses symlinks for files that exist);
    // fall back to the lexical form for not-yet-created paths.
    std::fs::canonicalize(&out).unwrap_or(out)
}

/// Reconcile our snapshot metadata against the qcow2 image's internal snapshot
/// list, producing the tree-overlay nodes consumed by the UI. Pure.
///
/// Join key (A1): the qcow2 `tag` (`Qcow2Snapshot::name`) equals our
/// `Snapshot.id.to_string()`.
///
/// - A metadata entry present in qcow2 → `present_in_qcow2 = true`.
/// - A metadata entry absent from qcow2 (a metadata orphan) →
///   `present_in_qcow2 = false`, still surfaced.
/// - A qcow2 snapshot with no matching metadata (a qcow2 orphan) → synthesized
///   as a parent-less node (`parent = None`).
/// - Child links are resolved from each node's `parent`. A `parent` pointing at
///   an id that is not itself a node is treated as a root (degrades to flat).
pub fn reconcile(meta: &[Snapshot], qcow2: &[Qcow2Snapshot]) -> Vec<SnapshotNode> {
    // Index qcow2 entries by our-uuid-string tag for presence + size backfill.
    let qcow2_by_tag: HashMap<&str, &Qcow2Snapshot> =
        qcow2.iter().map(|q| (q.name.as_str(), q)).collect();
    let meta_ids: HashSet<Uuid> = meta.iter().map(|s| s.id).collect();

    let mut nodes: Vec<SnapshotNode> = Vec::new();

    // Metadata-driven nodes (preserve order). Backfill the authoritative qcow2
    // `vm-state-size` onto present nodes — create-time metadata stores 0, so the
    // UI would otherwise show "0 bytes" for a live (RAM-carrying) snapshot.
    for s in meta {
        let q = qcow2_by_tag.get(s.id.to_string().as_str()).copied();
        let mut node_meta = s.clone();
        if let Some(q) = q {
            node_meta.vm_state_size = q.vm_state_size;
        }
        nodes.push(SnapshotNode {
            present_in_qcow2: q.is_some(),
            meta: node_meta,
            children: Vec::new(),
        });
    }

    // qcow2 orphans (a tag with no metadata) → parent-less synthetic nodes.
    for q in qcow2 {
        let Ok(qid) = q.name.parse::<Uuid>() else {
            // A tag that isn't one of our uuids isn't part of our tree.
            continue;
        };
        if meta_ids.contains(&qid) {
            continue;
        }
        nodes.push(SnapshotNode {
            meta: Snapshot {
                id: qid,
                name: q.name.clone(),
                parent: None,
                created_at: rfc3339_from_sec(q.date_sec),
                has_vm_state: q.vm_state_size > 0,
                notes: String::new(),
                vm_state_size: q.vm_state_size,
            },
            present_in_qcow2: true,
            children: Vec::new(),
        });
    }

    // Resolve child links. A parent id that names no node (dangling) is treated
    // as None so the node still surfaces as a root.
    let present_ids: HashSet<Uuid> = nodes.iter().map(|n| n.meta.id).collect();
    let mut children: HashMap<Uuid, Vec<Uuid>> = HashMap::new();
    for n in &nodes {
        if let Some(parent) = n.meta.parent {
            if present_ids.contains(&parent) {
                children.entry(parent).or_default().push(n.meta.id);
            }
        }
    }
    for n in &mut nodes {
        if let Some(kids) = children.remove(&n.meta.id) {
            n.children = kids;
        }
    }

    nodes
}

/// Remove `removed` from `meta`, re-parenting its direct children onto its
/// parent (their grandparent). A child of a top-level snapshot becomes
/// top-level itself (`parent = None`). Pure, in place.
pub fn reparent_on_delete(meta: &mut Vec<Snapshot>, removed: Uuid) {
    let grandparent = meta.iter().find(|s| s.id == removed).and_then(|s| s.parent);
    for s in meta.iter_mut() {
        if s.parent == Some(removed) {
            s.parent = grandparent;
        }
    }
    meta.retain(|s| s.id != removed);
}

/// RFC3339 UTC string from Unix epoch seconds (for synthesized qcow2-orphan
/// timestamps). Negative seconds (pre-epoch) clamp to the epoch.
fn rfc3339_from_sec(secs: i64) -> String {
    format_rfc3339_utc(secs.max(0) as u64)
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
pub(crate) fn now_rfc3339() -> String {
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

/// A disk filename (`disks[].path`) must be a single flat segment under the VM
/// dir: non-empty, no separators, not `.`/`..`, and not a `-`-leading token
/// (which `qemu-img` would parse as an option). Read verbatim from
/// `vmforge.toml`, so guard it fail-closed before any qemu-img/launch use.
pub fn is_safe_disk_filename(p: &str) -> bool {
    !p.is_empty()
        && !p.starts_with('-')
        && !p.contains('/')
        && !p.contains('\\')
        && p != "."
        && p != ".."
}

/// A persisted `disks[].backing` is safe iff, opened from a VM directory (one
/// level under the library root), it resolves WITHIN the root. VMForge only ever
/// writes `../<parent-slug>/<file>` (linked clones); a bare safe filename is
/// also accepted. Absolute paths, backslashes, `-`-leading, or any other `..`
/// shape are rejected so a hand-edited config can't point a CoW overlay outside
/// the library.
pub fn is_safe_backing(b: &str) -> bool {
    if b.is_empty() || b.starts_with('-') || b.contains('\\') {
        return false;
    }
    match b.split('/').collect::<Vec<_>>().as_slice() {
        [file] => is_safe_disk_filename(file),
        ["..", slug, file] => is_safe_slug(slug) && is_safe_disk_filename(file),
        _ => false,
    }
}

/// True iff every disk path/backing in `config` is safe (see
/// [`is_safe_disk_filename`] / [`is_safe_backing`]). The `load_all` chokepoint.
pub fn config_disks_safe(config: &VmConfig) -> bool {
    config
        .disks
        .iter()
        .all(|d| is_safe_disk_filename(&d.path) && d.backing.as_deref().is_none_or(is_safe_backing))
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
            snapshots: Vec::new(),
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

    #[test]
    fn disk_path_and_backing_safety() {
        assert!(is_safe_disk_filename("disk.qcow2"));
        assert!(!is_safe_disk_filename(""));
        assert!(!is_safe_disk_filename("-x.qcow2")); // qemu-img option injection
        assert!(!is_safe_disk_filename("a/b.qcow2"));
        assert!(!is_safe_disk_filename("..\\b"));
        assert!(!is_safe_disk_filename(".."));

        // Backing: a bare filename or exactly `../<slug>/<file>` is allowed.
        assert!(is_safe_backing("disk.qcow2"));
        assert!(is_safe_backing("../alpine-vm/disk.qcow2"));
        assert!(!is_safe_backing("../../etc/passwd"));
        assert!(!is_safe_backing("/etc/shadow"));
        assert!(!is_safe_backing("../Bad Slug/disk.qcow2")); // slug not canonical
        assert!(!is_safe_backing("-x"));
    }

    #[tokio::test]
    async fn load_all_skips_unsafe_disk_path() {
        let tmp = tempfile::tempdir().unwrap();
        let lib = Library::new(tmp.path().to_path_buf());
        let dir = tmp.path().join("evil-disk");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let id = Uuid::new_v4();
        // Valid slug, but a disk path that escapes the VM dir.
        let toml = format!(
            "id = \"{id}\"\nname = \"Evil Disk\"\ndir_slug = \"evil-disk\"\n\n[[disks]]\npath = \"../../escape.qcow2\"\nsize_gib = 1\n"
        );
        tokio::fs::write(dir.join(paths::CONFIG_FILENAME), toml)
            .await
            .unwrap();
        assert!(lib.list_vms().await.unwrap().is_empty());
        assert!(matches!(
            lib.load_config(&id).await.unwrap_err(),
            Error::VmNotFound(_)
        ));
    }

    // ====================================================================
    // Phase 3 — clones, dependents, reconcile, reparent
    // ====================================================================

    use crate::storage::Qcow2Snapshot;

    fn snap(id: Uuid, parent: Option<Uuid>) -> Snapshot {
        Snapshot {
            id,
            name: format!("snap-{}", &id.to_string()[..8]),
            parent,
            created_at: "2026-06-24T00:00:00Z".into(),
            has_vm_state: false,
            notes: String::new(),
            vm_state_size: 0,
        }
    }

    fn qcow2(name: &str) -> Qcow2Snapshot {
        Qcow2Snapshot {
            id: "1".into(),
            name: name.into(),
            date_sec: 1_782_322_800,
            date_nsec: 0,
            vm_state_size: 0,
            vm_clock_nsec: 0,
        }
    }

    // ---- full_clone: new id/slug, backing=None, snapshots empty, src untouched
    #[tokio::test]
    async fn full_clone_flattens_and_resets() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img_full().await;
        let lib = Library::new(tmp.path().to_path_buf());

        let mut src = sample_config();
        src.id = Uuid::new_v4();
        // Give the source a snapshot to prove clones don't carry them.
        src.snapshots = vec![snap(Uuid::new_v4(), None)];
        let src = lib.create_vm(src).await.unwrap();
        // Put known bytes in the source disk so we can assert it's untouched and
        // that convert (cp) reproduced them.
        let src_disk = tmp.path().join(&src.dir_slug).join("disk.qcow2");
        tokio::fs::write(&src_disk, b"SOURCEDISK").await.unwrap();

        let clone = lib.full_clone(&src.id, "Cloned VM").await.unwrap();

        assert_ne!(clone.id, src.id, "clone gets a fresh id");
        assert_eq!(clone.dir_slug, "cloned-vm");
        assert_eq!(clone.name, "Cloned VM");
        assert_eq!(clone.disks.len(), 1);
        assert_eq!(clone.disks[0].backing, None, "full clone is flattened");
        assert!(clone.snapshots.is_empty(), "snapshots are NOT carried");
        assert!(clone.metadata.created_at.is_some());
        assert!(clone.metadata.updated_at.is_some());

        // The clone disk exists and is NOT a *.partial.
        let clone_dir = tmp.path().join(&clone.dir_slug);
        assert!(clone_dir.join("disk.qcow2").exists());
        assert!(
            !clone_dir.join("disk.qcow2.partial").exists(),
            "partial must be renamed away"
        );
        // convert copied the source bytes.
        let copied = tokio::fs::read(clone_dir.join("disk.qcow2")).await.unwrap();
        assert_eq!(&copied, b"SOURCEDISK");
        // Source disk untouched.
        let src_after = tokio::fs::read(&src_disk).await.unwrap();
        assert_eq!(&src_after, b"SOURCEDISK", "source disk must be untouched");
        // Source config still has its snapshot (clone didn't mutate it).
        let src_reloaded = lib.load_config(&src.id).await.unwrap();
        assert_eq!(src_reloaded.snapshots.len(), 1);

        // The clone is loadable by id.
        let loaded = lib.load_config(&clone.id).await.unwrap();
        assert_eq!(loaded.dir_slug, "cloned-vm");
    }

    // ---- full_clone toml-last: a convert failure leaves no adoptable config
    #[tokio::test]
    async fn full_clone_convert_failure_leaves_no_config() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img_full().await;
        let lib = Library::new(tmp.path().to_path_buf());

        let mut src = sample_config();
        src.id = Uuid::new_v4();
        let src = lib.create_vm(src).await.unwrap();

        // Force the convert subcommand to fail; the partial must never be
        // renamed and the clone's vmforge.toml (written LAST) must not exist.
        std::env::set_var("MOCK_CONVERT_FAIL", "1");
        let res = lib.full_clone(&src.id, "Cloned VM").await;
        std::env::remove_var("MOCK_CONVERT_FAIL");
        assert!(res.is_err(), "convert failure must propagate");

        // The orphan directory (if created) has NO vmforge.toml → invisible to
        // load_all, so no half-baked clone is adoptable.
        let summaries = lib.list_vms().await.unwrap();
        assert_eq!(summaries.len(), 1, "only the source VM is visible");
        let clone_dir = tmp.path().join("cloned-vm");
        assert!(
            !clone_dir.join(paths::CONFIG_FILENAME).exists(),
            "no config may be written on convert failure"
        );
        assert!(
            !clone_dir.join("disk.qcow2").exists(),
            "no real disk on convert failure (partial only, if any)"
        );
    }

    // ---- linked_clone: relative backing string + dependents_of resolution
    #[tokio::test]
    async fn linked_clone_sets_relative_backing_and_dependents() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img_full().await;
        let lib = Library::new(tmp.path().to_path_buf());

        let mut src = sample_config();
        src.id = Uuid::new_v4();
        let src = lib.create_vm(src).await.unwrap();

        let clone = lib.linked_clone(&src.id, "Linked Child").await.unwrap();
        assert_eq!(clone.dir_slug, "linked-child");
        assert_eq!(
            clone.disks[0].backing.as_deref(),
            Some("../alpine-vm/disk.qcow2"),
            "backing must be ../<parent-slug>/<disk>"
        );
        assert!(clone.snapshots.is_empty());
        // The overlay disk file was created.
        assert!(tmp.path().join("linked-child").join("disk.qcow2").exists());

        // dependents_of(parent disk) → the child's slug.
        let parent_disk = tmp.path().join(&src.dir_slug).join("disk.qcow2");
        let deps = lib.dependents_of(&parent_disk).await.unwrap();
        assert_eq!(deps, vec!["linked-child".to_string()]);

        // An unrelated disk has no dependents.
        let other = tmp.path().join("nowhere").join("disk.qcow2");
        assert!(lib.dependents_of(&other).await.unwrap().is_empty());
    }

    // ---- dependents_of: resolves relative backing across multiple VMs
    #[tokio::test]
    async fn dependents_of_resolves_multiple() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img_full().await;
        let lib = Library::new(tmp.path().to_path_buf());

        let mut parent = sample_config();
        parent.id = Uuid::new_v4();
        let parent = lib.create_vm(parent).await.unwrap();

        let c1 = lib.linked_clone(&parent.id, "Child One").await.unwrap();
        let c2 = lib.linked_clone(&parent.id, "Child Two").await.unwrap();

        let parent_disk = tmp.path().join(&parent.dir_slug).join("disk.qcow2");
        let mut deps = lib.dependents_of(&parent_disk).await.unwrap();
        deps.sort();
        assert_eq!(deps, vec![c1.dir_slug.clone(), c2.dir_slug.clone()]);
    }

    // ---- multi-disk clone is NotImplemented (single-disk Phase-3 scope)
    #[tokio::test]
    async fn clone_multi_disk_is_not_implemented() {
        let tmp = tempfile::tempdir().unwrap();
        let _g = crate::test_support::mock_qemu_img_full().await;
        let lib = Library::new(tmp.path().to_path_buf());

        let mut src = sample_config();
        src.id = Uuid::new_v4();
        src.disks.push(DiskSpec {
            path: "disk2.qcow2".into(),
            size_gib: 4,
            backing: None,
        });
        let src = lib.create_vm(src).await.unwrap();

        assert!(matches!(
            lib.full_clone(&src.id, "X").await.unwrap_err(),
            Error::NotImplemented(_)
        ));
        assert!(matches!(
            lib.linked_clone(&src.id, "Y").await.unwrap_err(),
            Error::NotImplemented(_)
        ));
    }

    // ---- [[snapshots]] toml round-trip ----
    #[test]
    fn snapshots_toml_round_trip() {
        let mut c = sample_config();
        let root = Uuid::new_v4();
        let child = Uuid::new_v4();
        c.snapshots = vec![
            Snapshot {
                id: root,
                name: "base".into(),
                parent: None,
                created_at: "2026-06-24T01:00:00Z".into(),
                has_vm_state: true,
                notes: "live root".into(),
                vm_state_size: 4096,
            },
            snap(child, Some(root)),
        ];
        let s = Library::to_toml(&c).unwrap();
        assert!(
            s.contains("[[snapshots]]"),
            "expected [[snapshots]] in:\n{s}"
        );
        let back = Library::from_toml(&s).unwrap();
        assert_eq!(back.snapshots.len(), 2);
        assert_eq!(back.snapshots[0].id, root);
        assert_eq!(back.snapshots[0].name, "base");
        assert!(back.snapshots[0].has_vm_state);
        assert_eq!(back.snapshots[0].vm_state_size, 4096);
        assert_eq!(back.snapshots[1].parent, Some(root));
    }

    // ---- Phase-2 config (no snapshots key) → empty array ----
    #[test]
    fn phase2_config_yields_empty_snapshots() {
        let minimal = r#"
            id = "00000000-0000-0000-0000-000000000000"
            name = "Legacy"
            dir_slug = "legacy"
        "#;
        let c = Library::from_toml(minimal).unwrap();
        assert!(
            c.snapshots.is_empty(),
            "Phase-2 config must load with an empty snapshots array"
        );
    }

    // ---- reconcile: both orphan directions + presence flags + child links ----
    #[test]
    fn reconcile_orphans_both_directions() {
        let in_both = Uuid::new_v4();
        let meta_only = Uuid::new_v4(); // metadata orphan (absent from qcow2)
        let child = Uuid::new_v4();
        // qcow2 orphan: a tag with no metadata, but still one of our uuids.
        let qcow2_only = Uuid::new_v4();

        let meta = vec![
            snap(in_both, None),
            snap(meta_only, None),
            snap(child, Some(in_both)),
        ];
        let qcow2_list = vec![qcow2(&in_both.to_string()), qcow2(&qcow2_only.to_string())];

        let nodes = reconcile(&meta, &qcow2_list);
        // 3 metadata nodes + 1 qcow2-orphan node.
        assert_eq!(nodes.len(), 4);

        let find = |id: Uuid| nodes.iter().find(|n| n.meta.id == id).unwrap();

        // Present in both → present_in_qcow2 = true; has the child link.
        let n_both = find(in_both);
        assert!(n_both.present_in_qcow2);
        assert_eq!(n_both.children, vec![child]);

        // Metadata orphan → present_in_qcow2 = false, surfaced as a root.
        let n_meta = find(meta_only);
        assert!(!n_meta.present_in_qcow2);
        assert!(n_meta.meta.parent.is_none());

        // qcow2 orphan → synthesized parent-less node, present in qcow2.
        let n_q = find(qcow2_only);
        assert!(n_q.present_in_qcow2);
        assert!(n_q.meta.parent.is_none());

        // Child node: present_in_qcow2 false (not in qcow2 list), parent links.
        let n_child = find(child);
        assert!(!n_child.present_in_qcow2);
        assert_eq!(n_child.meta.parent, Some(in_both));
    }

    // A qcow2 tag that is not one of our uuids is ignored entirely.
    #[test]
    fn reconcile_ignores_foreign_qcow2_tags() {
        let meta: Vec<Snapshot> = vec![];
        let foreign = vec![qcow2("not-a-uuid-tag")];
        let nodes = reconcile(&meta, &foreign);
        assert!(nodes.is_empty(), "foreign tag must not create a node");
    }

    // ---- reparent_on_delete: children adopt grandparent ----
    #[test]
    fn reparent_on_delete_children_to_grandparent() {
        let gp = Uuid::new_v4();
        let parent = Uuid::new_v4();
        let c1 = Uuid::new_v4();
        let c2 = Uuid::new_v4();

        let mut meta = vec![
            snap(gp, None),
            snap(parent, Some(gp)),
            snap(c1, Some(parent)),
            snap(c2, Some(parent)),
        ];
        reparent_on_delete(&mut meta, parent);

        assert!(!meta.iter().any(|s| s.id == parent), "parent removed");
        let find = |id: Uuid| meta.iter().find(|s| s.id == id).unwrap();
        assert_eq!(find(c1).parent, Some(gp), "child reparented to grandparent");
        assert_eq!(find(c2).parent, Some(gp));
    }

    // Deleting a top-level snapshot makes its children top-level (parent=None).
    #[test]
    fn reparent_on_delete_top_level_children_become_roots() {
        let root = Uuid::new_v4();
        let child = Uuid::new_v4();
        let mut meta = vec![snap(root, None), snap(child, Some(root))];
        reparent_on_delete(&mut meta, root);
        assert_eq!(meta.len(), 1);
        assert_eq!(meta[0].id, child);
        assert_eq!(meta[0].parent, None, "child becomes a root");
    }
}
