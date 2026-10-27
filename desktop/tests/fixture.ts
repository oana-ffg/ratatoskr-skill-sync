import { execFileSync } from "node:child_process";
import { resolve } from "node:path";
import type {
  Snapshot,
  SkillDocument,
  EditorView,
  Analysis,
  Resource,
  ValueOption,
  LocalSettings,
  SourceDocument,
} from "../src/api";
const binary = resolve(
  "..",
  process.env.CARGO_TARGET_DIR ?? "target",
  "debug",
  "examples",
  `desktop_fixture${process.platform === "win32" ? ".exe" : ""}`,
);
export function oracle(
  cmd: string,
  args: Record<string, unknown> = {},
): unknown {
  try {
    return JSON.parse(
      execFileSync(binary, {
        input: JSON.stringify({ cmd, args }),
        encoding: "utf8",
        stdio: ["pipe", "pipe", "pipe"],
      }),
    );
  } catch (e) {
    const error = e as Error & { stderr?: Buffer };
    throw new Error(error.stderr?.toString() || error.message);
  }
}
export const fixture = oracle("fixture") as {
  workspace: Snapshot;
  document: SkillDocument;
  info: EditorView;
  analysis: Analysis;
  resources: Resource[];
  values: ValueOption[];
  statuses: Record<string, Record<string, boolean>>;
  local_settings: LocalSettings;
};

export function projectWorkspace(): Snapshot {
  const workspace = structuredClone(fixture.workspace);
  const project = workspace.plan.projects[0];
  project.changed_files = 0;
  project.outgoing_commits = 0;
  workspace.skills.push(
    { name: "mix-audio", enabled: true, project: project.root },
    { name: "archive-audio", enabled: false, project: project.root },
  );
  return workspace;
}
export function fixtureIPC(
  cmd: string,
  args: Record<string, unknown> = {},
): unknown {
  switch (cmd) {
    case "runtime_platform":
      return "linux";
    case "set_app_error_listener":
      return null;
    case "migrate_background":
      return null;
    case "background_status":
      return null;
    case "read_update_settings":
      return fixture.workspace.updates;
    case "read_workspace":
      return fixture.workspace;
    case "read_source_skill":
      return args.name === "aws-project-migration"
        ? oracle("realistic_document")
        : fixture.document;
    case "list_values":
      return fixture.values;
    case "list_skill_resources":
      return args.name === "aws-project-migration"
        ? oracle("realistic_resources")
        : fixture.resources;
    case "skill_target_statuses":
      return fixture.statuses;
    case "read_local_settings":
      return { settings: fixture.local_settings, hash: "fixture-config-hash" };
    case "login_enabled":
      return false;
    case "choose_folder":
      return "/projects/audio-tools";
    case "choose_resource":
      return null;
    default:
      return oracle(cmd, args);
  }
}

/** Stateful editor bridge; save goes through the Rust core and reads disk back. */
export function editorFixture() {
  let document = structuredClone(fixture.document);
  return {
    get document() {
      return document;
    },
    invoke(cmd: string, args: Record<string, unknown> = {}): unknown {
      if (cmd === "read_source_skill") return document;
      if (cmd === "save_skill_draft") {
        document = oracle(cmd, { ...args, current: document }) as SkillDocument;
        return fixture.workspace;
      }
      if (cmd === "archive_source_skill") {
        const workspace = structuredClone(fixture.workspace);
        workspace.skills = workspace.skills.filter((s) => s.name !== args.name);
        return { path: oracle(cmd, args), workspace };
      }
      return fixtureIPC(cmd, args);
    },
  };
}

/** Stateful global instructions bridge; save goes through the Rust core and reads disk back. */
export function instructionsFixture() {
  let document = oracle("read_global_instructions") as SourceDocument;
  return {
    get document() {
      return document;
    },
    invoke(cmd: string, args: Record<string, unknown> = {}): unknown {
      if (cmd === "read_global_instructions") return document;
      if (cmd === "save_global_instructions") {
        document = oracle(cmd, {
          ...args,
          current: document.text,
        }) as SourceDocument;
        return fixture.workspace;
      }
      return fixtureIPC(cmd, args);
    },
  };
}
