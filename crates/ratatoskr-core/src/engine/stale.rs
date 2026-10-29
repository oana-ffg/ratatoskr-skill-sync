//! Explicit decisions for ownership left behind by a target configuration change.
use super::*;

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum StaleAction {
    Forget,
    Remove,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StaleFile {
    pub path: PathBuf,
    pub owned: bool,
    /// Includes missing files and executable-bit drift from the recorded ownership.
    pub modified: bool,
    readonly: Option<bool>,
    recorded: Option<Fingerprint>,
    current: Option<Fingerprint>,
    protection: Option<Protection>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StaleLocation {
    pub path: PathBuf,
    pub harness: Option<String>,
    pub files: Vec<StaleFile>,
    /// Previously owned skill directories released by the decision. Only roots
    /// disjoint from current targets move whole; otherwise only stale files move.
    pub roots: BTreeSet<PathBuf>,
    archive_roots: BTreeSet<PathBuf>,
    directories: BTreeMap<PathBuf, u32>,
}

#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct StalePreview {
    pub action: StaleAction,
    pub location: StaleLocation,
    /// Primary receipt directory returned by resolve.
    pub archive: Option<PathBuf>,
    /// Each former target archives locally so a grouped decision can span volumes.
    pub archives: BTreeSet<PathBuf>,
    /// Binds the decision to the whole ownership/configuration, not just one file.
    revision: String,
}

fn allowed_file(path: &Path, desired: &Desired) -> bool {
    desired
        .allowed
        .iter()
        .any(|(skills, global)| path.starts_with(skills) || path == global)
}

fn allowed_root(path: &Path, desired: &Desired) -> bool {
    desired
        .allowed
        .iter()
        .any(|(skills, _)| path.parent() == Some(skills))
}

pub(super) fn active_manifest(manifest: &Manifest, desired: &Desired) -> Manifest {
    let mut active = manifest.clone();
    active.files.retain(|path, _| allowed_file(path, desired));
    active.roots.retain(|path| allowed_root(path, desired));
    active.retain_owned_targets();
    active
}

/// Recorded harnesses share one decision. Historical manifests without labels
/// keep their original per-path decisions; current definitions cannot identify them.
#[derive(PartialEq, Eq, PartialOrd, Ord)]
enum LocationKey {
    Harness(String),
    Path(PathBuf),
}

fn location<'a>(
    groups: &'a mut BTreeMap<LocationKey, StaleLocation>,
    manifest: &Manifest,
    path: &Path,
) -> &'a mut StaleLocation {
    let harness = manifest.target_harnesses.get(path).cloned();
    let key = match &harness {
        Some(name) => LocationKey::Harness(name.clone()),
        None => LocationKey::Path(path.into()),
    };
    groups.entry(key).or_insert_with(|| StaleLocation {
        harness,
        path: path.into(),
        files: vec![],
        roots: BTreeSet::new(),
        archive_roots: BTreeSet::new(),
        directories: BTreeMap::new(),
    })
}

fn inspect_file(path: &Path, recorded: Option<&Fingerprint>) -> Result<StaleFile> {
    no_symlinks(path)?;
    let protection = protection::capture(path)?;
    let readonly = if path.try_exists()? {
        Some(fs::metadata(path)?.permissions().readonly())
    } else {
        None
    };
    let current = if path.try_exists()? {
        Some(fingerprint(&fs::read(path)?, executable(path)?))
    } else {
        None
    };
    Ok(StaleFile {
        path: path.into(),
        owned: recorded.is_some(),
        modified: recorded.is_some() && current.as_ref() != recorded,
        readonly,
        recorded: recorded.cloned(),
        current,
        protection,
    })
}

fn directory_mode(path: &Path) -> Result<u32> {
    let metadata = fs::metadata(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        Ok(metadata.mode())
    }
    #[cfg(not(unix))]
    {
        Ok(u32::from(metadata.permissions().readonly()))
    }
}

pub(super) fn locations(desired: &Desired, manifest: &Manifest) -> Result<Vec<StaleLocation>> {
    let stale_roots: BTreeSet<_> = manifest
        .roots
        .iter()
        .filter(|r| !allowed_root(r, desired))
        .cloned()
        .collect();
    let mut groups = BTreeMap::new();
    for root in &stale_roots {
        let path = root
            .parent()
            .context("Ownership root has no skills directory")?
            .to_path_buf();
        let location = location(&mut groups, manifest, &path);
        location.roots.insert(root.clone());
    }
    for path in manifest.files.keys().filter(|p| !allowed_file(p, desired)) {
        let group = stale_roots
            .iter()
            .find(|root| path.starts_with(root))
            .and_then(|root| root.parent())
            .unwrap_or(path);
        let location = location(&mut groups, manifest, group);
        location
            .files
            .push(inspect_file(path, manifest.files.get(path))?);
    }
    for location in groups.values_mut() {
        // A root must never move files still allowed by another current target.
        for root in &location.roots {
            let can_archive = !desired
                .allowed
                .iter()
                .any(|(skills, global)| overlaps(root, skills) || overlaps(root, global));
            if can_archive {
                location.archive_roots.insert(root.clone());
            }
            no_symlinks(root)?;
            if !root.try_exists()? {
                continue;
            }
            ensure!(
                root.is_dir(),
                "Ownership root is not a directory: {}",
                root.display()
            );
            for entry in WalkDir::new(root).follow_links(false) {
                let entry = entry?;
                no_symlinks(entry.path())?;
                if entry.file_type().is_dir() {
                    location
                        .directories
                        .insert(entry.path().into(), directory_mode(entry.path())?);
                } else {
                    ensure!(
                        entry.file_type().is_file(),
                        "Unsupported output: {}",
                        entry.path().display()
                    );
                    if can_archive && !location.files.iter().any(|file| file.path == entry.path()) {
                        location.files.push(inspect_file(
                            entry.path(),
                            manifest.files.get(entry.path()),
                        )?);
                    }
                }
            }
        }
        location.files.sort_by(|a, b| a.path.cmp(&b.path));
    }
    Ok(groups.into_values().collect())
}

pub fn stale_targets(settings: &Settings) -> Result<Vec<StaleLocation>> {
    settings.validate()?;
    let desired = desired(settings)?;
    locations(&desired, &manifest(settings)?)
}

fn revision(settings: &Settings, manifest: &Manifest, desired: &Desired) -> Result<String> {
    let mut hash = Sha256::new();
    hash.update(serde_json::to_vec(manifest)?);
    hash.update(serde_json::to_vec(&desired.allowed)?);
    for path in [
        settings.source.join("harnesses.toml"),
        settings
            .source
            .join("machines")
            .join(format!("{}.toml", settings.machine)),
    ] {
        no_symlinks(&path)?;
        hash.update(fs::read(path)?);
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn archive_parent(target: &Path) -> Result<PathBuf> {
    Ok(target
        .parent()
        .context("Former target has no parent")?
        .join("ratatoskr-backups"))
}

pub fn preview_stale_target(
    settings: &Settings,
    path: &Path,
    action: StaleAction,
) -> Result<StalePreview> {
    settings.validate()?;
    let desired = desired(settings)?;
    let manifest = manifest(settings)?;
    let location = locations(&desired, &manifest)?
        .into_iter()
        .find(|l| l.path == path)
        .context("Stale target no longer exists; review again")?;
    let archive = if action == StaleAction::Remove {
        Some(
            archive_parent(&location.path)?
                .join(chrono::Utc::now().format("%Y%m%d%H%M%S%f").to_string()),
        )
    } else {
        None
    };
    let archives = if let Some(archive) = &archive {
        archive_directories(&archive_moves(&location, archive)?, archive)?
    } else {
        BTreeSet::new()
    };
    Ok(StalePreview {
        action,
        location,
        archive,
        archives,
        revision: revision(settings, &manifest, &desired)?,
    })
}

fn detach(manifest: &mut Manifest, location: &StaleLocation) {
    manifest.files.retain(|path, _| {
        !location
            .files
            .iter()
            .any(|file| file.owned && file.path == *path)
    });
    manifest.roots.retain(|root| !location.roots.contains(root));
    manifest.retain_owned_targets();
    for roots in manifest.project_roots.values_mut() {
        roots.retain(|root| !location.roots.contains(root));
    }
}

fn archive_moves(location: &StaleLocation, archive: &Path) -> Result<BTreeMap<PathBuf, PathBuf>> {
    let name = archive.file_name().context("Invalid archive name")?;
    let paths: BTreeSet<_> = location
        .archive_roots
        .iter()
        .cloned()
        .chain(
            location
                .files
                .iter()
                .filter(|f| {
                    f.owned
                        && !location
                            .archive_roots
                            .iter()
                            .any(|root| f.path.starts_with(root))
                })
                .map(|f| f.path.clone()),
        )
        .collect();
    let mut moves = BTreeMap::new();
    for (index, path) in paths.into_iter().enumerate() {
        let target = location
            .roots
            .iter()
            .find(|root| path.starts_with(root))
            .and_then(|root| root.parent())
            .unwrap_or(&path);
        let directory = archive_parent(target)?.join(name);
        moves.insert(path, directory.join("entries").join(index.to_string()));
    }
    Ok(moves)
}

fn archive_directories(
    moves: &BTreeMap<PathBuf, PathBuf>,
    primary: &Path,
) -> Result<BTreeSet<PathBuf>> {
    let mut archives = BTreeSet::from([primary.to_path_buf()]);
    for destination in moves.values() {
        archives.insert(
            destination
                .parent()
                .and_then(Path::parent)
                .context("Archive entry has no directory")?
                .to_path_buf(),
        );
    }
    Ok(archives)
}

fn prepare_archive(
    settings: &Settings,
    desired: &Desired,
    expected: &StalePreview,
) -> Result<BTreeMap<PathBuf, PathBuf>> {
    let archive = expected
        .archive
        .as_ref()
        .context("Remove requires an archive")?;
    ensure!(
        archive.parent() == Some(archive_parent(&expected.location.path)?.as_path()),
        "Archive outside approved backup folder"
    );
    identifier(
        archive
            .file_name()
            .and_then(|s| s.to_str())
            .context("Invalid archive name")?,
    )?;
    let planned = archive_moves(&expected.location, archive)?;
    let archives = archive_directories(&planned, archive)?;
    ensure!(
        archives == expected.archives,
        "Archive destinations changed; review again"
    );
    for directory in &archives {
        no_symlinks(directory)?;
        ensure!(
            !directory.try_exists()?,
            "Archive already exists; review again: {}",
            directory.display()
        );
        ensure!(
            !overlaps(directory, &settings.source)
                && !desired
                    .allowed
                    .iter()
                    .any(|(skills, global)| overlaps(directory, skills)
                        || overlaps(directory, global))
                && !expected
                    .location
                    .roots
                    .iter()
                    .any(|root| overlaps(directory, root)),
            "Archive overlaps managed content"
        );
    }
    let mut moves = BTreeMap::new();
    for (path, destination) in planned {
        if path.try_exists()? {
            moves.insert(path, destination);
        }
    }
    // Each target has a local archive. All receipts contain the complete mapping,
    // written before the first move, even when this decision spans volumes.
    let receipt = serde_json::to_vec_pretty(
        &serde_json::json!({"preview": expected, "moves": moves, "recovery": "Move each archived entry back to its original path only when that path is absent. Nothing deleted; ownership is not automatically restored."}),
    )?;
    for directory in archives {
        fs::create_dir_all(directory.parent().context("Archive parent missing")?)?;
        fs::create_dir(&directory)?;
        fs::create_dir(directory.join("entries"))?;
        atomic_write(&directory.join("receipt.json"), &receipt)?;
    }
    Ok(moves)
}

/// Returns the archive directory for Remove; Forget never touches installed files.
pub fn resolve_stale_target(settings: &Settings, expected: &str) -> Result<Option<PathBuf>> {
    settings.validate()?;
    let _lock = crate::config::operation_lock(settings, "apply.lock")?;
    let expected: StalePreview = serde_json::from_str(expected)?;
    let desired = desired(settings)?;
    let mut manifest = manifest(settings)?;
    let current = locations(&desired, &manifest)?
        .into_iter()
        .find(|l| l.path == expected.location.path);
    ensure!(
        current.as_ref() == Some(&expected.location)
            && revision(settings, &manifest, &desired)? == expected.revision,
        "Stale target changed since preview; review again"
    );
    protection::capture(&manifest_path(settings))?;
    let mut moved = Vec::new();
    let result = (|| -> Result<()> {
        match expected.action {
            StaleAction::Forget => ensure!(
                expected.archive.is_none() && expected.archives.is_empty(),
                "Forget must not specify an archive"
            ),
            StaleAction::Remove => {
                let moves = prepare_archive(settings, &desired, &expected)?;
                ensure!(
                    locations(&desired, &manifest)?
                        .into_iter()
                        .find(|l| l.path == expected.location.path)
                        .as_ref()
                        == Some(&expected.location),
                    "Stale target changed since preview; review again"
                );
                for (source, destination) in moves {
                    no_symlinks(&source)?;
                    no_symlinks(&destination)?;
                    ensure!(
                        !destination.try_exists()?,
                        "Archive destination appeared; review again: {}",
                        destination.display()
                    );
                    fs::rename(&source, &destination).with_context(|| {
                        format!("Archive {} to {}", source.display(), destination.display())
                    })?;
                    moved.push((source, destination));
                }
            }
        }
        detach(&mut manifest, &expected.location);
        atomic_write(
            &manifest_path(settings),
            &serde_json::to_vec_pretty(&manifest)?,
        )
    })();
    if let Err(error) = result {
        let mut failures = Vec::new();
        for (source, destination) in moved.into_iter().rev() {
            let rollback = (|| -> Result<()> {
                no_symlinks(&source)?;
                ensure!(!source.try_exists()?, "Original path now occupied");
                fs::rename(&destination, &source)?;
                Ok(())
            })();
            if let Err(rollback) = rollback {
                failures.push(format!(
                    "{} remains at {}: {rollback:#}",
                    source.display(),
                    destination.display()
                ));
            }
        }
        let mut message = if failures.is_empty() {
            "No installed files changed (any moves rolled back); ownership unchanged.".to_owned()
        } else {
            format!(
                "Ownership unchanged; rollback incomplete: {}.",
                failures.join("; ")
            )
        };
        if let Some(archive) = &expected.archive {
            message.push_str(&format!(
                " Archive receipt, if created: {}",
                archive.join("receipt.json").display()
            ));
        }
        return Err(error).context(message);
    }
    Ok(expected.archive)
}
