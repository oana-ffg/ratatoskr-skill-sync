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
    windowsHide: true,
  });
  child.stdout.pipe(log);
  child.stderr.pipe(log);
  const exited = new Promise((resolve) => child.once("exit", resolve));
  const running = () => child.exitCode === null && child.signalCode === null;
  return {
    port,
    pid: child.pid,
    webviews: new Map(),
    launchedAt,
    running,
    /** Terminates the app, forcing it if it has not exited within ten seconds. */
    async stop() {
      if (!running()) return;
      child.kill();
      const timeout = new Promise((resolve) => setTimeout(resolve, 10000));
      if ((await Promise.race([exited.then(() => true), timeout])) !== true) {
        child.kill("SIGKILL");
        await exited;
      }
    },
  };
}

/** Real OS ancestry, including the browser, renderer, GPU and utility children. */
function webviewProcesses(desktop) {
  const { pid, webviews } = desktop;
  const processes = JSON.parse(
    execFileSync(
      "powershell.exe",
      [
        "-NoProfile",
        "-Command",
        "ConvertTo-Json -Compress -InputObject @(Get-CimInstance Win32_Process | Select-Object ProcessId,ParentProcessId,Name,CreationDate)",
      ],
      { encoding: "utf8", windowsHide: true, timeout: 15000 },
    ),
  );
  const descendants = new Set([pid]);
  let previous;
  do {
    previous = descendants.size;
    for (const child of processes) {
      if (descendants.has(child.ParentProcessId))
        descendants.add(child.ProcessId);
    }
  } while (descendants.size !== previous);
  assert.ok(
    processes.some((child) => child.ProcessId === pid),
    "The app PID is absent from the OS process snapshot",
  );
  const remaining = processes.filter(
    (child) =>
      child.Name.toLowerCase() === "msedgewebview2.exe" &&
      (descendants.has(child.ProcessId) ||
        webviews.get(child.ProcessId) === child.CreationDate),
  );
  // Retain observed identities so children still count after their browser parent
  // exits. CreationDate prevents an unrelated reused PID from counting as ours.
  for (const child of remaining)
    webviews.set(child.ProcessId, child.CreationDate);
  return remaining;
}
async function noWebviews(desktop, phase) {
  let remaining;
  await until(
    `${phase}: zero WebView2 descendants of ${desktop.pid}`,
    () => {
      assert.ok(desktop.running(), "The tray exited with its window");
      remaining = webviewProcesses(desktop);
      if (remaining.length)
        throw new Error(
          `WebView2 processes still alive: ${JSON.stringify(remaining)}`,
        );
      return true;
    },
    30000,
  );
  assert.deepEqual(remaining, []);
  console.log(`${phase}: app ${desktop.pid} alive, WebView2 descendants = 0`);
}
/** A second process must forward Open to the existing tray, then exit. */
async function secondLaunch(workspace, desktop, name) {
  const second = await launch(workspace, name);
  try {
    await until(
      "the second launch to hand off and exit",
      () => !second.running(),
    );
    assert.ok(desktop.running(), "The original tray process exited");
  } finally {
    await second.stop();
  }
}

/** Runs a failure-diagnostic or cleanup step. After an earlier failure, a problem
 * here is appended to that failure instead of replacing it. */
async function attempt(failure, what, step) {
  try {
    await step();
  } catch (problem) {
    if (!failure) throw problem;
    failure.message += `\n${what} also failed: ${problem.message}`;
  }
}

/** Runs a journey, keeping a screenshot, app error banners and sync diagnostics on failure. */
async function drive(name, journey) {
  const workspace = createWorkspace();
  const desktop = await launch(workspace, name);
  let session;
  let failure;
  try {
    session = await Session.start(desktop.port, desktop.running);
    await journey(session, desktop, workspace);
  } catch (error) {
    failure = error;
  }
  try {
    if (failure) {
      await attempt(failure, "Recording sync diagnostics", () =>
        diagnose(workspace, name),
      );
    }
    if (failure && session) {
      await attempt(failure, "Taking a screenshot", () =>
        session.screenshot(join(results, `${name}.png`)),
      );
      await attempt(failure, "Reading app alerts", async () => {
        const messages = await session.texts("//*[@role='alert']");
        if (messages.length)
          failure.message += `\nApp alerts: ${messages.join(" | ")}`;
      });
    }
    if (session)
      await attempt(failure, "Ending the WebDriver session", () =>
        session.end(),
      );
  } finally {
    await desktop.stop();
  }
  if (failure) throw failure;
}

/** Opens a screen and waits for `arrived`. The app shows its unsaved-changes guard
 * while a save is still finishing, so keep editing and retry; a guard that never
 * clears fails the journey. */
async function navigate(session, name, arrived) {
  // WebDriver find rejects modal dialogs because their offsetParent is null; use a child control.
  const guard =
    "//dialog[@aria-label='You have unsaved changes']//button[normalize-space(.)='Keep editing']";
  await until(`${name} to open`, async () => {
    if (await session.find(arrived)) return true;
    if (await session.find(guard)) await session.click(button("Keep editing"));
    else await session.click(nav(name));
    return false;
  });
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

      const apply =
        "//button[contains(normalize-space(.), 'in the staging folder')]";
      await navigate(session, "Review changes", apply);
      await session.click(apply);
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
      await noWebviews(tray, "background startup");
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
      await noWebviews(tray, "background sync");
      // Two simultaneous launches exercise serialized get-or-create.
      await Promise.all([
        secondLaunch(workspace, tray, "tray-open-a"),
        secondLaunch(workspace, tray, "tray-open-b"),
      ]);
      let session = await Session.start(tray.port, tray.running);
      await session.element(
        "//button[.//strong[normalize-space(.)='tray-check']]",
      );
      assert.deepEqual(await session.command("GET", "/window/handles"), [
        "main",
      ]);
      assert.ok(
        webviewProcesses(tray).length > 0,
        "Open created no WebView2 processes",
      );
      await session.command("DELETE", "/window");
      assert.deepEqual(await session.command("GET", "/window/handles"), []);
      await session.end();
      await noWebviews(tray, "clean close");

      workspace.publish("Publish after window destruction", {
        "skills/after-close/SKILL.md": skill(
          "after-close",
          "Published after closing.",
          "Tray still syncs.",
        ),
        "skills/after-close/skill.toml": "",
      });
      const afterClose = workspace.staged("codex/skills/after-close/SKILL.md");
      await until(
        "tray sync after destroying the window",
        () => existsSync(afterClose),
        120000,
      );
      assert.match(readFileSync(afterClose, "utf8"), /Tray still syncs\./);
      await noWebviews(tray, "sync after close");
      await secondLaunch(workspace, tray, "tray-reopen");
      session = await Session.start(tray.port, tray.running);
      await session.click(
        "//button[.//strong[normalize-space(.)='after-close']]",
      );
      const editor = labelled("Skill Markdown editor");
      await session.click(editor);
      await session.type(editor, "Unsaved Windows edit.");
      await until("Windows dirty editor to enable Save", () =>
        session.enabled(button("Save")),
      );
      await session.command("DELETE", "/window");
      // The dialog itself has no offsetParent; query a displayed control inside it.
      const keepEditing =
        "//dialog[@aria-label='You have unsaved changes']//button[normalize-space(.)='Keep editing']";
      await session.element(keepEditing);
      await session.click(keepEditing);
      await session.element(editor);
      assert.ok(
        await session.enabled(button("Save")),
        "Keep editing lost the dirty draft",
      );
      assert.ok(
        (await session.texts(editor)).some((text) =>
          text.includes("Unsaved Windows edit."),
        ),
      );
      await session.command("DELETE", "/window");
      await session.element(keepEditing);
      await session.click(button("Discard and continue"));
      await until(
        "discard to destroy the Windows window",
        async () =>
          (await session.command("GET", "/window/handles")).length === 0,
      );
      await session.end();
      await noWebviews(tray, "dirty discard close");
      assert.doesNotMatch(
        workspace.remoteFile("skills/after-close/SKILL.md"),
        /Unsaved Windows edit/,
      );
      await secondLaunch(workspace, tray, "tray-reopen-after-discard");
      session = await Session.start(tray.port, tray.running);
      await session.element(
        "//button[.//strong[normalize-space(.)='after-close']]",
      );
      const alerts = await session.texts("//*[@role='alert']");
      assert.ok(
        !alerts.some((text) =>
          /already exists|duplicate.*label|Cannot.*window/i.test(text),
        ),
        alerts.join(" | "),
      );
      await session.end();
    } catch (error) {
      await attempt(error, "Recording sync diagnostics", () =>
        diagnose(workspace, "tray"),
      );
      throw error;
    } finally {
      await tray.stop();
    }
  },
);

test(
  "macOS closing the only window quits the process",
  { skip: disabled || process.platform !== "darwin" },
  async () => {
    const workspace = createWorkspace();
    const desktop = await launch(workspace, "macos-close");
    try {
      const session = await Session.start(desktop.port, desktop.running);
      await session.element("//h2[normalize-space(.)='No skills yet']", 60000);
      // Exit may close the WebDriver connection before its response is flushed.
      const closing = Promise.allSettled([
        session.command("DELETE", "/window"),
      ]);
      await until("the window close to quit the app", () => !desktop.running());
      await closing;
    } finally {
      await desktop.stop();
    }
  },
);

test(
  "macOS dirty window keeps editing, then discard closes and quits",
  { skip: disabled || process.platform !== "darwin" },
  async () => {
    const workspace = createWorkspace();
    workspace.publish("Add close-check", {
      "skills/close-check/SKILL.md": skill(
        "close-check",
        "Checks the unsaved close guard.",
        "Original content.",
      ),
      "skills/close-check/skill.toml": "",
    });
    execFileSync("git", ["-C", workspace.content, "pull", "--ff-only"]);
    const desktop = await launch(workspace, "macos-dirty-close");
    try {
      const session = await Session.start(desktop.port, desktop.running);
      await session.click(
        "//button[.//strong[normalize-space(.)='close-check']]",
      );
      const editor = labelled("Skill Markdown editor");
      await session.click(editor);
      await session.type(editor, "Unsaved edit.");
      await until("the typed edit in the skill editor", async () =>
        (await session.texts(editor)).some((text) =>
          text.includes("Unsaved edit."),
        ),
      );
      // The compact layout hides the status text; Save appears only for an unsaved draft.
      await until("the unsaved draft to enable Save", () =>
        session.enabled(button("Save")),
      );
      await session.command("DELETE", "/window");
      const guard =
        "//dialog[@aria-label='You have unsaved changes']//button[normalize-space(.)='Keep editing']";
      await session.element(guard);
      assert.ok(desktop.running(), "Dirty window closed without prompting");
      await session.click(button("Keep editing"));
      await session.element(editor);
      assert.ok(desktop.running(), "Keep editing quit the app");
      assert.ok(
        (await session.texts(editor)).some((text) =>
          text.includes("Unsaved edit."),
        ),
        "Keep editing lost the unsaved text",
      );
      assert.ok(
        await session.enabled(button("Save")),
        "Keep editing lost the dirty draft",
      );
      await session.command("DELETE", "/window");
      await session.element(guard);
      // Quitting may close WebDriver before the click response is flushed.
      const closing = Promise.allSettled([
        session.click(button("Discard and continue")),
      ]);
      await until(
        "discard to close and quit the app",
        () => !desktop.running(),
      );
      await closing;
      assert.doesNotMatch(
        workspace.remoteFile("skills/close-check/SKILL.md"),
        /Unsaved edit/,
      );
    } finally {
      await desktop.stop();
    }
  },
);
