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
}
interface Plan {
  source: string;
  machine: string;
  mode: string;
  skills: number;
  targets: number;
  changes: Change[];
  conflicts: string[];
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
  helper: string;
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
  <section id="changes" class="panel"><div class="section-heading"><div><p class="eyebrow">REVIEW BEFORE YOU SYNC</p><h2>Pending changes</h2></div><button id="apply" disabled>Apply reviewed changes</button></div><div id="conflicts"></div><div id="change-list" class="empty">Connect your content to preview changes.</div></section>
  <section id="skills" class="panel"><p class="eyebrow">EDIT THE SOURCE</p><h2>Skills in your grove</h2><div id="skill-list" class="empty">Connect content to manage skills.</div><form id="new-skill-form"><label>New skill name<input id="new-skill-name" required pattern="[a-z0-9]+(-[a-z0-9]+)*"></label><label>Location<select id="new-skill-project"><option value="">Global content</option></select></label><label class="full">Skill markdown, including YAML name and description<textarea id="new-skill-body" rows="6" required spellcheck="false"></textarea></label><button type="submit">Validate and create skill</button></form><section id="skill-editor" hidden><h3 id="editor-title"></h3><label>Source file<select id="editor-file"><option>SKILL.md</option><option>skill.toml</option></select></label><label>Contents<textarea id="editor-text" rows="15" spellcheck="false"></textarea></label><button id="save-skill">Validate and save source</button><p>Conditions use only comment blocks. Variables use the af prefix. Target selection and per-harness frontmatter live in skill.toml.</p></section></section>
  <section id="projects" class="panel"><p class="eyebrow">DON’T LEAVE SKILLS BEHIND</p><h2>Monitored projects</h2><p>Track skill files that still need a commit or push.</p><div id="project-list" class="empty">No projects registered.</div><form id="project-form"><label>Git repository root<input id="project-root" autocomplete="off" required></label><label>Skill source folder inside project<input id="project-source" value="agent-skills" required></label><button type="submit" class="secondary">Monitor project</button></form></section>
  <section id="configuration" class="panel"><div class="section-heading"><div><p class="eyebrow">YOUR MACHINE, YOUR RULES</p><h2>Configuration</h2></div><button id="save-config" class="secondary" disabled>Validate and save</button></div><p id="config-path"></p><label for="config-text">Local TOML configuration</label><textarea id="config-text" rows="11" spellcheck="false" disabled></textarea></section>
  <section class="panel"><h2>Agent presets and machine profiles</h2><p>Edit harness definitions, machine conditions, or shared global instructions in your content repository.</p><label>Source file<select id="source-file"></select></label><button id="open-source" class="secondary">Open source file</button><label>Contents<textarea id="source-text" rows="12" spellcheck="false" disabled></textarea></label><button id="save-source" disabled>Validate and save source file</button></section>
  <section id="automation" class="panel"><p class="eyebrow">THE MESSENGER KEEPS RUNNING</p><h2>Background sync</h2><p>The helper runs independently through launchd on Mac or Task Scheduler on Windows. Use a clean deployment checkout with a Git upstream. Configure alert_command in local TOML for failure and recovery notifications.</p><form id="schedule-form"><label>Helper executable<input id="helper-path" required></label><label>Interval in seconds<input id="schedule-seconds" type="number" min="60" max="86400" value="60" required></label><button type="button" id="preview-schedule" class="secondary">Preview schedule</button><button type="submit">Install background schedule</button><button type="button" id="remove-schedule" class="secondary">Remove schedule</button></form><pre id="schedule-preview" hidden></pre><button id="sync-now" class="secondary">Pull and sync now</button></section>
  <section class="panel"><h2>Adoption and recovery</h2><p>Adoption backs up existing target skills before recording ownership. Review the resulting plan before applying: source files will replace adopted copies, and adopted files absent from source will be removed. Restore only verified backups from this workspace.</p><button id="adopt" class="secondary">Back up and adopt existing targets</button><form id="restore-form"><label>Backup directory<input id="backup-path" required></label><button type="submit" class="secondary">Restore backup</button></form></section>
  <section class="panel"><h2>Signed updates</h2><p>Official updates use the built-in signing key. Automatic updates wait for idle work, then install and relaunch this workspace. Unsaved edits defer updates for up to one hour. Launching the app catches up on a missed update before editing starts. Forks can explicitly override endpoint and public_key in local TOML.</p><p id="update-status" role="status"></p><form id="update-schedule-form"><label class="check"><input id="automatic-updates" type="checkbox" checked>Enable nightly updates</label><label>Local update time<input id="update-time" type="time" value="00:00" required></label><button type="submit">Save update schedule</button></form><div class="button-row"><button id="check-updates" class="secondary">Check desktop updates</button><button id="install-update" class="secondary">Install signed desktop update</button><button id="restart" class="secondary">Restart app</button></div></section>
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
  for (const conflict of plan.conflicts) {
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
  if (!element<HTMLInputElement>("helper-path").value)
    element<HTMLInputElement>("helper-path").value = result.helper;
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
        editing = { skill, document, saved: { ...document } };
        element("skill-editor").hidden = false;
        element("editor-title").textContent = skill.name;
        element<HTMLSelectElement>("editor-file").value = "SKILL.md";
        element<HTMLTextAreaElement>("editor-text").value = document.markdown;
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
});
element("editor-file").addEventListener("change", () => {
  if (!editing) return;
  element<HTMLTextAreaElement>("editor-text").value =
    element<HTMLSelectElement>("editor-file").value === "SKILL.md"
      ? editing.document.markdown
      : editing.document.metadata;
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
element("adopt").addEventListener("click", () => {
  void operation(async () => {
    const path = await invoke<string>("adopt_workspace");
    await refresh();
    element<HTMLInputElement>("backup-path").value = path;
    message(
      `Adoption complete. Original files are backed up at ${path}. Review changes before applying.`,
    );
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
function scheduleArguments(): { helper: string; seconds: number } {
  return {
    helper: element<HTMLInputElement>("helper-path").value,
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
