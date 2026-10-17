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
export function fixtureIPC(
  cmd: string,
  args: Record<string, unknown> = {},
): unknown {
  switch (cmd) {
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
