//! Render source content into desired files without changing destinations.
use super::*;

fn skill_markdown(
    source: &str,
    name: &str,
    context: &toml::Table,
    metadata: &toml::Table,
) -> Result<String> {
    let rendered = render(source, context, true)?;
    let rendered = rendered.replace("\r\n", "\n");
    let (mut yaml, body) = crate::template::skill_frontmatter(&rendered, name)?;
    ensure!(!body.trim().is_empty(), "Skill body is empty");
    if let Some(additions) = metadata.get("frontmatter") {
        let frontmatters = additions
            .as_table()
            .context("frontmatter must be a table")?;
        if let Some(selected) = frontmatters.get(string(context, "harness")?) {
            for (key, value) in selected
                .as_table()
                .context("Harness frontmatter must be a table")?
            {
                ensure!(
                    !matches!(key.as_str(), "name" | "description"),
                    "Harness frontmatter cannot replace skill identity"
                );
                ensure!(
                    yaml.get(key.as_str()).is_none(),
                    "Harness frontmatter cannot overwrite shared field {key}"
                );
                yaml.as_mapping_mut()
                    .context("Frontmatter must be a mapping")?
                    .insert(
                        serde_yaml_ng::Value::String(key.clone()),
                        serde_yaml_ng::to_value(value)?,
                    );
            }
        }
    }
    Ok(format!(
        "---\n{}---\n{body}",
        serde_yaml_ng::to_string(&yaml)?
    ))
}

pub(super) struct SkillSources {
    pub(super) metadata: toml::Table,
    pub(super) files: BTreeMap<PathBuf, PathBuf>,
}

pub(super) fn skill_sources(
    skill: &Path,
    skill_name: &str,
    context: &toml::Table,
    draft: Option<SkillDraft<'_>>,
) -> Result<Option<SkillSources>> {
    let metadata = match draft {
        Some(draft) => toml::from_str(draft.metadata)?,
        None => read_toml(&skill.join("skill.toml"))?,
    };
    ensure!(
        metadata
            .keys()
            .all(|k| matches!(k.as_str(), "target" | "frontmatter" | "enabled")),
        "Unknown skill.toml fields in {skill_name}"
    );
    if let Some(enabled) = metadata.get("enabled") {
        ensure!(enabled.is_bool(), "enabled must be a boolean");
        if !enabled.as_bool().unwrap_or(false) {
            return Ok(None);
        }
    }
    if let Some(target) = metadata.get("target")
        && !selectors_match(
            target.as_table().context("Target must be a table")?,
            context,
        )?
    {
        return Ok(None);
    }
    let mut sources = file_paths(skill)?;
    sources.retain(|p, _| p != Path::new("skill.toml") && !p.starts_with("variants"));
    let variants = skill.join("variants");
    let mut replacements = BTreeSet::new();
    if variants.try_exists()? {
        no_symlinks(&variants)?;
        for variant in fs::read_dir(&variants)? {
            let variant = variant?;
            if variant_runtime_entry(&variant)? {
                continue;
            }
            let kind = variant.file_type()?;
            ensure!(
                !kind.is_symlink(),
                "Variant is a symlink: {}",
                variant.file_name().to_string_lossy()
            );
            ensure!(
                kind.is_dir(),
                "Variants must be directories: {}",
                variant.file_name().to_string_lossy()
            );
            let variant_name = variant
                .file_name()
                .into_string()
                .map_err(|_| anyhow::anyhow!("Non-UTF8 variant"))?;
            let (key, value) = variant_name
                .split_once('=')
                .context("Variant requires key=value")?;
            ensure!(
                matches!(key, "harness" | "machine" | "os"),
                "Unknown variant key"
            );
            identifier(value)?;
            let variant_files = file_paths(&variant.path())?;
            if context.get(key).and_then(|v| v.as_str()) == Some(value) {
                for (path, source) in variant_files {
                    ensure!(
                        replacements.insert(path.clone()),
                        "Two matching variants replace {}",
                        path.display()
                    );
                    ensure!(
                        path != Path::new("skill.toml"),
                        "Variants cannot replace skill metadata"
                    );
                    sources.insert(path, source);
                }
            }
        }
    }
    ensure!(
        sources.contains_key(Path::new("SKILL.md")),
        "Skill {skill_name} lacks SKILL.md"
    );
    Ok(Some(SkillSources {
        metadata,
        files: sources,
    }))
}

/// A variant can replace `SKILL.md`, so the draft applies only to the skill's own file.
pub(super) fn skill_file_bytes(
    skill: &Path,
    source: &Path,
    draft: Option<SkillDraft<'_>>,
) -> Result<Vec<u8>> {
    Ok(match draft {
        Some(draft) if source == skill.join("SKILL.md") => draft.markdown.as_bytes().to_vec(),
        _ => fs::read(source)?,
    })
}

pub(super) fn render_skill_file(
    path: &Path,
    source: &Path,
    original: Vec<u8>,
    skill_name: &str,
    harness: &str,
    context: &toml::Table,
    metadata: &toml::Table,
) -> Result<Rendered> {
    let reversible = std::str::from_utf8(&original).is_ok_and(|text| {
        !(text.contains("{{af:")
            || text.contains("<!--")
            || path == Path::new("SKILL.md") && metadata.contains_key("frontmatter"))
    });
    let bytes = original;
    let required_text = matches!(
        path.extension().and_then(|e| e.to_str()),
        Some("md" | "toml" | "py" | "txt")
    );
    let bytes = match std::str::from_utf8(&bytes) {
        Ok(text) if path == Path::new("SKILL.md") => {
            skill_markdown(text, skill_name, context, metadata)?.into_bytes()
        }
        Ok(text) => {
            render(text, context, path.extension().is_some_and(|e| e == "md"))?.into_bytes()
        }
        Err(error) if required_text => {
            return Err(error).context("Required text file is not UTF-8");
        }
        Err(_) => bytes,
    };
    Ok(Rendered {
        harness: harness.into(),
        skill: Some(skill_name.into()),
        source: source.into(),
        reversible,
        bytes,
        executable: executable(source)?,
    })
}

fn render_scope(settings: &Settings, content: &Path, project: Option<&Path>) -> Result<Desired> {
    settings.validate()?;
    render_scope_inner(
        &RenderScope {
            source: &settings.source,
            machine: &settings.machine,
            content,
            project,
            output: Output::Workspace { settings },
            focus: None,
        },
        &mut RenderDiagnostics::default(),
    )
}

#[derive(Clone, Copy)]
pub(super) enum Output<'a> {
    Workspace { settings: &'a Settings },
    Validation,
}

pub(super) struct RenderScope<'a> {
    pub(super) source: &'a Path,
    pub(super) machine: &'a str,
    pub(super) content: &'a Path,
    pub(super) project: Option<&'a Path>,
    pub(super) output: Output<'a>,
    pub(super) focus: Option<ValidationFocus<'a>>,
}

pub(super) fn render_scope_inner(
    scope: &RenderScope<'_>,
    diagnostics: &mut RenderDiagnostics,
) -> Result<Desired> {
    let RenderScope {
        source,
        machine: machine_name,
        ..
    } = *scope;
    no_symlinks(source)?;
    identifier(machine_name)?;
    let machine = read_toml(&source.join("machines").join(format!("{machine_name}.toml")))?;
    let harnesses: BTreeMap<String, Harness> =
        read_toml(&source.join("harnesses.toml"))?.try_into()?;
    let os = string(&machine, "os")?;
    ensure!(crate::SUPPORTED_OS.contains(&os), "Unknown machine OS");
    let mode = machine
        .get("mode")
        .and_then(|v| v.as_str())
        .unwrap_or("staging");
    ensure!(
        matches!(mode, "staging" | "live"),
        "Mode must be staging or live"
    );
    let enabled = machine
        .get("harnesses")
        .and_then(|v| v.as_array())
        .context("Machine must enable harnesses")?;
    ensure!(
        !enabled.is_empty(),
        "Machine must enable at least one harness"
    );
    let mut result = Desired {
        files: BTreeMap::new(),
        roots: BTreeSet::new(),
        allowed: vec![],
        install_targets: vec![],
        mode: mode.into(),
        skills: 0,
        targets: enabled.len(),
        project_roots: BTreeMap::new(),
        project_statuses: vec![],
    };
    let mut base_roots = Vec::new();
    let profile_path = source.join("machines").join(format!("{machine_name}.toml"));
    for harness_name in enabled {
        let name_result = harness_name
            .as_str()
            .with_context(|| format!("Harness name must be a string: {harness_name}"));
        let Some(name) =
            diagnostics.capture(name_result, machine_name, None, None, &profile_path)?
        else {
            continue;
        };
        let harness_result = render_harness(
            scope,
            &machine,
            name,
            &harnesses,
            &mut result,
            &mut base_roots,
            diagnostics,
        );
        if let Some(summary) = diagnostics.capture(
            harness_result,
            machine_name,
            Some(name),
            None,
            &source.join("harnesses.toml"),
        )? {
            diagnostics.harnesses.push(summary);
        }
    }
    Ok(result)
}

struct HarnessRender<'a, 's> {
    scope: &'a RenderScope<'s>,
    name: &'a str,
    definition: &'a Harness,
    context: &'a toml::Table,
    os: &'a str,
    mode: &'a str,
}

fn harness_base(
    rendering: &HarnessRender<'_, '_>,
    diagnostics: &mut RenderDiagnostics,
) -> Result<PathBuf> {
    let HarnessRender {
        scope,
        name,
        definition: harness,
        context,
        os,
        mode,
    } = *rendering;
    let RenderScope {
        source,
        machine: machine_name,
        project,
        output,
        ..
    } = *scope;
    let settings = match output {
        Output::Workspace { settings } => Some(settings),
        Output::Validation => None,
    };
    // Validation uses logical destinations only. They are never inspected or created.
    Ok(if let Some(settings) = settings {
        if mode == "staging" {
            let staging = settings.state_dir.join(&settings.machine).join("staging");
            if let Some(project) = project {
                staging
                    .join("projects")
                    .join(format!(
                        "{:x}",
                        Sha256::digest(project.to_string_lossy().as_bytes())
                    ))
                    .join(name)
            } else {
                staging.join(name)
            }
        } else if let Some(project) = project {
            ensure!(os == host_os(), "Project profile does not match host OS");
            project.join(relative(
                harness
                    .project_root
                    .as_deref()
                    .unwrap_or(&format!(".{name}")),
            )?)
        } else {
            ensure!(
                os == host_os(),
                "Cannot apply {os} profile on {}",
                host_os()
            );
            let home = Path::new(string(context, "home")?);
            let actual_home = directories::BaseDirs::new()
                .context("Cannot find home")?
                .home_dir()
                .canonicalize()?;
            ensure!(
                home.canonicalize()? == actual_home,
                "Live machine profile belongs to another home"
            );
            let root = harness
                .roots
                .get(os)
                .context("Harness lacks a root for machine OS")?;
            PathBuf::from(render(root, context, false)?)
        }
    } else {
        if mode == "live" {
            let root = harness
                .roots
                .get(os)
                .context("Harness lacks a root for machine OS")
                .and_then(|root| render(root, context, false))
                .and_then(|root| validation::validate_root(&root, os));
            diagnostics.capture(
                root,
                machine_name,
                Some(name),
                None,
                &source.join("harnesses.toml"),
            )?;
            if let Some(root) = &harness.global_root {
                let root = render(root, context, false)
                    .and_then(|root| validation::validate_root(&root, os));
                diagnostics.capture(
                    root,
                    machine_name,
                    Some(name),
                    None,
                    &source.join("harnesses.toml"),
                )?;
            }
        }
        if let Some(root) = &harness.project_root {
            diagnostics.capture(
                relative(root),
                machine_name,
                Some(name),
                None,
                &source.join("harnesses.toml"),
            )?;
        }
        source.join(".validation").join(name)
    })
}

fn render_harness(
    scope: &RenderScope<'_>,
    machine: &toml::Table,
    name: &str,
    harnesses: &BTreeMap<String, Harness>,
    result: &mut Desired,
    base_roots: &mut Vec<PathBuf>,
    diagnostics: &mut RenderDiagnostics,
) -> Result<HarnessValidation> {
    let RenderScope {
        machine: machine_name,
        content,
        project,
        output,
        ..
    } = *scope;
    let mut summary = HarnessValidation::new(name);
    identifier(name)?;
    let harness = harnesses
        .get(name)
        .with_context(|| format!("Unknown harness: {name}"))?;
    let mut context = machine.clone();
    context.insert("machine".into(), machine_name.into());
    context.insert("harness".into(), name.into());
    let os = string(machine, "os")?;
    let mode = result.mode.clone();
    let mode = mode.as_str();
    let rendering = &HarnessRender {
        scope,
        name,
        definition: harness,
        context: &context,
        os,
        mode,
    };
    let settings = match output {
        Output::Workspace { settings } => Some(settings),
        Output::Validation => None,
    };
    let base = harness_base(rendering, diagnostics)?;
    ensure!(base.is_absolute(), "Output root must be absolute");
    if settings.is_some() {
        no_symlinks(&base)?;
    }
    if let Some(settings) = settings {
        ensure!(!overlaps(&base, content), "Output overlaps source checkout");
        if mode == "live" {
            ensure!(
                !overlaps(&base, &settings.state_dir),
                "Output overlaps state"
            );
        }
    }
    ensure!(
        !base_roots
            .iter()
            .any(|other: &PathBuf| overlaps(other, &base)),
        "Harness output roots overlap"
    );
    base_roots.push(base.clone());
    let skills_base = base.join(relative(&harness.skills_dir)?);
    // Validation only checks logical destinations, so it never leaves the output root.
    let global_base = if mode == "live" && settings.is_some() && project.is_none() {
        if let Some(root) = &harness.global_root {
            PathBuf::from(render(root, &context, false)?)
        } else {
            base.clone()
        }
    } else {
        base.clone()
    };
    ensure!(
        global_base.is_absolute(),
        "Global instruction root must be absolute"
    );
    if settings.is_some() {
        no_symlinks(&global_base)?;
    }
    if let Some(settings) = settings {
        ensure!(
            !overlaps(&global_base, content),
            "Instruction root overlaps source"
        );
        if mode == "live" {
            ensure!(
                !overlaps(&global_base, &settings.state_dir),
                "Instruction root overlaps state"
            );
        }
    }
    let global_file = global_base.join(relative(&harness.global_file)?);
    for (other_skills, other_global) in &result.allowed {
        ensure!(
            !overlaps(&skills_base, other_skills)
                && !overlaps(&skills_base, other_global)
                && !overlaps(&global_file, other_skills)
                && global_file != *other_global,
            "Harness destinations overlap"
        );
    }
    ensure!(
        !global_file.starts_with(&skills_base),
        "Global file overlaps skills directory"
    );
    result
        .allowed
        .push((skills_base.clone(), global_file.clone()));
    result.install_targets.push(InstallTarget {
        harness: name.into(),
        skills_root: skills_base.clone(),
        instructions_file: global_file.clone(),
        project: project.map(Path::to_path_buf),
    });
    let skills_dir = content.join("skills");
    no_symlinks(&skills_dir)?;
    let skill_focus = match scope.focus {
        Some(ValidationFocus::Skill { name, .. }) => Some(name),
        _ => None,
    };
    let render_skills = !matches!(
        scope.focus,
        Some(ValidationFocus::GlobalInstructions { .. })
    );
    if render_skills && skills_dir.try_exists()? {
        let mut names = BTreeSet::new();
        for skill in fs::read_dir(&skills_dir)? {
            let skill = match diagnostics.capture(
                skill.map_err(Into::into),
                machine_name,
                Some(name),
                None,
                &skills_dir,
            )? {
                Some(skill) => skill,
                None => continue,
            };
            let skill_path = skill.path();
            // `.DS_Store` and `.pyc` files are ignored inside a skill; ignore them at the root too.
            if skills_root_runtime_file(&skill)? {
                continue;
            }
            let skill_label = skill.file_name().to_string_lossy().into_owned();
            if skill_focus.is_some_and(|focus| focus != skill_label) {
                continue;
            }
            let skill_result = render_skill(
                rendering,
                &skill,
                &skills_base,
                &mut names,
                result,
                &mut summary,
                diagnostics,
            );
            if let Some(false) = diagnostics.capture(
                skill_result,
                machine_name,
                Some(name),
                Some(&skill_label),
                &skill_path,
            )? {
                summary.skills_skipped += 1;
            }
        }
    }

    if skill_focus.is_none() {
        let global = content.join(crate::content::GLOBAL_INSTRUCTIONS);
        let global_result = render_global(rendering, &global, global_file, result);
        if let Some(rendered) =
            diagnostics.capture(global_result, machine_name, Some(name), None, &global)?
        {
            summary.global_instructions_rendered = rendered;
        }
    }
    Ok(summary)
}

fn render_skill(
    rendering: &HarnessRender<'_, '_>,
    skill: &fs::DirEntry,
    skills_base: &Path,
    names: &mut BTreeSet<String>,
    result: &mut Desired,
    summary: &mut HarnessValidation,
    diagnostics: &mut RenderDiagnostics,
) -> Result<bool> {
    let HarnessRender {
        scope,
        name,
        context,
        ..
    } = *rendering;
    let machine_name = scope.machine;
    let skill_path = skill.path();
    require_skill_directory(skill)?;
    let skill_name = skill
        .file_name()
        .into_string()
        .map_err(|_| anyhow::anyhow!("Non-UTF8 skill name"))?;
    identifier(&skill_name)?;
    ensure_skill_name_allowed(&skill_name)?;
    ensure!(
        names.insert(skill_name.to_lowercase()),
        "Duplicate skill name"
    );
    let draft = match scope.focus {
        Some(ValidationFocus::Skill { draft, .. }) => draft,
        _ => None,
    };
    let Some(SkillSources {
        metadata,
        files: sources,
    }) = skill_sources(&skill_path, &skill_name, context, draft)?
    else {
        return Ok(false);
    };
    let root = skills_base.join(&skill_name);
    result.roots.insert(root.clone());
    let mut clean = true;
    let mut cases = BTreeSet::new();
    for (path, source) in sources {
        let file_result = render_skill_output(
            &SkillRender {
                harness: rendering,
                name: &skill_name,
                root: &root,
                skill: &skill_path,
                metadata: &metadata,
                draft,
            },
            &path,
            &source,
            &mut cases,
            result,
        );
        if diagnostics
            .capture(
                file_result,
                machine_name,
                Some(name),
                Some(&skill_name),
                &source,
            )?
            .is_some()
        {
            summary.files_rendered += 1;
        } else {
            clean = false;
        }
    }
    if clean {
        summary.skills_rendered += 1;
        result.skills += 1;
    }
    Ok(true)
}

struct SkillRender<'a, 's> {
    harness: &'a HarnessRender<'a, 's>,
    name: &'a str,
    root: &'a Path,
    skill: &'a Path,
    metadata: &'a toml::Table,
    draft: Option<SkillDraft<'a>>,
}

pub(super) fn ensure_skill_name_allowed(name: &str) -> Result<()> {
    ensure!(
        !matches!(name, "synced" | "anthropic-skills"),
        "Reserved skill name"
    );
    Ok(())
}

fn render_skill_output(
    skill: &SkillRender<'_, '_>,
    path: &Path,
    source: &Path,
    cases: &mut BTreeSet<String>,
    result: &mut Desired,
) -> Result<()> {
    let SkillRender {
        harness,
        name: skill_name,
        root,
        skill: skill_path,
        metadata,
        draft,
    } = *skill;
    let HarnessRender { name, context, .. } = *harness;
    ensure!(
        cases.insert(path.to_string_lossy().to_lowercase()),
        "Variant filename collision"
    );
    ensure!(
        result
            .files
            .insert(
                root.join(path),
                render_skill_file(
                    path,
                    source,
                    skill_file_bytes(skill_path, source, draft)?,
                    skill_name,
                    name,
                    context,
                    metadata,
                )?
            )
            .is_none(),
        "Duplicate output file"
    );
    Ok(())
}

fn render_global(
    rendering: &HarnessRender<'_, '_>,
    global: &Path,
    global_file: PathBuf,
    result: &mut Desired,
) -> Result<bool> {
    let HarnessRender {
        scope,
        name,
        context,
        ..
    } = *rendering;
    let project = scope.project;
    no_symlinks(global)?;
    let draft = match scope.focus {
        Some(ValidationFocus::GlobalInstructions { draft }) => draft,
        _ => None,
    };
    if project.is_none() && (draft.is_some() || global.try_exists()?) {
        let text = match draft {
            Some(draft) => draft.to_owned(),
            None => fs::read_to_string(global)?,
        };
        result.files.insert(
            global_file,
            Rendered {
                harness: name.into(),
                skill: None,
                source: global.into(),
                reversible: !text.contains("{{af:") && !text.contains("<!--"),
                bytes: render_global_instructions(&text, context)?.into_bytes(),
                executable: false,
            },
        );
        return Ok(true);
    }
    Ok(false)
}

/// The single renderer for global instructions, shared by deployment and editor previews.
pub(crate) fn render_global_instructions(text: &str, context: &toml::Table) -> Result<String> {
    render(text, context, true)
}

pub(super) fn desired(settings: &Settings) -> Result<Desired> {
    let mut result = render_scope(settings, &settings.source, None)?;
    let mut statuses = Vec::with_capacity(settings.projects.len());
    for root in &settings.projects {
        let status = project_status(root);
        ensure!(
            status.error.is_none(),
            "Project {} cannot be rendered: {:?}",
            root.display(),
            status.error
        );
        let content = status
            .source
            .clone()
            .context("Project has no skill source")?;
        statuses.push(status);
        let project = render_scope(settings, &content, Some(root))?;
        result
            .project_roots
            .insert(root.clone(), project.roots.clone());
        for (path, file) in project.files {
            ensure!(
                result.files.insert(path.clone(), file).is_none(),
                "Project/global output collision: {}",
                path.display()
            );
        }
        result.roots.extend(project.roots);
        result.allowed.extend(project.allowed);
        result.install_targets.extend(project.install_targets);
        result.skills += project.skills;
        result.targets += project.targets;
    }
    result.project_statuses = statuses;
    Ok(result)
}
