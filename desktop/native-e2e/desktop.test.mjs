// Native journeys through the real desktop app, its Rust core and Git. They run
// on disposable CI machines only: the app shares its single-instance record and
// app data with any installed copy. See docs/development.md.
import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import {
  createWriteStream,
  existsSync,
  mkdirSync,
  readFileSync,
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

let workspace;
before(() => {
  if (disabled) return;
  mkdirSync(results, { recursive: true });
  workspace = createWorkspace();
});

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

/** Starts the app against the fixture workspace, logging its output for CI artifacts. */
async function launch(name, args = []) {
  const port = await freePort();
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
    running,
    async stop() {
      if (running()) child.kill();
      await exited;
    },
  };
}

/** Runs a journey, keeping a screenshot and any app error banners when it fails. */
async function drive(name, journey) {
  const desktop = await launch(name);
  try {
    const session = await Session.start(desktop.port, desktop.running);
    try {
      await journey(session);
    } catch (error) {
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

/** Windows tray sync runs every minute; pausing keeps the reviewed Apply deterministic. */
async function traySync(session, action, result) {
  if (process.platform !== "win32") return;
  await session.click(nav("Settings"));
  await session.click(button(action));
  await session.element(button(result));
  await session.click(nav("Skills"));
}

test(
  "the app creates, edits, publishes and applies a skill through the real core",
  { skip: disabled },
  () =>
    drive("editor", async (session) => {
      await session.element("//h2[normalize-space(.)='No skills yet']", 60000);
      await traySync(session, "Pause sync", "Resume sync");

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
      const modifier = process.platform === "darwin" ? "" : "";
      await session.type(editor, `${modifier}`);
      await session.type(editor, "Written in the native editor.");
      await session.click(button("Save"));
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
      await traySync(session, "Resume sync", "Pause sync");
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
    const startedAt = Math.floor(Date.now() / 1000);
    const tray = await launch("tray", ["--background"]);
    try {
      const lastRun = () =>
        JSON.parse(workspace.ratatoskr("job-status")).last_run;
      await until(
        "the startup sync",
        () => lastRun()?.finished_at >= startedAt && lastRun(),
        60000,
      );
      assert.equal(lastRun().error, null);
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
    } finally {
      await tray.stop();
    }
  },
);
