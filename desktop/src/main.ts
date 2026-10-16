import { createLivePreview } from "./live-preview";
import { invoke, isTauri } from "@tauri-apps/api/core";
import "./style.css";
import {
  runNightlyUpdate,
  runStartupUpdate,
  hasUnsavedEdits,
  updateStatusText,
  type UpdateStatus,
} from "./update-policy";

interface Project {
  root: string;
  changed_files: number;
  outgoing_commits: number | null;
  upstream: string | null;
  error: string | null;
}
interface Change {
  path: string;
  action: string;
  diff: string | null;
  executable?: boolean | null;
}
interface Conflict {
  path: string;
  kind: string;
  diff: string | null;
  source: string | null;
  copy_back_blocked: string | null;
}
interface AgentTarget {
  tag: string;
  global_folder: string;
  project_folder: string;
  instructions_file: string;
  instructions_folder: string | null;
}
interface AgentPreset {
  title: string;
  target: AgentTarget;
  sources: string[];
}
interface JobStatus {
  last_run: {
    started_at: number;
    finished_at: number | null;
    error: string | null;
  } | null;
  current_error: string | null;
  helper_update_error: string | null;
  desktop_update_error: string | null;
  log: string;
}
interface Plan {
  source: string;
  machine: string;
  mode: string;
  skills: number;
  targets: number;
  changes: Change[];
  conflicts: string[];
  conflict_details: Conflict[];
  projects: Project[];
}
interface Skill {
  name: string;
  enabled: boolean;
  project: string | null;
}
interface SkillDocument {
  name: string;
  markdown: string;
  metadata: string;
  markdown_hash: string;
  metadata_hash: string;
}
interface Snapshot {
  config_path: string;
  config_text: string;
  plan: Plan;
  skills: Skill[];
  helper: string | null;
  helper_error: string | null;
  jobs: JobStatus;
  presets: AgentPreset[];
  preview_targets: { machine: string; harness: string }[];
  source_files: string[];
  updates: UpdateStatus;
}

const app = document.querySelector<HTMLElement>("#app");
if (!app) throw new Error("Missing app root");
app.inert = true;
app.innerHTML = `
  <aside><div class="brand"><img src="/ratatoskr.png" alt="Ratatoskr the squirrel"><span>Ratatoskr<small>SKILL SYNC</small></span></div><p class="eyebrow">ONE SOURCE. YOUR WHOLE GROVE.</p><nav aria-label="Workspace"><a href="#overview">Overview</a><a href="#changes">Pending changes</a><a href="#skills">Source skills</a><a href="#projects">Projects</a><a href="#automation">Background sync</a><a href="#configuration">Configuration</a></nav><p class="aside-note">Run the helper with this window closed.</p></aside>
  <section class="workspace"><header id="overview"><div><p class="eyebrow">YOUR AGENT WORKSPACE</p><h1>Everything, in its place.</h1><p id="subtitle">Connect a skill repository to see what needs syncing.</p></div><button id="refresh" class="secondary">Refresh</button></header>
  <div id="message" role="status" aria-live="polite"></div>
  <section id="setup" class="panel" hidden><h2>Connect your content</h2><p>Connect an existing checkout, clone a Git repository, or start an empty content repository with Claude and Codex presets. New content starts in staging.</p><form id="connect-form"><label>Local content directory<input id="source" autocomplete="off" required></label><label>Machine profile<input id="machine" autocomplete="off" required pattern="[a-z0-9]+(-[a-z0-9]+)*"></label><label>Git URL (optional)<input id="remote" autocomplete="off"></label><label class="check"><input type="checkbox" id="create-content">Create empty content repository</label><button type="submit">Connect and preview</button></form></section>
  <section class="stats" aria-label="Workspace status"><article><span>SKILL INSTALLATIONS</span><strong id="skill-count">—</strong></article><article><span>AGENT TARGETS</span><strong id="target-count">—</strong></article><article><span>PENDING FILES</span><strong id="change-count">—</strong></article><article><span>INSTALL MODE</span><strong id="mode">—</strong></article></section>
  <section id="changes" class="panel"><div class="section-heading"><div><p class="eyebrow">REVIEW BEFORE YOU SYNC</p><h2>Pending changes</h2></div><button id="apply" disabled>Apply reviewed changes</button></div><div id="conflicts"></div><section id="conflict-source-editor" hidden><h3 id="conflict-source-title"></h3><label>Mapped source<textarea id="conflict-source-text" rows="14" spellcheck="false"></textarea></label><button id="preview-source-edit" class="secondary">Validate and preview source edit</button></section><section id="resolution-preview" hidden><h3>Review conflict action</h3><p id="resolution-explanation"></p><pre id="resolution-effects"></pre><button id="confirm-resolution">Back up and execute this action</button><button id="cancel-resolution" class="secondary">Cancel</button></section><div id="change-list" class="empty">Connect your content to preview changes.</div></section>
  <section id="skills" class="panel"><p class="eyebrow">EDIT THE SOURCE</p><h2>Skills in your grove</h2><div id="skill-list" class="empty">Connect content to manage skills.</div><form id="new-skill-form"><label>New skill name<input id="new-skill-name" required pattern="[a-z0-9]+(-[a-z0-9]+)*"></label><label>Location<select id="new-skill-project"><option value="">Global content</option></select></label><label class="full">Skill markdown, including YAML name and description<textarea id="new-skill-body" rows="6" required spellcheck="false"></textarea></label><button type="submit">Validate and create skill</button></form><section id="skill-editor" hidden><h3 id="editor-title"></h3><label>Source file<select id="editor-file"><option>SKILL.md</option><option>skill.toml</option></select></label><label>Contents<textarea id="editor-text" rows="15" spellcheck="false"></textarea></label><button id="save-skill">Validate and save source</button><label>Preview target<select id="preview-target"></select></label><p id="editor-validation" role="status" aria-live="polite"></p><pre id="editor-preview"></pre><p>Conditions use only comment blocks. Variables use the af prefix. Target selection and per-harness frontmatter live in skill.toml.</p></section></section>
  <section id="projects" class="panel"><p class="eyebrow">DON’T LEAVE SKILLS BEHIND</p><h2>Monitored projects</h2><p>Track skill files that still need a commit or push.</p><div id="project-list" class="empty">No projects registered.</div><form id="project-form"><label>Git repository root<input id="project-root" autocomplete="off" required></label><label>Skill source folder inside project<input id="project-source" value="agent-skills" required></label><button type="submit" class="secondary">Monitor project</button></form></section>
  <section id="configuration" class="panel"><div class="section-heading"><div><p class="eyebrow">YOUR MACHINE, YOUR RULES</p><h2>Configuration</h2></div><button id="save-config" class="secondary" disabled>Validate and save</button></div><p id="config-path"></p><label for="config-text">Local TOML configuration</label><textarea id="config-text" rows="11" spellcheck="false" disabled></textarea></section>
  <section class="panel"><h2>Agent presets and machine profiles</h2><form id="agent-form"><label>Agent<select id="agent-preset"><option value="">Custom agent</option></select></label><label>Tag<input id="agent-tag" required pattern="[a-z0-9]+(-[a-z0-9]+)*"></label><label>Global folder (absolute or home template)<input id="agent-global" required></label><label>Project subfolder<input id="agent-project" required></label><label>Instructions filename<input id="agent-instructions" required></label><label>Separate global instructions folder (optional)<input id="agent-instructions-folder"></label><button type="submit" class="secondary">Preview agent definition</button></form><p id="preset-sources"></p><pre id="agent-preview" hidden></pre><button id="confirm-agent" disabled>Save reviewed agent definition</button><p>New definitions are available to machine profiles. Enable a tag in a profile’s harnesses list. Presets use default locations; custom environment overrides can be edited below.</p><p>Edit harness definitions, machine conditions, or shared global instructions in your content repository.</p><label>Source file<select id="source-file"></select></label><button id="open-source" class="secondary">Open source file</button><label>Contents<textarea id="source-text" rows="12" spellcheck="false" disabled></textarea></label><button id="save-source" disabled>Validate and save source file</button></section>
  <section id="automation" class="panel"><p class="eyebrow">THE MESSENGER KEEPS RUNNING</p><h2>Background sync</h2><p>The helper runs independently through launchd on Mac or Task Scheduler on Windows. Use a clean deployment checkout with a Git upstream. Configure alert_command in local TOML for failure and recovery notifications.</p><form id="schedule-form"><p id="helper-path"></p><label>Interval in seconds<input id="schedule-seconds" type="number" min="60" max="86400" value="60" required></label><button type="button" id="preview-schedule" class="secondary">Preview schedule</button><button id="install-schedule" type="submit">Install background schedule</button><button type="button" id="remove-schedule" class="secondary">Remove schedule</button></form><pre id="schedule-preview" hidden></pre><button id="sync-now" class="secondary">Pull and sync now</button><button id="refresh-job-status" class="secondary">Refresh job status</button><p id="job-result" role="status"></p><pre id="job-error"></pre><details><summary>Background log</summary><pre id="job-log"></pre></details></section>
  <section class="panel"><h2>Adoption and recovery</h2><p>Use the per-conflict action previews above to adopt existing target skills. Review the resulting plan before applying: source files will replace adopted copies, and adopted files absent from source will be removed. Restore only verified backups from this workspace.</p><form id="restore-form"><label>Backup directory<input id="backup-path" required></label><button type="submit" class="secondary">Restore backup</button></form></section>
  <section class="panel"><h2>Signed updates</h2><label class="check"><input id="open-at-login" type="checkbox">Open at login</label><p>Off by default. Enable to let the desktop catch missed nightly updates after a reboot.</p><p>Official updates use the built-in signing key. Automatic updates wait for idle work, then install and relaunch this workspace. Unsaved edits defer updates for up to one hour. Launching the app catches up on a missed update before editing starts. Forks can explicitly override endpoint and public_key in local TOML.</p><p id="update-status" role="status"></p><form id="update-schedule-form"><label class="check"><input id="automatic-updates" type="checkbox" checked>Enable nightly updates</label><label>Local update time<input id="update-time" type="time" value="00:00" required></label><button type="submit">Save update schedule</button></form><div class="button-row"><button id="check-updates" class="secondary">Check desktop updates</button><button id="install-update" class="secondary">Install signed desktop update</button><button id="restart" class="secondary">Restart app</button></div></section>
  </section>`;

function element<T extends HTMLElement>(id: string): T {
  const found = document.getElementById(id);
  if (!found) throw new Error(`Missing element ${id}`);
  return found as T;
}
let snapshot: Snapshot | null = null;
let busy = false;
let configDirty = false;
let sourceDocument: { file: string; text: string; hash: string } | null = null;
let editing: {
  skill: Skill;
  document: SkillDocument;
  saved: SkillDocument;
} | null = null;
function message(text: string, error = false): void {
  const target = element("message");
  target.textContent = text;
  target.className = error ? "message error" : "message";
}
function textRow(title: string, detail: string): HTMLElement {
  const row = document.createElement("article");
  row.className = "row";
  const heading = document.createElement("strong");
  heading.textContent = title;
  const description = document.createElement("p");
  description.textContent = detail;
  row.append(heading, description);
  return row;
}
function render(result: Snapshot): void {
  snapshot = result;
  resolution = null;
  sourceEditPath = null;
  element("conflict-source-editor").hidden = true;
  element("resolution-preview").hidden = true;
  const targets = element<HTMLSelectElement>("preview-target");
  const chosen = targets.value;
  targets.replaceChildren(
    ...result.preview_targets.map(
      (t) =>
        new Option(`${t.machine} × ${t.harness}`, `${t.machine}/${t.harness}`),
    ),
  );
  if (
    result.preview_targets.some((t) => `${t.machine}/${t.harness}` === chosen)
  )
    targets.value = chosen;
  const presets = element<HTMLSelectElement>("agent-preset");
  const selectedPreset = presets.value;
  presets.replaceChildren(
    new Option("Custom agent", ""),
    ...result.presets.map((p) => new Option(p.title, p.target.tag)),
  );
  presets.value = selectedPreset;
  showJobStatus(result.jobs);
  element("helper-path").textContent = result.helper
    ? `Detected helper: ${result.helper}`
    : result.helper_error;
  const plan = result.plan;
  element("setup").hidden = true;
  element("subtitle").textContent = `${plan.machine} · ${plan.source}`;
  element("skill-count").textContent = String(plan.skills);
  element("target-count").textContent = String(plan.targets);
  element("change-count").textContent = String(plan.changes.length);
  element("mode").textContent = plan.mode;
  const changes = element("change-list");
  changes.replaceChildren();
  changes.className = "";
  for (const change of plan.changes) {
    const details = document.createElement("details");
    const summary = document.createElement("summary");
    summary.textContent = `${change.action.toUpperCase()}  ${change.path}`;
    const diff = document.createElement("pre");
    diff.textContent =
      change.diff ?? "Binary or large file. Review the source before applying.";
    details.append(summary, diff);
    changes.append(details);
  }
  if (!plan.changes.length) {
    changes.className = "empty";
    changes.textContent = plan.conflicts.length
      ? "Resolve the reported problems before applying."
      : "All generated files are up to date.";
  }
  const conflicts = element("conflicts");
  conflicts.replaceChildren();
  for (const conflict of plan.conflict_details)
    conflicts.append(conflictRow(conflict));
  for (const conflict of plan.conflict_details.length ? [] : plan.conflicts) {
    const row = textRow("Sync paused: protect your edits", conflict);
    row.classList.add("conflict");
    conflicts.append(row);
  }
  const projects = element("project-list");
  projects.replaceChildren();
  projects.className = "";
  for (const project of plan.projects) {
    const status =
      project.error ??
      `${project.changed_files} uncommitted files · ${project.outgoing_commits === null ? "No upstream configured" : `${project.outgoing_commits} unpushed skill commits`}`;
    const row = textRow(project.root, status);
    const remove = document.createElement("button");
    remove.className = "secondary";
    remove.textContent = "Stop monitoring";
    remove.addEventListener("click", () => {
      void operation(async () =>
        render(
          await invoke<Snapshot>("remove_monitored_project", {
            root: project.root,
          }),
        ),
      );
    });
    row.append(remove);
    projects.append(row);
  }
  if (!plan.projects.length) {
    projects.className = "empty";
    projects.textContent = "No projects registered.";
  }
  element("update-status").textContent = updateStatusText(result.updates);
  element("update-status").className = result.updates.override_active
    ? "message error"
    : "message";
  element<HTMLInputElement>("automatic-updates").checked =
    result.updates.automatic;
  element<HTMLInputElement>("update-time").value = result.updates.time;
  element("config-path").textContent = result.config_path;
  if (!configDirty)
    element<HTMLTextAreaElement>("config-text").value = result.config_text;
  const files = element<HTMLSelectElement>("source-file");
  const sourceSelected = files.value;
  files.replaceChildren(
    ...result.source_files.map((file) => new Option(file, file)),
  );
  if (result.source_files.includes(sourceSelected))
    files.value = sourceSelected;
  element<HTMLTextAreaElement>("config-text").disabled = false;
  element<HTMLButtonElement>("save-config").disabled = false;
  element<HTMLButtonElement>("apply").disabled =
    !plan.changes.length || !!plan.conflicts.length;
  const locations = element<HTMLSelectElement>("new-skill-project");
  const selected = locations.value;
  locations.replaceChildren(new Option("Global content", ""));
  for (const project of plan.projects)
    locations.add(new Option(project.root, project.root));
  locations.value = selected;
  const skills = element("skill-list");
  skills.replaceChildren();
  skills.className = "";
  for (const skill of result.skills) {
    const row = textRow(
      skill.name,
      `${skill.project ?? "Global content"} · ${skill.enabled ? "Enabled" : "Disabled"}`,
    );
    const edit = document.createElement("button");
    edit.className = "secondary";
    edit.textContent = "Edit";
    edit.addEventListener("click", () => {
      void operation(async () => {
        const document = await invoke<SkillDocument>("read_source_skill", {
          name: skill.name,
          project: skill.project,
        });
        livePreview.cancel();
        editing = { skill, document, saved: { ...document } };
        element("skill-editor").hidden = false;
        element("editor-title").textContent = skill.name;
        element<HTMLSelectElement>("editor-file").value = "SKILL.md";
        element<HTMLTextAreaElement>("editor-text").value = document.markdown;
        schedulePreview();
        element("skill-editor").scrollIntoView({ block: "center" });
      });
    });
    const toggle = document.createElement("button");
    toggle.className = "secondary";
    toggle.textContent = skill.enabled ? "Disable" : "Enable";
    toggle.addEventListener("click", () => {
      void operation(async () =>
        render(
          await invoke<Snapshot>("set_skill_enabled", {
            name: skill.name,
            project: skill.project,
            enabled: !skill.enabled,
          }),
        ),
      );
    });
    const archive = document.createElement("button");
    archive.className = "secondary";
    archive.textContent = "Archive";
    archive.addEventListener("click", () => {
      void operation(async () => {
        const path = await invoke<string>("archive_source_skill", {
          name: skill.name,
          project: skill.project,
        });
        await refresh();
        message(
          `Skill preserved in ${path}. Review output removals before applying.`,
        );
        if (
          editing?.skill.name === skill.name &&
          editing.skill.project === skill.project
        ) {
          livePreview.cancel();
          editing = null;
          element("skill-editor").hidden = true;
        }
      });
    });
    row.append(edit, toggle, archive);
    skills.append(row);
  }
  if (!result.skills.length) {
    skills.className = "empty";
    skills.textContent = "No skills yet. Create your first source skill below.";
  }
}
async function operation(action: () => Promise<void>): Promise<void> {
  if (busy) return;
  busy = true;
  element<HTMLInputElement>("open-at-login").disabled = true;
  const buttons = Array.from(
    document.querySelectorAll<HTMLButtonElement>("button"),
  );
  const previous = buttons.map((button) => button.disabled);
  buttons.forEach((button) => {
    button.disabled = true;
  });
  try {
    await action();
  } catch (error) {
    message(String(error), true);
  } finally {
    busy = false;
    element<HTMLInputElement>("open-at-login").disabled = !isTauri();
    buttons.forEach((button, index) => {
      button.disabled = previous[index];
    });
    element<HTMLButtonElement>("apply").disabled =
      !snapshot?.plan.changes.length || !!snapshot.plan.conflicts.length;
    element<HTMLButtonElement>("save-config").disabled = !snapshot;
    availableControls();
  }
}
async function refresh(): Promise<void> {
  if (!isTauri()) {
    message(
      "Open the desktop app to connect local content. This browser preview cannot access your files.",
    );
    return;
  }
  element<HTMLInputElement>("open-at-login").checked =
    await invoke<boolean>("login_enabled");
  const result = await invoke<Snapshot | null>("read_workspace");
  if (result) {
    render(result);
    message("Workspace refreshed.");
  } else {
    element("setup").hidden = false;
    message("Connect a content checkout to begin.");
  }
}
element("config-text").addEventListener("input", () => {
  configDirty = true;
});
function availableControls(): void {
  for (const button of Array.from(
    document.querySelectorAll<HTMLButtonElement>("button"),
  )) {
    button.disabled =
      !isTauri() ||
      (!snapshot && button.id !== "refresh" && !button.closest("#setup"));
  }
  element<HTMLButtonElement>("apply").disabled =
    !snapshot?.plan.changes.length || !!snapshot.plan.conflicts.length;
  element<HTMLButtonElement>("confirm-agent").disabled =
    !isTauri() || !agentDraft;
  element<HTMLButtonElement>("confirm-resolution").disabled =
    !isTauri() || !resolution;
  element<HTMLButtonElement>("preview-schedule").disabled = !snapshot?.helper;
  element<HTMLButtonElement>("install-schedule").disabled = !snapshot?.helper;
  element<HTMLButtonElement>("save-source").disabled =
    !sourceDocument || !snapshot;
}
element("refresh").addEventListener("click", () => {
  void operation(refresh);
});
element("connect-form").addEventListener("submit", (event) => {
  event.preventDefault();
  void operation(async () => {
    render(
      await invoke<Snapshot>("connect_workspace", {
        source: element<HTMLInputElement>("source").value,
        machine: element<HTMLInputElement>("machine").value,
        create: element<HTMLInputElement>("create-content").checked,
        remote: element<HTMLInputElement>("remote").value || null,
      }),
    );
    message("Content connected. Review changes before applying.");
  });
});
element("apply").addEventListener("click", () => {
  void operation(async () => {
    if (!snapshot) return;
    render(
      await invoke<Snapshot>("apply_workspace", {
        expectedPlan: JSON.stringify(snapshot.plan),
      }),
    );
    message("Changes applied. Ownership checks passed.");
  });
});
element("save-config").addEventListener("click", () => {
  void operation(async () => {
    const result = await invoke<Snapshot>("save_configuration", {
      text: element<HTMLTextAreaElement>("config-text").value,
    });
    configDirty = false;
    render(result);
    message("Configuration validated and saved. No skills were applied.");
  });
});
element("project-form").addEventListener("submit", (event) => {
  event.preventDefault();
  void operation(async () => {
    render(
      await invoke<Snapshot>("add_monitored_project", {
        root: element<HTMLInputElement>("project-root").value,
        source: element<HTMLInputElement>("project-source").value,
      }),
    );
    message("Project registered. Its files will be checked on refresh.");
  });
});
element("editor-text").addEventListener("input", () => {
  if (!editing) return;
  if (element<HTMLSelectElement>("editor-file").value === "SKILL.md")
    editing.document.markdown =
      element<HTMLTextAreaElement>("editor-text").value;
  else
    editing.document.metadata =
      element<HTMLTextAreaElement>("editor-text").value;
  schedulePreview();
});
element("editor-file").addEventListener("change", () => {
  if (!editing) return;
  element<HTMLTextAreaElement>("editor-text").value =
    element<HTMLSelectElement>("editor-file").value === "SKILL.md"
      ? editing.document.markdown
      : editing.document.metadata;
  schedulePreview();
});
element("save-skill").addEventListener("click", () => {
  void operation(async () => {
    if (!editing) return;
    const file = element<HTMLSelectElement>("editor-file").value;
    render(
      await invoke<Snapshot>("save_source_skill", {
        name: editing.skill.name,
        project: editing.skill.project,
        file,
        text: element<HTMLTextAreaElement>("editor-text").value,
        expectedHash:
          file === "SKILL.md"
            ? editing.document.markdown_hash
            : editing.document.metadata_hash,
      }),
    );
    const saved = await invoke<SkillDocument>("read_source_skill", {
      name: editing.skill.name,
      project: editing.skill.project,
    });
    if (file === "SKILL.md") {
      editing.document.markdown = saved.markdown;
      editing.document.markdown_hash = saved.markdown_hash;
    } else {
      editing.document.metadata = saved.metadata;
      editing.document.metadata_hash = saved.metadata_hash;
    }
    editing.saved = { ...saved };
    schedulePreview();
    message(
      "Source saved and syntax validated. Review the resulting output changes.",
    );
  });
});
element("new-skill-form").addEventListener("submit", (event) => {
  event.preventDefault();
  void operation(async () => {
    render(
      await invoke<Snapshot>("new_source_skill", {
        name: element<HTMLInputElement>("new-skill-name").value,
        project: element<HTMLSelectElement>("new-skill-project").value || null,
        markdown: element<HTMLTextAreaElement>("new-skill-body").value,
      }),
    );
    element<HTMLInputElement>("new-skill-name").value = "";
    element<HTMLTextAreaElement>("new-skill-body").value = "";
    message("New source skill created and validated.");
  });
});
element("restore-form").addEventListener("submit", (event) => {
  event.preventDefault();
  void operation(async () => {
    render(
      await invoke<Snapshot>("restore_workspace", {
        backup: element<HTMLInputElement>("backup-path").value,
      }),
    );
    message("Backup restored.");
  });
});
function scheduleArguments(): { seconds: number } {
  return {
    seconds: Number(element<HTMLInputElement>("schedule-seconds").value),
  };
}
element("preview-schedule").addEventListener("click", () => {
  void operation(async () => {
    const result = await invoke<{ definition: string }>(
      "preview_schedule",
      scheduleArguments(),
    );
    element("schedule-preview").textContent = result.definition;
    element("schedule-preview").hidden = false;
  });
});
element("schedule-form").addEventListener("submit", (event) => {
  event.preventDefault();
  void operation(async () =>
    message(await invoke<string>("enable_schedule", scheduleArguments())),
  );
});
element("remove-schedule").addEventListener("click", () => {
  void operation(async () => message(await invoke<string>("disable_schedule")));
});
element("sync-now").addEventListener("click", () => {
  void operation(async () => {
    render(await invoke<Snapshot>("sync_workspace"));
    message("Content pulled and synced.");
  });
});
for (const [id, install] of [
  ["check-updates", false],
  ["install-update", true],
] as const) {
  element(id).addEventListener("click", () => {
    void operation(async () =>
      message(await invoke<string>("update_desktop", { install })),
    );
  });
}
element("restart").addEventListener("click", () => {
  void operation(async () => invoke<void>("restart_desktop"));
});

element("open-source").addEventListener("click", () => {
  void operation(async () => {
    sourceDocument = await invoke("read_source_configuration", {
      file: element<HTMLSelectElement>("source-file").value,
    });
    if (sourceDocument) {
      element<HTMLTextAreaElement>("source-text").value = sourceDocument.text;
      element<HTMLTextAreaElement>("source-text").disabled = false;
    }
  });
});
element("save-source").addEventListener("click", () => {
  void operation(async () => {
    if (!sourceDocument) return;
    render(
      await invoke<Snapshot>("save_source_configuration", {
        file: sourceDocument.file,
        text: element<HTMLTextAreaElement>("source-text").value,
        expectedHash: sourceDocument.hash,
      }),
    );
    sourceDocument = await invoke("read_source_configuration", {
      file: sourceDocument.file,
    });
    message(
      "Source configuration validated and saved. Review changes before applying.",
    );
  });
});

function unsavedChanges(): boolean {
  if (!snapshot) return false;
  if (
    sourceEditPath ||
    resolution ||
    agentDraft ||
    [
      "agent-tag",
      "agent-global",
      "agent-project",
      "agent-instructions",
      "agent-instructions-folder",
    ].some((id) => element<HTMLInputElement>(id).value !== "")
  )
    return true;
  return hasUnsavedEdits({
    configuration: {
      text: element<HTMLTextAreaElement>("config-text").value,
      saved: snapshot.config_text,
    },
    source: sourceDocument
      ? {
          text: element<HTMLTextAreaElement>("source-text").value,
          saved: sourceDocument.text,
        }
      : null,
    skill: editing
      ? {
          markdown: editing.document.markdown,
          metadata: editing.document.metadata,
          savedMarkdown: editing.saved.markdown,
          savedMetadata: editing.saved.metadata,
        }
      : null,
    newSkill: {
      name: element<HTMLInputElement>("new-skill-name").value,
      markdown: element<HTMLTextAreaElement>("new-skill-body").value,
    },
    schedule: {
      automatic: element<HTMLInputElement>("automatic-updates").checked,
      savedAutomatic: snapshot.updates.automatic,
      time: element<HTMLInputElement>("update-time").value,
      savedTime: snapshot.updates.time,
    },
  });
}

element("update-schedule-form").addEventListener("submit", (event) => {
  event.preventDefault();
  void operation(async () => {
    render(
      await invoke<Snapshot>("save_update_schedule", {
        automatic: element<HTMLInputElement>("automatic-updates").checked,
        time: element<HTMLInputElement>("update-time").value,
      }),
    );
    message("Nightly update schedule saved.");
  });
});

interface ResolutionPreview {
  path: string;
  action: string;
  conflict: Conflict;
  effects: Change[];
  explanation: string;
  revision: string;
}
let resolution: ResolutionPreview | null = null;
let sourceEditPath: string | null = null;
let agentDraft: { file: string; text: string; hash: string } | null = null;

function showJobStatus(status: JobStatus): void {
  const run = status.last_run;
  element("job-result").textContent = run
    ? `Last started: ${new Date(run.started_at * 1000).toLocaleString()} · ${run.finished_at === null ? "No completion recorded" : run.error ? `Failed: ${run.error}` : "Succeeded"}`
    : "No background run recorded yet.";
  element("job-error").textContent =
    [
      status.current_error && `Sync: ${status.current_error}`,
      status.helper_update_error &&
        `Helper update: ${status.helper_update_error}`,
      status.desktop_update_error &&
        `Desktop update: ${status.desktop_update_error}`,
    ]
      .filter(Boolean)
      .join("\n") || "No current ledger errors.";
  element("job-log").textContent = status.log || "No logs recorded yet.";
}

function conflictRow(conflict: Conflict): HTMLElement {
  const row = textRow(conflict.kind.replaceAll("-", " "), conflict.path);
  row.classList.add("conflict");
  const diff = document.createElement("pre");
  diff.textContent =
    conflict.diff ||
    "No text diff available. Review the file paths and action effects below.";
  row.append(diff);
  if (conflict.source) {
    const source = document.createElement("p");
    source.textContent = `Source: ${conflict.source}`;
    row.append(source);
  }
  if (conflict.source) {
    const edit = document.createElement("button");
    edit.className = "secondary";
    edit.textContent = "Edit mapped source";
    edit.addEventListener("click", () => {
      void operation(async () => {
        const source = await invoke<{ file: string; text: string }>(
          "read_conflict_source",
          { path: conflict.path },
        );
        sourceEditPath = conflict.path;
        resolution = null;
        element("resolution-preview").hidden = true;
        element("conflict-source-title").textContent = source.file;
        element<HTMLTextAreaElement>("conflict-source-text").value =
          source.text;
        element("conflict-source-editor").hidden = false;
        element("conflict-source-editor").scrollIntoView({ block: "center" });
      });
    });
    row.append(edit);
  }
  if (conflict.copy_back_blocked) {
    const note = document.createElement("p");
    note.textContent = conflict.copy_back_blocked;
    row.append(note);
  }
  for (const [action, label] of [
    ["copy-back", "Copy edit back to source"],
    ["reinstall", "Discard edit and reinstall"],
    ["adopt", "Take ownership as-is"],
  ] as const) {
    if (action === "copy-back" && conflict.copy_back_blocked) continue;
    const button = document.createElement("button");
    button.className = "secondary";
    button.textContent = `Preview: ${label}`;
    button.addEventListener("click", () => {
      void operation(async () => {
        resolution = await invoke<ResolutionPreview>("preview_conflict", {
          path: conflict.path,
          action,
        });
        showResolution(resolution);
      });
    });
    row.append(button);
  }
  return row;
}

function showResolution(preview: ResolutionPreview): void {
  element("resolution-preview").hidden = false;
  element("resolution-explanation").textContent = preview.explanation;
  element("resolution-effects").textContent = preview.effects
    .map(
      (effect) =>
        `${effect.action}: ${effect.path}\nExecutable after: ${effect.executable ?? "absent"}\n${effect.diff || "No content change; ownership or permissions will change."}`,
    )
    .join("\n\n");
  element("resolution-preview").scrollIntoView({ block: "center" });
}
element("preview-source-edit").addEventListener("click", () => {
  void operation(async () => {
    if (!sourceEditPath) return;
    resolution = await invoke<ResolutionPreview>("preview_source_edit", {
      path: sourceEditPath,
      text: element<HTMLTextAreaElement>("conflict-source-text").value,
    });
    showResolution(resolution);
  });
});
element("conflict-source-text").addEventListener("input", () => {
  resolution = null;
  element("resolution-preview").hidden = true;
});
element("confirm-resolution").addEventListener("click", () => {
  void operation(async () => {
    if (!resolution) return;
    const backup = await invoke<string>("resolve_conflict", {
      expected: JSON.stringify(resolution),
    });
    element<HTMLInputElement>("backup-path").value = backup;
    resolution = null;
    element("resolution-preview").hidden = true;
    message(`Action completed. Verified backup: ${backup}`);
    await refresh();
    message(`Action completed. Verified backup: ${backup}`);
  });
});
element("cancel-resolution").addEventListener("click", () => {
  resolution = null;
  element("resolution-preview").hidden = true;
});

function agentTarget(): AgentTarget {
  return {
    tag: element<HTMLInputElement>("agent-tag").value,
    global_folder: element<HTMLInputElement>("agent-global").value,
    project_folder: element<HTMLInputElement>("agent-project").value,
    instructions_file: element<HTMLInputElement>("agent-instructions").value,
    instructions_folder:
      element<HTMLInputElement>("agent-instructions-folder").value || null,
  };
}
function fillAgent(target: AgentTarget): void {
  element<HTMLInputElement>("agent-tag").value = target.tag;
  element<HTMLInputElement>("agent-global").value = target.global_folder;
  element<HTMLInputElement>("agent-project").value = target.project_folder;
  element<HTMLInputElement>("agent-instructions").value =
    target.instructions_file;
  element<HTMLInputElement>("agent-instructions-folder").value =
    target.instructions_folder || "";
}
function invalidateAgentDraft(): void {
  agentDraft = null;
  element("agent-preview").hidden = true;
  element<HTMLButtonElement>("confirm-agent").disabled = true;
}
element("agent-preset").addEventListener("change", () => {
  invalidateAgentDraft();
  const preset = snapshot?.presets.find(
    (p) => p.target.tag === element<HTMLSelectElement>("agent-preset").value,
  );
  fillAgent(
    preset?.target ?? {
      tag: "",
      global_folder: "",
      project_folder: "",
      instructions_file: "",
      instructions_folder: null,
    },
  );
  const sources = element("preset-sources");
  sources.replaceChildren();
  for (const url of preset?.sources ?? []) {
    const link = document.createElement("a");
    link.href = url;
    link.textContent = url;
    link.target = "_blank";
    link.rel = "noreferrer";
    sources.append(link, document.createElement("br"));
  }
});
element("agent-form").addEventListener("input", invalidateAgentDraft);
element("agent-form").addEventListener("submit", (event) => {
  event.preventDefault();
  void operation(async () => {
    agentDraft = await invoke("preview_agent", { target: agentTarget() });
    if (agentDraft) {
      element("agent-preview").textContent = agentDraft.text;
      element("agent-preview").hidden = false;
    }
  });
});
element("confirm-agent").addEventListener("click", () => {
  void operation(async () => {
    if (!agentDraft) return;
    render(
      await invoke<Snapshot>("add_agent_target", {
        target: agentTarget(),
        expectedHash: agentDraft.hash,
      }),
    );
    invalidateAgentDraft();
    fillAgent({
      tag: "",
      global_folder: "",
      project_folder: "",
      instructions_file: "",
      instructions_folder: null,
    });
    element<HTMLSelectElement>("agent-preset").value = "";
    element("preset-sources").replaceChildren();
    message(
      "Agent definition saved. Open the machine profile below and add the tag to its harnesses list to enable it.",
    );
  });
});

type Draft = {
  name: string;
  project: string | null;
  markdown: string;
  metadata: string;
  machine: string;
  harness: string;
};
const livePreview = createLivePreview<
  Draft,
  { path: string; text: string | null }[]
>(
  (draft) => invoke("preview_skill", draft),
  (files) => {
    element("editor-validation").textContent = files.length
      ? "Valid draft. Preview rendered by the Rust core."
      : "Valid draft. This skill is excluded or disabled for the selected target.";
    element("editor-validation").className = "message";
    element("editor-preview").textContent = files
      .map((file) => `${file.path}\n${file.text ?? "Binary output"}`)
      .join("\n\n");
  },
  (error) => {
    element("editor-validation").textContent = String(error);
    element("editor-validation").className = "message error";
    element("editor-preview").textContent = "";
  },
);
function schedulePreview(): void {
  if (!editing || !isTauri()) return;
  const target = snapshot?.preview_targets.find(
    (t) =>
      `${t.machine}/${t.harness}` ===
      element<HTMLSelectElement>("preview-target").value,
  );
  if (!target) {
    livePreview.cancel();
    element("editor-validation").textContent =
      "No enabled machine × harness available.";
    return;
  }
  element("editor-validation").textContent = "Validating draft…";
  element("editor-preview").textContent = "";
  livePreview.schedule({
    name: editing.skill.name,
    project: editing.skill.project,
    markdown: editing.document.markdown,
    metadata: editing.document.metadata,
    ...target,
  });
}
element("preview-target").addEventListener("change", schedulePreview);
element("refresh-job-status").addEventListener("click", () => {
  void operation(async () =>
    showJobStatus(await invoke<JobStatus>("read_job_status")),
  );
});
element("open-at-login").addEventListener("change", () => {
  const checkbox = element<HTMLInputElement>("open-at-login");
  const enabled = checkbox.checked;
  void operation(async () => {
    checkbox.disabled = true;
    try {
      checkbox.checked = await invoke<boolean>("set_login_enabled", {
        enabled,
      });
    } catch (error) {
      checkbox.checked = !enabled;
      throw error;
    } finally {
      checkbox.disabled = false;
    }
    message(
      checkbox.checked
        ? "Ratatoskr will open at login."
        : "Open at login is off.",
    );
  });
});

let checkingNightly = false;
async function nightlyTick(): Promise<void> {
  if (!isTauri() || !snapshot || checkingNightly || busy) return;
  checkingNightly = true;
  try {
    await runNightlyUpdate(
      () => invoke<boolean>("nightly_update", { install: false }),
      () => !busy && !unsavedChanges(),
      () =>
        operation(async () => {
          // Freeze editors only after checking for edits, through the install/relaunch boundary.
          const controls = Array.from(
            document.querySelectorAll<
              HTMLInputElement | HTMLTextAreaElement | HTMLSelectElement
            >("input, textarea, select"),
          );
          const disabled = controls.map((control) => control.disabled);
          controls.forEach((control) => {
            control.disabled = true;
          });
          try {
            await invoke<boolean>("nightly_update", { install: true });
          } finally {
            controls.forEach((control, index) => {
              control.disabled = disabled[index];
            });
          }
        }),
    );
  } catch (error) {
    message(`Nightly update failed: ${String(error)}`, true);
  } finally {
    checkingNightly = false;
  }
}
setInterval(() => {
  void nightlyTick();
}, 30_000);

message("Checking for missed nightly updates…");
void runStartupUpdate(
  async () => {
    if (isTauri()) await invoke<boolean>("startup_update");
  },
  async () => {
    await operation(refresh);
    app.inert = false;
  },
).catch((error: unknown) => {
  message(`Startup update failed: ${String(error)}`, true);
});
