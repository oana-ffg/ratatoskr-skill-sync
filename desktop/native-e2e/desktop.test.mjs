// Native journeys through the real desktop app, its Rust core and Git. They run
// on disposable CI machines only: the app shares its single-instance record and
// app data with any installed copy. See docs/development.md.
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import {
  createWriteStream,
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { createServer } from "node:net";
import { join } from "node:path";
import { before, test } from "node:test";
import { fileURLToPath } from "node:url";
import { Session, until, xpathString } from "./webdriver.mjs";
import { app, createWorkspace } from "./workspace.mjs";

const disabled =
  process.env.RATATOSKR_NATIVE_E2E !== "1" &&
  "Set RATATOSKR_NATIVE_E2E=1 on a disposable machine";
const results = fileURLToPath(
  new URL("../test-results/native/", import.meta.url),
);
const nav = (name) =>
  `//nav[@aria-label='Main navigation']//button[contains(normalize-space(.), ${xpathString(name)})]`;
const button = (name) => `//button[normalize-space(.)=${xpathString(name)}]`;
const labelled = (label) => `//*[@aria-label=${xpathString(label)}]`;
const field = (dialog, label, tag) =>
  `//dialog[@aria-label=${xpathString(dialog)}]//label[contains(., ${xpathString(label)})]//${tag}`;
const skill = (name, description, body) =>
  `---\nname: ${name}\ndescription: ${description}\n---\n${body}\n`;

before(() => {
  if (!disabled) mkdirSync(results, { recursive: true });
});

/** The latest run, once one has finished at or after `since` (Unix seconds). */
function finishedSince(workspace, since) {
  const run = JSON.parse(workspace.ratatoskr("job-status")).last_run;
  return run?.finished_at >= since && run;
}

/** Records sync state and running Git/Ratatoskr processes beside a failure. */
function diagnose(workspace, name) {
  const section = (title, read) => {
    try {
      return `## ${title}\n${read()}`;
    } catch (error) {
      return `## ${title}\nUnavailable: ${error.message}`;
    }
  };
  const state = join(workspace.state, workspace.machine);
  const report = [
    section("job-status", () => workspace.ratatoskr("job-status")),
    section("sync.stdout.log", () =>
      readFileSync(join(state, "sync.stdout.log"), "utf8"),
    ),
    section("sync.stderr.log", () =>
      readFileSync(join(state, "sync.stderr.log"), "utf8"),
    ),
    section("processes", () =>
      process.platform === "win32"
        ? execFileSync(
            "powershell.exe",
            [
              "-NoProfile",
              "-Command",
              "Get-CimInstance Win32_Process | Where-Object { $_.Name -match 'git|ratatoskr' } | Format-List ProcessId,ParentProcessId,CommandLine",
            ],
            { encoding: "utf8" },
          )
        : execFileSync("ps", ["-axo", "pid,ppid,command"], { encoding: "utf8" })
            .split("\n")
            .filter((line) => /git|ratatoskr/.test(line))
            .join("\n"),
    ),
  ].join("\n\n");
  writeFileSync(join(results, `${name}-diagnostics.txt`), report);
  console.error(`Diagnostics for ${name}:\n${report}`);
}

function freePort() {
  return new Promise((resolve, reject) => {
    const server = createServer();
    server.once("error", reject);
    server.listen(0, "127.0.0.1", () => {
      const { port } = server.address();
      server.close(() => resolve(port));
    });
  });
}

/** Starts the app against a fixture workspace, logging its output for CI artifacts. */
async function launch(workspace, name, args = []) {
  const port = await freePort();
  const launchedAt = Math.floor(Date.now() / 1000);
  const log = createWriteStream(join(results, `${name}.log`));
  const child = spawn(app, args, {
    env: {
      ...process.env,
      RATATOSKR_CONFIG: workspace.config,
      TAURI_WEBDRIVER_PORT: String(port),
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  child.stdout.pipe(log);
  child.stderr.pipe(log);
  const exited = new Promise((resolve) => child.once("exit", resolve));
  const running = () => child.exitCode === null && child.signalCode === null;
  return {
    port,
    launchedAt,
    running,
    async stop() {
      if (running()) child.kill();
      await exited;
    },
  };
}

/** Runs a journey, keeping a screenshot, app error banners and sync diagnostics on failure. */
async function drive(name, journey) {
  const workspace = createWorkspace();
  const desktop = await launch(workspace, name);
  try {
    const session = await Session.start(desktop.port, desktop.running);
    try {
      await journey(session, desktop, workspace);
    } catch (error) {
      diagnose(workspace, name);
      await session.screenshot(join(results, `${name}.png`));
      const messages = await session.texts("//*[@role='alert']");
      if (messages.length)
        error.message += `\nApp alerts: ${messages.join(" | ")}`;
      throw error;
    } finally {
      await session.end();
    }
  } finally {
    await desktop.stop();
  }
}

test(
  "the app creates, edits, publishes and applies a skill through the real core",
  { skip: disabled },
  () =>
    drive("editor", async (session, desktop, workspace) => {
      await session.element("//h2[normalize-space(.)='No skills yet']", 60000);
      if (process.platform === "win32") {
        // The tray holds sync.lock while it syncs at launch and every minute.
        // Pausing after the launch run keeps the edits and Apply deterministic.
        await until(
          "the launch tray sync",
          () => finishedSince(workspace, desktop.launchedAt),
          60000,
        );
        await session.click(nav("Settings"));
        await session.click(button("Pause sync"));
        await session.element(button("Resume sync"));
        await session.click(nav("Skills"));
      }

      await session.click(button("＋ New skill"));
      await session.type(
        field("Create a skill", "Name", "input"),
        "native-check",
      );
      await session.type(
        field("Create a skill", "Description", "textarea"),
        "Created through the native app.",
      );
      await session.click(button("Create skill"));
      await until("the created skill on the remote", () =>
        workspace
          .remoteFile("skills/native-check/SKILL.md")
          .includes("Created through the native app."),
      );

      const editor = labelled("Skill Markdown editor");
      await session.click(editor);
      // Ctrl/Cmd+End, release modifiers, then type at the end of the body.
      const modifier = process.platform === "darwin" ? "\uE03D" : "\uE009";
      await session.type(editor, `${modifier}\uE010\uE000`);
      await session.type(editor, "Written in the native editor.");
      await session.click(button("Save"));
      // The push lands before the app finishes handling the save result.
      await until(
        "the save to finish",
        async () =>
          !(await session.find("//*[contains(@class, 'unsaved-indicator')]")),
      );
      await until("the edit on the remote", () =>
        workspace
          .remoteFile("skills/native-check/SKILL.md")
          .includes("Written in the native editor."),
      );

      await session.click(nav("Review changes"));
      await session.click(
        "//button[contains(normalize-space(.), 'in the staging folder')]",
      );
      for (const harness of ["claude", "codex"]) {
        const staged = workspace.staged(
          `${harness}/skills/native-check/SKILL.md`,
        );
        await until(`${harness} staged output`, () => existsSync(staged));
        assert.match(
          readFileSync(staged, "utf8"),
          /Written in the native editor\./,
        );
      }
    }),
);

test(
  "the Windows tray syncs published changes with no window open",
  {
    skip:
      disabled ||
      (process.platform !== "win32" &&
        "macOS background sync uses launchd; see the CLI end-to-end journeys"),
  },
  async () => {
    const workspace = createWorkspace();
    const tray = await launch(workspace, "tray", ["--background"]);
    try {
      const launchRun = await until(
        "the launch sync",
        () => finishedSince(workspace, tray.launchedAt),
        60000,
      );
      assert.equal(launchRun.error, null);
      workspace.publish("Add tray-check", {
        "skills/tray-check/SKILL.md": skill(
          "tray-check",
          "Published by another machine.",
          "Synced by the tray.",
        ),
        "skills/tray-check/skill.toml": "",
      });
      const staged = workspace.staged("codex/skills/tray-check/SKILL.md");
      await until("the interval sync", () => existsSync(staged), 120000);
      assert.match(readFileSync(staged, "utf8"), /Synced by the tray\./);
      assert.ok(tray.running(), "The tray app exited while syncing");
    } catch (error) {
      diagnose(workspace, "tray");
      throw error;
    } finally {
      await tray.stop();
    }
  },
);
