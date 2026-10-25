use super::*;

#[derive(Clone, Debug, Serialize)]
pub struct Conflict {
    pub harness: Option<String>,
    pub skill: Option<String>,
    pub path: PathBuf,
    pub kind: String,
    pub diff: Option<String>,
    pub source: Option<PathBuf>,
    pub copy_back_blocked: Option<String>,
}

fn text_diff(before: &[u8], after: &[u8]) -> Option<String> {
    if before.len() + after.len() > 131_072 {
        return None;
    }
    Some(
        similar::TextDiff::from_lines(
            std::str::from_utf8(before).ok()?,
            std::str::from_utf8(after).ok()?,
        )
        .unified_diff()
        .header("installed", "rendered source")
        .to_string(),
    )
}

pub(super) fn conflict(path: &Path, kind: &str, desired: &Desired) -> Result<Conflict> {
    let file = desired.files.get(path);
    let installed = if path.is_file() {
        Some(fs::read(path)?)
    } else {
        None
    };
    let blocked = if file.is_none() {
        Some("No single source file maps to this destination. Edit the source instead.".into())
    } else if installed.is_none() {
        Some("The installed file is missing. Edit the source instead.".into())
    } else if !file.is_some_and(|f| f.reversible) {
        Some("Conditional blocks, variables or injected frontmatter make copy-back ambiguous. Edit the source instead.".into())
    } else {
        None
    };
    let identity = file.or_else(|| {
        desired
            .files
            .iter()
            .find(|(p, _)| p.starts_with(path))
            .map(|(_, f)| f)
    });
    Ok(Conflict {
        harness: identity.map(|f| f.harness.clone()),
        skill: identity.and_then(|f| f.skill.clone()),
        path: path.to_path_buf(),
        kind: kind.into(),
        diff: installed.as_deref().map_or_else(
            || file.and_then(|f| text_diff(b"", &f.bytes)),
            |b| text_diff(b, file.map_or(b"".as_slice(), |f| f.bytes.as_slice())),
        ),
        source: file.map(|f| f.source.clone()),
        copy_back_blocked: blocked,
    })
}

#[derive(Clone, Copy, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum ConflictAction {
    CopyBack,
    EditSource,
    Reinstall,
    Adopt,
}

#[derive(Serialize)]
pub struct ResolutionPreview {
    pub path: PathBuf,
    pub action: ConflictAction,
    pub conflict: Conflict,
    pub effects: Vec<Change>,
    pub explanation: String,
    // Covers source bytes, permissions, ownership and all installed files, even for a zero-diff adoption.
    pub revision: String,
    pub source_edit: Option<String>,
}

fn scope_files(path: &Path, desired: &Desired, owned: &Manifest) -> Result<BTreeSet<PathBuf>> {
    let mut paths: BTreeSet<PathBuf> = desired
        .files
        .keys()
        .chain(owned.files.keys())
        .filter(|p| *p == path || p.starts_with(path))
        .cloned()
        .collect();
    if path.is_dir() {
        paths.extend(file_paths(path)?.into_values());
    } else {
        paths.insert(path.to_path_buf());
    }
    Ok(paths)
}

pub fn preview_resolution(
    settings: &Settings,
    path: &Path,
    action: ConflictAction,
) -> Result<ResolutionPreview> {
    let desired = desired(settings)?;
    let owned = manifest(settings, &desired)?;
    resolution_preview(settings, path, action, None, &desired, &owned)
}

fn resolution_preview(
    settings: &Settings,
    path: &Path,
    action: ConflictAction,
    source_edit: Option<&str>,
    desired: &Desired,
    owned: &Manifest,
) -> Result<ResolutionPreview> {
    let check = preflight(settings, desired, owned)?;
    let conflict = check
        .conflict_details
        .into_iter()
        .find(|c| c.path == path)
        .context("This path is no longer a conflict. Refresh the plan.")?;
    if action == ConflictAction::CopyBack {
        ensure!(
            conflict.copy_back_blocked.is_none(),
            "{}",
            conflict
                .copy_back_blocked
                .as_deref()
                .unwrap_or("Copy-back unavailable")
        );
    }
    ensure!(
        (action == ConflictAction::EditSource) == source_edit.is_some(),
        "Source editing requires explicit draft text"
    );
    let paths = scope_files(path, desired, owned)?;
    let mut revision = Sha256::new();
    revision.update(serde_json::to_vec(owned)?);
    revision.update(serde_json::to_vec(&check.changes)?);
    revision.update([u8::from(path.is_dir())]);
    let mut effects = Vec::new();
    for destination in paths {
        no_symlinks(&destination)?;
        let before = if destination.try_exists()? {
            Some(fs::read(&destination)?)
        } else {
            None
        };
        let current = before
            .as_ref()
            .map(|b| executable(&destination).map(|x| fingerprint(b, x)))
            .transpose()?;
        revision.update(destination.to_string_lossy().as_bytes());
        revision.update(serde_json::to_vec(&current)?);
        revision.update(serde_json::to_vec(&protection::capture(&destination)?)?);
        let next = desired.files.get(&destination);
        if let Some(file) = next {
            no_symlinks(&file.source)?;
            revision.update(file.source.to_string_lossy().as_bytes());
            revision.update(fs::read(&file.source)?);
            revision.update([u8::from(executable(&file.source)?)]);
            revision.update(serde_json::to_vec(&protection::capture(&file.source)?)?);
            revision.update(&file.bytes);
        }
        let (effect_path, after, exec) = if matches!(
            action,
            ConflictAction::CopyBack | ConflictAction::EditSource
        ) {
            let file = next.context("Source mapping disappeared")?;
            let installed = if let Some(text) = source_edit {
                text.as_bytes()
            } else {
                before.as_deref().context("Installed file disappeared")?
            };
            let original = fs::read(&file.source)?;
            effects.push(Change {
                path: file.source.clone(),
                action: if source_edit.is_some() {
                    "save source edit"
                } else {
                    "copy installed content to source"
                }
                .into(),
                diff: text_diff(&original, installed),
                before_sha256: Some(fingerprint(&original, executable(&file.source)?).sha256),
                after_sha256: Some(fingerprint(installed, false).sha256),
                executable: Some(if source_edit.is_some() {
                    executable(&file.source)?
                } else {
                    current
                        .as_ref()
                        .context("Installed file disappeared")?
                        .executable
                }),
            });
            continue;
        } else if action == ConflictAction::Adopt {
            (
                destination.clone(),
                before.as_deref(),
                current.as_ref().map(|f| f.executable),
            )
        } else {
            (
                destination.clone(),
                next.map(|f| f.bytes.as_slice()),
                next.map(|f| f.executable),
            )
        };
        effects.push(Change {
            path: effect_path,
            action: if action == ConflictAction::Adopt {
                "record ownership as-is"
            } else if after.is_some() {
                "reinstall"
            } else {
                "remove unmanaged file"
            }
            .into(),
            diff: text_diff(
                before.as_deref().unwrap_or_default(),
                after.unwrap_or_default(),
            ),
            before_sha256: current.map(|f| f.sha256),
            after_sha256: after.map(|b| fingerprint(b, exec.unwrap_or(false)).sha256),
            executable: exec,
        });
    }
    let explanation = match action {
        ConflictAction::EditSource => {
            "Back up source and installed content, then save this source edit. Installed files and ownership stay unchanged. Review the conflict again afterwards."
        }
        ConflictAction::CopyBack => {
            "Back up source and installed content, then copy installed bytes and executable permission to the mapped source. Other installations update only after reviewing Apply."
        }
        ConflictAction::Reinstall => {
            "Back up installed content and ownership, then replace only this conflict with rendered source. Extra files in an unmanaged directory are backed up and removed."
        }
        ConflictAction::Adopt => {
            "Back up installed content and ownership, then record its current state. Source differences remain pending; a later Apply will replace them."
        }
    };
    Ok(ResolutionPreview {
        source_edit: source_edit.map(str::to_owned),
        path: path.into(),
        action,
        conflict,
        effects,
        revision: format!("{:x}", revision.finalize()),
        explanation: explanation.into(),
    })
}

pub fn resolve_conflict(settings: &Settings, expected: &str) -> Result<PathBuf> {
    settings.validate()?;
    let expected: serde_json::Value = serde_json::from_str(expected)?;
    let path: PathBuf =
        serde_json::from_value(expected.get("path").context("Preview has no path")?.clone())?;
    let action: ConflictAction = serde_json::from_value(
        expected
            .get("action")
            .context("Preview has no action")?
            .clone(),
    )?;
    let source_edit = expected.get("source_edit").and_then(|v| v.as_str());
    let _lock = crate::config::operation_lock(settings, "apply.lock")?;
    let desired = desired(settings)?;
    let original = manifest(settings, &desired)?;
    let review = resolution_preview(settings, &path, action, source_edit, &desired, &original)?;
    ensure!(
        serde_json::to_value(&review)? == expected,
        "Workspace changed since preview. Review the action again."
    );
    let paths = scope_files(&path, &desired, &original)?;
    let mut captured = original.clone();
    // A snapshot records the real bytes, rather than pretending drift still matches old ownership.
    for p in original.files.keys().chain(paths.iter()) {
        no_symlinks(p)?;
        if p.try_exists()? {
            captured
                .files
                .insert(p.clone(), fingerprint(&fs::read(p)?, executable(p)?));
        } else {
            captured.files.remove(p);
        }
    }
    let source = if matches!(
        action,
        ConflictAction::CopyBack | ConflictAction::EditSource
    ) {
        let file = desired.files.get(&path).context("No source mapping")?;
        let bytes = if let Some(text) = source_edit {
            text.as_bytes().to_vec()
        } else {
            fs::read(&path)?
        };
        crate::content::validate_source_replacement(settings, &file.source, &bytes)?;
        Some((
            file.source.clone(),
            bytes,
            if source_edit.is_some() {
                executable(&file.source)?
            } else {
                executable(&path)?
            },
        ))
    } else {
        None
    };
    let directory = backup(settings, &captured)?;
    let snapshot_path = directory.join("snapshot.json");
    let mut snapshot: Backup = serde_json::from_slice(&fs::read(&snapshot_path)?)?;
    snapshot.ownership = Some(original.clone());
    snapshot.absent = paths
        .iter()
        .filter(|p| !captured.files.contains_key(*p))
        .cloned()
        .collect();
    if let Some((source, edited, executable_after)) = &source {
        let bytes = fs::read(source)?;
        atomic_write(&directory.join("source"), &bytes)?;
        snapshot.sources.insert(
            source.clone(),
            SourceBackup {
                stored: "source".into(),
                fingerprint: fingerprint(&bytes, executable(source)?),
                after_fingerprint: fingerprint(edited, *executable_after),
                protection: protection::capture(source)?,
            },
        );
    }
    atomic_write(&snapshot_path, &serde_json::to_vec_pretty(&snapshot)?)?;
    ensure!(
        serde_json::to_value(resolution_preview(
            settings,
            &path,
            action,
            source_edit,
            &desired,
            &original
        )?)? == expected,
        "Workspace changed while backing up. Review the action again. Backup: {}",
        directory.display()
    );
    let mut owned = original;
    if let Some((source, bytes, exec)) = source {
        let saved = snapshot
            .sources
            .get(&source)
            .context("Missing source snapshot")?;
        ensure!(
            fingerprint(&fs::read(&source)?, executable(&source)?) == saved.fingerprint,
            "Source changed before copy-back. Backup: {}",
            directory.display()
        );
        write_rendered(&source, &bytes, exec)?;
    }
    if action == ConflictAction::EditSource {
        return Ok(directory);
    }
    for p in paths {
        no_symlinks(&p)?;
        let current = if p.try_exists()? {
            Some(fingerprint(&fs::read(&p)?, executable(&p)?))
        } else {
            None
        };
        ensure!(
            current.as_ref() == captured.files.get(&p),
            "Output changed before resolution. Backup: {}",
            directory.display()
        );
        if action == ConflictAction::Reinstall {
            if let Some(file) = desired.files.get(&p) {
                write_rendered(&p, &file.bytes, file.executable)?;
            } else if p.try_exists()? {
                protection::capture(&p)?;
                fs::remove_file(&p)?;
            }
        }
        if p.try_exists()? {
            owned
                .files
                .insert(p.clone(), fingerprint(&fs::read(&p)?, executable(&p)?));
        } else {
            owned.files.remove(&p);
        }
    }
    if desired.roots.contains(&path) {
        owned.roots.insert(path);
    }
    owned.project_roots = desired.project_roots;
    atomic_write(
        &manifest_path(settings),
        &serde_json::to_vec_pretty(&owned)?,
    )?;
    Ok(directory)
}

pub(super) fn write_rendered(path: &Path, bytes: &[u8], exec: bool) -> Result<()> {
    crate::config::protected_write(path, bytes, Some(exec), None)
}

pub fn read_conflict_source(settings: &Settings, path: &Path) -> Result<crate::SourceDocument> {
    let desired = desired(settings)?;
    let file = desired.files.get(path).context("No mapped source file")?;
    no_symlinks(&file.source)?;
    let bytes = fs::read(&file.source)?;
    Ok(crate::SourceDocument {
        file: file.source.to_string_lossy().into_owned(),
        text: String::from_utf8(bytes.clone())
            .context("Source is binary; edit it outside the text editor")?,
        hash: fingerprint(&bytes, false).sha256,
    })
}

pub fn preview_source_edit(
    settings: &Settings,
    path: &Path,
    text: &str,
) -> Result<ResolutionPreview> {
    let desired = desired(settings)?;
    let owned = manifest(settings, &desired)?;
    let file = desired.files.get(path).context("No mapped source file")?;
    crate::content::validate_source_replacement(settings, &file.source, text.as_bytes())?;
    resolution_preview(
        settings,
        path,
        ConflictAction::EditSource,
        Some(text),
        &desired,
        &owned,
    )
}
