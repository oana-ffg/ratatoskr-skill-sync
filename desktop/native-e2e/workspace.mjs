// Disposable content workspace for native journeys. The real CLI creates the
// content repository and local configuration; a local bare remote and a second
// checkout stand in for the Git host and another machine.
import { execFileSync } from "node:child_process";
import {
  mkdirSync,
  mkdtempSync,
  readFileSync,
  realpathSync,
  writeFileSync,
} from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const executable = process.platform === "win32" ? ".exe" : "";
const target = fileURLToPath(new URL("../../target/debug/", import.meta.url));
export const app = join(target, `ratatoskr-desktop${executable}`);
const cli = join(target, `ratatoskr${executable}`);

// The fixture's config is always explicit, never an inherited selection.
const { RATATOSKR_CONFIG: _inherited, ...environment } = process.env;

function git(directory, ...args) {
  return execFileSync("git", ["-C", directory, ...args], { encoding: "utf8" });
}

function identity(directory) {
  for (const [key, value] of [
    ["user.name", "Ratatoskr native e2e"],
    ["user.email", "native-e2e@ratatoskr.invalid"],
    ["commit.gpgsign", "false"],
    ["core.hooksPath", ".git/hooks"],
  ]) {
    git(directory, "config", key, value);
  }
}

export function createWorkspace() {
  const root = realpathSync.native(
    mkdtempSync(join(tmpdir(), "ratatoskr-native-")),
  );
  const workspace = {
    root,
    config: join(root, "config.toml"),
    state: join(root, "state"),
    content: join(root, "content"),
    remote: join(root, "remote.git"),
    publisher: join(root, "publisher"),
    machine: "e2e",
    ratatoskr(...args) {
      return execFileSync(cli, ["--config", this.config, ...args], {
        encoding: "utf8",
        env: environment,
      });
    },
    staged(path) {
      return join(this.state, this.machine, "staging", ...path.split("/"));
    },
    remoteFile(path) {
      return git(this.remote, "show", `main:${path}`);
    },
    /** Commits and pushes a change from another checkout, as a second machine would. */
    publish(message, files) {
      git(this.publisher, "pull", "--ff-only");
      for (const [path, contents] of Object.entries(files)) {
        const file = join(this.publisher, ...path.split("/"));
        mkdirSync(dirname(file), { recursive: true });
        writeFileSync(file, contents);
      }
      git(this.publisher, "add", "--all");
      git(this.publisher, "commit", "-m", message);
      git(this.publisher, "push");
    },
  };
  workspace.ratatoskr(
    "init",
    "--create",
    "--source",
    workspace.content,
    "--machine",
    workspace.machine,
    // Explicit, so no default can place state outside the disposable root.
    "--state-dir",
    workspace.state,
  );
  identity(workspace.content);
  git(workspace.content, "add", "--all");
  git(workspace.content, "commit", "-m", "Initial content");
  execFileSync("git", ["init", "--bare", "-b", "main", workspace.remote]);
  git(workspace.content, "remote", "add", "origin", workspace.remote);
  git(workspace.content, "push", "-u", "origin", "main");
  execFileSync("git", ["clone", workspace.remote, workspace.publisher]);
  identity(workspace.publisher);
  // Keep the app offline: automatic updates would query the public releases.
  const config = readFileSync(workspace.config, "utf8");
  const offline = config.replace("automatic = true", "automatic = false");
  if (offline === config) {
    throw new Error(`Cannot disable automatic updates in:\n${config}`);
  }
  writeFileSync(workspace.config, offline);
  return workspace;
}
