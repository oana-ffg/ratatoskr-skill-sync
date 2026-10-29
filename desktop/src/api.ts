import { invoke } from "@tauri-apps/api/core";
export interface Range {
  from: number;
  to: number;
}
export interface Block {
  range: Range;
  opening: Range;
  body: Range;
  closing: Range;
  selectors: Record<string, string[]>;
  active: boolean;
  reachable: boolean;
}
export interface Analysis {
  blocks: Block[];
  variables: {
    range: Range;
    name: string;
    value: string | null;
    active: boolean;
  }[];
  diagnostics: { range: Range; severity: string; message: string }[];
}
export interface Skill {
  name: string;
  enabled: boolean;
  project: string | null;
}
export interface SkillDocument {
  name: string;
  markdown: string;
  metadata: string;
  markdown_hash: string;
  metadata_hash: string;
}
export type Value =
  string | number | boolean | Value[] | { [key: string]: Value };
export type Table = Record<string, Value>;
export interface EditorView {
  description: string;
  description_sections: {
    text: string;
    selectors: Record<string, string[]> | null;
  }[];
  description_error: string | null;
  frontmatter: Range;
  metadata: Table;
}
export interface SourceDocument {
  file: string;
  text: string;
  hash: string;
}
export interface StructuredSource {
  file: string;
  hash: string;
  values: Table;
}
export interface AgentTarget {
  tag: string;
  global_folder: string;
  project_folder: string;
  instructions_file: string;
  instructions_folder: string | null;
}
export interface Preset {
  title: string;
  target: AgentTarget;
  sources: string[];
}
export interface Project {
  root: string;
  changed_files: number;
  outgoing_commits: number | null;
  upstream: string | null;
  error: string | null;
  source: string | null;
}
export interface Change {
  path: string;
  action: string;
  diff: string | null;
  before_sha256: string | null;
  after_sha256: string | null;
  executable: boolean | null;
}
export interface Conflict {
  harness: string | null;
  skill: string | null;
  path: string;
  kind: string;
  diff: string | null;
  source: string | null;
  copy_back_blocked: string | null;
}
export interface InstallTarget {
  harness: string;
  skills_root: string;
  instructions_file: string;
  project: string | null;
}
export interface ContentStatus {
  changed_files: number;
  outgoing_commits: number | null;
  upstream: string | null;
  error: string | null;
}
export interface Plan {
  content_status: ContentStatus;
  source: string;
  machine: string;
  mode: string;
  skills: number;
  targets: number;
  install_targets: InstallTarget[];
  staging_root: string | null;
  changes: Change[];
  conflicts: string[];
  conflict_details: Conflict[];
  stale_targets: StaleLocation[];
  projects: Project[];
}
export interface Jobs {
  last_run: {
    started_at: number;
    finished_at: number | null;
    error: string | null;
  } | null;
  current_error: string | null;
  helper_update_error: string | null;
  desktop_update_error: string | null;
  background_error: string | null;
  log: string;
}
export interface Legacy {
  name: string;
  destination: string;
  path: string;
  files: { path: string; sha256: string; executable: boolean }[];
}
export interface StaleFile {
  path: string;
  owned: boolean;
  modified: boolean;
  readonly: boolean | null;
  recorded: { sha256: string; executable: boolean } | null;
  current: { sha256: string; executable: boolean } | null;
  protection: { mode: number; uid: number; gid: number } | null;
}
export interface StaleLocation {
  path: string;
  harness: string | null;
  files: StaleFile[];
  roots: string[];
  archive_roots: string[];
  directories: Record<string, number>;
}
export interface StalePreview {
  action: "forget" | "remove";
  location: StaleLocation;
  archive: string | null;
  archives: string[];
  revision: string;
}
export interface Snapshot {
  plan: Plan;
  skills: Skill[];
  skills_error: string | null;
  helper: string | null;
  helper_error: string | null;
  jobs: Jobs;
  presets: Preset[];
  preview_targets: { machine: string; harness: string }[];
  source_files: string[];
  updates: {
    automatic: boolean;
    time: string;
    override_active: boolean;
    endpoint: string;
  };
  legacy: Legacy[];
}
export type SecretReference =
  | { kind: "gopass"; entry: string }
  | { kind: "keychain"; service: string; account: string }
  | { kind: "windows-credential"; target: string }
  | { kind: "environment"; variable: string };
export type AlertSettings =
  | { kind: "system" }
  | { kind: "pushover"; token: SecretReference; user: SecretReference }
  | { kind: "ntfy"; url: string; token?: SecretReference };
export interface BackgroundStatus {
  login_enabled: boolean;
  paused: boolean;
  interval_seconds: number;
  running: boolean;
  last_success: number | null;
  skills: number;
  error: string | null;
  migration_error: string | null;
}
export interface LocalSettings {
  source: string;
  machine: string;
  state_dir: string;
  projects: string[];
  alert_command: string[];
  deploy_branch: string;
  updates: Table & { automatic: boolean; time: string };
  alert?: AlertSettings | null;
}
export interface Resolution {
  action: string;
  path: string;
  effects: { path: string; diff?: string | null }[];
  [key: string]: unknown;
}
export interface PreviewFile {
  path: string;
  text: string | null;
  body: string | null;
}
export interface ValueOption {
  name: string;
  label: string;
  value: string;
}
export interface Resource {
  path: string;
  label: string;
}
// All production calls go through typed boundaries; test fixtures use Tauri's mockIPC.
export const api = {
  workspace: () => invoke<Snapshot | null>("read_workspace"),
  readSkill: (skill: Skill) =>
    invoke<SkillDocument>("read_source_skill", {
      name: skill.name,
      project: skill.project,
    }),
  view: (markdown: string, metadata: string) =>
    invoke<EditorView>("editor_view", { markdown, metadata }),
  edit: (markdown: string, metadata: string, edit: Record<string, unknown>) =>
    invoke<{ markdown: string; metadata: string }>("edit_skill_draft", {
      markdown,
      metadata,
      edit,
    }),
  analyze: (name: string, markdown: string, machine: string, harness: string) =>
    invoke<Analysis>("analyze_skill", { name, markdown, machine, harness }),
  preview: (
    skill: Skill,
    markdown: string,
    metadata: string,
    machine: string,
    harness: string,
  ) =>
    invoke<PreviewFile[]>("preview_skill", {
      name: skill.name,
      project: skill.project,
      markdown,
      metadata,
      machine,
      harness,
    }),
  values: (machine: string, harness: string) =>
    invoke<ValueOption[]>("list_values", { machine, harness }),
  resources: (skill: Skill) =>
    invoke<Resource[]>("list_skill_resources", {
      name: skill.name,
      project: skill.project,
    }),
  source: (file: string) =>
    invoke<StructuredSource>("read_structured_source", { file }),
  saveSource: (source: StructuredSource) =>
    invoke<Snapshot>(
      "save_structured_source",
      source as unknown as Record<string, unknown>,
    ),
  readInstructions: () => invoke<SourceDocument>("read_global_instructions"),
  saveInstructions: (markdown: string, expectedHash: string) =>
    invoke<Snapshot>("save_global_instructions", { markdown, expectedHash }),
  analyzeInstructions: (markdown: string, machine: string, harness: string) =>
    invoke<Analysis>("analyze_global_instructions", {
      markdown,
      machine,
      harness,
    }),
  previewInstructions: (markdown: string, machine: string, harness: string) =>
    invoke<string>("preview_global_instructions", {
      markdown,
      machine,
      harness,
    }),
  instructionsPath: (machine: string, harness: string) =>
    invoke<string>("preview_global_instructions_path", { machine, harness }),
  folder: () => invoke<string | null>("choose_folder"),
  command: <T>(command: string, args?: Record<string, unknown>) =>
    invoke<T>(command, args),
};
export function agentName(tag: string): string {
  return (
    (
      {
        claude: "Claude Code",
        codex: "Codex",
        gemini: "Gemini CLI",
        copilot: "GitHub Copilot",
        opencode: "OpenCode",
      } as Record<string, string>
    )[tag] ?? tag
  );
}
export function title(value: string): string {
  return value === "macos"
    ? "macOS"
    : value === "windows"
      ? "Windows"
      : value === "linux"
        ? "Linux"
        : value.replace(/[-_]/g, " ").replace(/^./, (c) => c.toUpperCase());
}
export function basename(path: string): string {
  return path.split(/[\\/]/).filter(Boolean).at(-1) ?? path;
}
export function errorText(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}
export function relativeTime(seconds: number | null | undefined): string {
  if (!seconds) return "No completed sync yet";
  const minutes = Math.max(0, Math.floor((Date.now() / 1000 - seconds) / 60));
  return minutes < 1
    ? "Just now"
    : minutes < 60
      ? `${minutes} min ago`
      : `${Math.floor(minutes / 60)} hr ago`;
}
export const emptyAnalysis: Analysis = {
  blocks: [],
  variables: [],
  diagnostics: [],
};

export interface ReviewGroup {
  key: string;
  name: string;
  harness: string | null;
  project: string | null;
  changes: Change[];
  conflicts: Conflict[];
}
const portable = (path: string) =>
  path.replaceAll("\\", "/").replace(/\/$/, "");
const within = (path: string, root: string) =>
  portable(path) === portable(root) ||
  portable(path).startsWith(`${portable(root)}/`);

/** Group by actual rendered destination, including custom roots and project scope. */
export function reviewSummary(workspace: Snapshot) {
  const { plan, legacy } = workspace;
  const readyChanges = plan.changes.filter(
    (change) => !plan.conflict_details.some((c) => within(change.path, c.path)),
  );
  const groups = new Map<string, ReviewGroup>();
  function group(path: string) {
    const target = plan.install_targets.find(
      (t) =>
        within(path, t.skills_root) ||
        portable(path) === portable(t.instructions_file),
    );
    const name = target
      ? portable(path) === portable(target.instructions_file)
        ? basename(target.instructions_file)
        : portable(path)
            .slice(portable(target.skills_root).length + 1)
            .split("/")[0]
      : basename(path);
    const key = JSON.stringify([
      target?.project,
      target?.harness,
      target?.skills_root ?? path,
      name,
    ]);
    let value = groups.get(key);
    if (!value) {
      value = {
        key,
        name,
        harness: target?.harness ?? null,
        project: target?.project ?? null,
        changes: [],
        conflicts: [],
      };
      groups.set(key, value);
    }
    return value;
  }
  for (const change of readyChanges) group(change.path).changes.push(change);
  for (const conflict of plan.conflict_details)
    group(conflict.path).conflicts.push(conflict);
  const errors = plan.conflicts.filter(
    (message) =>
      !plan.conflict_details.some((c) => message.includes(c.path)) &&
      !legacy.some((item) => message.includes(item.path)) &&
      !plan.stale_targets.some((item) => message.includes(item.path)),
  );
  const items = [...groups.values()];
  const decisions =
    items.filter((g) => g.conflicts.length).length +
    errors.length +
    legacy.length +
    plan.stale_targets.length;
  const writes = readyChanges.filter((c) => c.action !== "remove").length;
  const removes = readyChanges.length - writes;
  return {
    readyCount: readyChanges.length,
    applyAction: applyAction(plan.mode, writes, removes),
    groups: items,
    errors,
    decisions,
    total:
      items.length + errors.length + legacy.length + plan.stale_targets.length,
  };
}

function applyAction(
  mode: Snapshot["plan"]["mode"],
  writes: number,
  removes: number,
): string {
  const actions = [
    writes ? `Write ${writes} ${writes === 1 ? "file" : "files"}` : "",
    removes ? `remove ${removes} ${removes === 1 ? "file" : "files"}` : "",
  ]
    .filter(Boolean)
    .join(" and ");
  return actions
    ? `${actions[0].toUpperCase()}${actions.slice(1)} ${mode === "staging" ? "in the staging folder" : "in the real agent folders"}`
    : "No files to apply";
}
