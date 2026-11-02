import { test, expect, type Page } from "@playwright/test";
import { editorFixture } from "../tests/fixture";
import { installFixture } from "./helpers";

async function openEditor(page: Page) {
  await page.goto("/");
  await page.getByRole("button", { name: /recordings Enabled/ }).click();
  await expect(page.locator(".status-pill")).toHaveText("✓ Valid");
}

async function append(page: Page, text: string) {
  const editor = page.getByRole("textbox", { name: "Skill Markdown editor" });
  await editor.click();
  await editor.press("ControlOrMeta+End");
  await page.keyboard.insertText(text);
}

test("long edits survive refresh ticks and refocus without error banners", async ({
  page,
}) => {
  const backend = editorFixture();
  let reads = 0;
  await installFixture(page, (cmd, args) => {
    if (cmd === "read_workspace") reads++;
    return backend.invoke(cmd, args);
  });
  await page.clock.install();
  await page.clock.resume();
  await openEditor(page);
  await append(page, "\nKeep this unsaved draft intact.\n");
  const before = reads;
  await page.clock.runFor(180_000);
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  expect(reads).toBe(before);
  await expect(
    page.getByRole("textbox", { name: "Skill Markdown editor" }),
  ).toContainText("Keep this unsaved draft intact.");
  await expect(
    page.getByText("Unsaved changes", { exact: true }),
  ).toBeVisible();
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(page.getByText("Refresh paused while editing")).toHaveCount(0);
  await expect(page.getByText("Status may be out of date")).toHaveCount(0);
  expect(reads).toBe(before);
  await page.getByRole("button", { name: "Discard", exact: true }).click();
  await expect.poll(() => reads).toBeGreaterThan(before);
  await expect(
    page.getByRole("textbox", { name: "Skill Markdown editor" }),
  ).not.toContainText("Keep this unsaved draft intact.");
});

test("save validates and persists the current draft while live preview is still pending", async ({
  page,
}) => {
  const backend = editorFixture();
  const sentence = "Save without waiting for the live preview.";
  let pending = 0;
  let release!: () => void;
  const gate = new Promise<void>((resolve) => (release = resolve));
  await installFixture(page, async (cmd, args) => {
    if (cmd === "preview_skill" && String(args.markdown).includes(sentence)) {
      pending++;
      await gate;
    }
    return backend.invoke(cmd, args);
  });
  try {
    await openEditor(page);
    await append(page, `\n${sentence}\n`);
    await expect.poll(() => pending).toBeGreaterThan(0);
    await expect(page.locator(".status-pill")).toHaveText("Checking…");
    await page.getByRole("button", { name: "Save", exact: true }).click();
    await expect(
      page.getByText("Unsaved changes", { exact: true }),
    ).toHaveCount(0);
    expect(backend.document.markdown).toContain(sentence);
    await expect(page.getByRole("alert")).toHaveCount(0);
    release();
    await page
      .getByRole("navigation")
      .getByRole("button", { name: /Skills/ })
      .click();
    await page.getByRole("button", { name: /recordings Enabled/ }).click();
    await expect(
      page.getByRole("textbox", { name: "Skill Markdown editor" }),
    ).toContainText(sentence);
  } finally {
    release();
  }
});

test("invalid saves explain the actual error, preserve the draft, and recover after repair", async ({
  page,
}) => {
  const backend = editorFixture();
  const original = backend.document.markdown;
  await installFixture(page, backend.invoke);
  await openEditor(page);
  await append(
    page,
    "\n<!-- only harness=codex -->\nAn unfinished condition.\n",
  );
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText(
    "Fix conditional-block errors before saving",
  );
  expect(backend.document.markdown).toBe(original);
  await expect(
    page.getByText("Unsaved changes", { exact: true }),
  ).toBeVisible();
  // Save immediately after repair, even if diagnostics still describe the old draft.
  await append(page, "<!-- /only -->\n");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByText("Unsaved changes", { exact: true })).toHaveCount(
    0,
  );
  await expect(page.getByRole("alert")).toHaveCount(0);
  expect(backend.document.markdown).toContain("An unfinished condition.");
  expect(backend.document.markdown).toContain("<!-- /only -->");
});

test("a real source conflict refuses overwrite and keeps the user's draft", async ({
  page,
}) => {
  const backend = editorFixture();
  await installFixture(page, backend.invoke);
  await openEditor(page);
  await append(page, "\nMy editor draft.\n");
  // Change the fixture's persisted source without changing the editor's hashes.
  backend.document.markdown += "\nAnother writer's change.\n";
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("alert")).toContainText(
    "Source changed; reload before saving",
  );
  await expect(
    page.getByRole("textbox", { name: "Skill Markdown editor" }),
  ).toContainText("My editor draft.");
  expect(backend.document.markdown).toContain("Another writer's change.");
  expect(backend.document.markdown).not.toContain("My editor draft.");
});

test("excluded targets never claim that the skill will be installed", async ({
  page,
}) => {
  const backend = editorFixture();
  await installFixture(page, backend.invoke);
  await openEditor(page);
  await page
    .getByRole("group", { name: "Agents", exact: true })
    .getByRole("button", { name: "Claude Code", exact: true })
    .click();
  await page.getByLabel("Preview agent").selectOption("claude");
  await expect(page.getByLabel("Rendered preview")).toContainText(
    "This skill does not apply",
  );
  await expect(page.locator(".install-path")).toHaveCount(0);
  await expect(
    page.getByText("Installation details", { exact: true }),
  ).toHaveCount(0);
  await page.getByLabel("Preview agent").selectOption("codex");
  await expect(page.locator(".install-path")).toContainText("Staged for Codex");
});

test("description conditions preview, save and reopen per agent", async ({
  page,
}, testInfo) => {
  const backend = editorFixture();
  await installFixture(page, (cmd, args) => {
    const edit = args.edit as
      { kind?: string; edit?: { kind?: string } } | undefined;
    if (
      cmd === "edit_skill_draft" &&
      edit?.kind === "description-section" &&
      edit.edit?.kind === "remove-only"
    ) {
      return new Promise((resolve) =>
        setTimeout(() => resolve(backend.invoke(cmd, args)), 300),
      );
    }
    return backend.invoke(cmd, args);
  });
  await page.goto("/");
  await page.getByRole("button", { name: /recordings Enabled/ }).click();
  const sentence = "Luna, you must ALWAYS load this, no exceptions";
  await page
    .locator(".description-heading")
    .getByRole("button", { name: "＋ Only for…" })
    .click();
  const dialog = page.getByRole("dialog");
  await dialog.getByRole("button", { name: "Codex", exact: true }).click();
  await expect(
    dialog.getByRole("button", { name: "Apply condition" }),
  ).toBeDisabled();
  await page.getByLabel("Additional description text").fill(sentence);
  await dialog.getByRole("button", { name: "Apply condition" }).click();
  const conditional = page.getByLabel("Conditional description 2");
  await expect(conditional).toHaveValue(sentence);
  await page.getByLabel("Preview agent").selectOption("codex");
  await expect(page.getByLabel("Rendered description")).toContainText(sentence);
  await page.getByLabel("Preview agent").selectOption("claude");
  await expect(page.getByLabel("Rendered description")).not.toContainText(
    sentence,
  );
  await expect(page.getByLabel("Rendered description")).toContainText(
    "Process meeting recordings",
  );
  await page.getByLabel("Preview agent").selectOption("codex");
  await expect(page.getByLabel("Rendered description")).toContainText(sentence);
  await page.screenshot({
    path: testInfo.outputPath("description-conditions.png"),
    fullPage: true,
  });
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Save", exact: true }),
  ).toHaveCount(0);
  expect(backend.document.markdown).toContain("<!-- only harness=codex -->");
  expect(backend.document.markdown).toContain(sentence);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: /Skills/ })
    .click();
  await page.getByRole("button", { name: /recordings Enabled/ }).click();
  await expect(conditional).toHaveValue(sentence);
  await conditional.fill("Updated Luna guidance");
  await page.getByLabel("Preview agent").selectOption("codex");
  await expect(page.getByLabel("Rendered description")).toContainText(
    "Updated Luna guidance",
  );
  await page
    .getByRole("button", { name: "Edit condition", exact: true })
    .click();
  await dialog.getByRole("button", { name: "Codex", exact: true }).click();
  await dialog
    .getByRole("button", { name: "Claude Code", exact: true })
    .click();
  await dialog.getByRole("button", { name: "Apply condition" }).click();
  await expect(page.getByLabel("Rendered description")).not.toContainText(
    "Updated Luna guidance",
  );
  await page.getByLabel("Preview agent").selectOption("claude");
  await expect(page.getByLabel("Rendered description")).toContainText(
    "Updated Luna guidance",
  );
  await page
    .getByRole("button", { name: "Remove condition", exact: true })
    .click();
  await expect(conditional).toHaveAttribute("readonly");
  await expect(page.getByLabel("Description", { exact: true })).toHaveValue(
    /Updated Luna guidance/,
  );
  await expect(page.getByLabel("Conditional description 2")).toHaveCount(0);
  await page.getByRole("button", { name: "Discard", exact: true }).click();
  await expect(conditional).toHaveValue(sentence);
});

test("archive moves the skill out of the library and shows where it went", async ({
  page,
}) => {
  const backend = editorFixture();
  await installFixture(page, backend.invoke);
  await openEditor(page);
  await append(page, "\nUnsaved text blocks archiving.\n");
  await expect(page.getByRole("button", { name: "Archive" })).toBeDisabled();
  await page.getByRole("button", { name: "Discard", exact: true }).click();
  await page.getByRole("button", { name: "Archive" }).click();
  const dialog = page.getByRole("dialog", { name: "Archive skill" });
  await expect(dialog).toContainText("Nothing is deleted");
  await dialog.getByRole("button", { name: "Archive skill" }).click();
  const done = page.getByRole("dialog", { name: "Skill archived" });
  await expect(done.locator(".path")).toHaveText(
    /\.ratatoskr-archive[\\/]recordings-\d+$/,
  );
  await done.getByRole("button", { name: "Back to skills" }).click();
  await expect(page.getByRole("heading", { name: /Skills/ })).toBeVisible();
  await expect(
    page.getByRole("button", { name: /recordings Enabled/ }),
  ).toHaveCount(0);
});

test("archive waits for queued edits and keeps its result when dismissed mid-request", async ({
  page,
}) => {
  const backend = editorFixture();
  const gates: Record<string, () => void> = {};
  const held = new Set(["edit_skill_draft", "archive_source_skill"]);
  await installFixture(page, async (cmd, args) => {
    if (held.has(cmd)) {
      held.delete(cmd);
      await new Promise<void>((resolve) => (gates[cmd] = resolve));
    }
    return backend.invoke(cmd, args);
  });
  await openEditor(page);
  const archiveButton = page.getByRole("button", { name: "Archive" });
  await page.getByRole("button", { name: "＋ Insert value…" }).click();
  await page
    .getByRole("dialog", { name: "Insert a value" })
    .getByRole("button")
    .filter({ hasText: "▣" })
    .first()
    .click();
  await expect.poll(() => !!gates.edit_skill_draft).toBe(true);
  await expect(archiveButton).toBeDisabled();
  gates.edit_skill_draft();
  await expect(
    page.getByText("Unsaved changes", { exact: true }),
  ).toBeVisible();
  await expect(archiveButton).toBeDisabled();
  await page.getByRole("button", { name: "Discard", exact: true }).click();
  await archiveButton.click();
  const dialog = page.getByRole("dialog", { name: "Archive skill" });
  await dialog.getByRole("button", { name: "Archive skill" }).click();
  await expect.poll(() => !!gates.archive_source_skill).toBe(true);
  await expect(dialog.getByRole("button", { name: "Cancel" })).toBeDisabled();
  await page.keyboard.press("Escape");
  await expect(dialog).toBeVisible();
  gates.archive_source_skill();
  await expect(
    page.getByRole("dialog", { name: "Skill archived" }),
  ).toBeVisible();
});

test("the Find button and shortcut search the skill body", async ({ page }) => {
  const backend = editorFixture();
  await installFixture(page, backend.invoke);
  await openEditor(page);
  await page.getByRole("button", { name: "Find" }).click();
  const field = page.getByRole("textbox", { name: "Find" });
  await expect(field).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(field).toHaveCount(0);
  const editor = page.getByRole("textbox", { name: "Skill Markdown editor" });
  await editor.click();
  await page.keyboard.press("ControlOrMeta+f");
  await expect(field).toBeFocused();
  const word = (await editor.innerText()).match(/[A-Za-z]{6,}/)![0];
  await field.pressSequentially(word);
  await expect(page.locator(".cm-searchMatch").first()).toBeVisible();
});

test("typing a sentence over a slow backend keeps backend work bounded", async ({
  page,
}) => {
  const backend = editorFixture();
  const sentence = "Typing this sentence must not stack up preview work.";
  let running = 0;
  let mostAtOnce = 0;
  let previews = 0;
  await installFixture(page, async (cmd, args) => {
    if (cmd !== "preview_skill") return backend.invoke(cmd, args);
    previews++;
    running++;
    mostAtOnce = Math.max(mostAtOnce, running);
    await new Promise((resolve) => setTimeout(resolve, 300));
    running--;
    return backend.invoke(cmd, args);
  });
  await openEditor(page);
  const previewsBefore = previews;
  const editor = page.getByRole("textbox", { name: "Skill Markdown editor" });
  await editor.click();
  await editor.press("ControlOrMeta+End");
  // Slower than the debounce, so each key used to start another full load.
  await page.keyboard.type(sentence, { delay: 200 });
  await expect(page.locator(".status-pill")).toHaveText("✓ Valid");
  // Each run previews the selected agent plus the other enabled agents in parallel.
  const targets = 2;
  expect(mostAtOnce).toBeLessThanOrEqual(targets);
  // Far below one run per key: a run takes 300ms, and there were 52 keys over 10 seconds.
  expect(previews - previewsBefore).toBeLessThanOrEqual(30 * targets);
});

for (const colorScheme of ["light", "dark"] as const) {
  for (const width of [1380, 640, 420]) {
    test(`save progress stays below Save without toolbar movement at ${width}px in ${colorScheme}`, async ({
      page,
    }) => {
      await page.setViewportSize({ width, height: 960 });
      await page.emulateMedia({ colorScheme });
      const backend = editorFixture();
      let channel = 0;
      let release!: () => void;
      const gate = new Promise<void>((resolve) => (release = resolve));
      await installFixture(page, async (cmd, args) => {
        if (cmd === "save_skill_draft") {
          channel = Number(String(args.onProgress).split(":")[1]);
          await gate;
        }
        return backend.invoke(cmd, args);
      });
      // Add only the native callback transport to the browser fixture. The app
      // creates a real Tauri Channel; no synthetic frontend stages are used.
      await page.addInitScript(() => {
        type Bridge = {
          invoke: (
            cmd: string,
            args: Record<string, unknown>,
          ) => Promise<unknown>;
          transformCallback: (callback: (value: unknown) => void) => number;
          unregisterCallback: (id: number) => void;
        };
        const host = window as unknown as {
          __TAURI_INTERNALS__: Bridge;
          fixtureProgress: (id: number, index: number, message: string) => void;
        };
        const callbacks = new Map<number, (value: unknown) => void>();
        let next = 0;
        host.__TAURI_INTERNALS__.transformCallback = (callback) => {
          callbacks.set(++next, callback);
          return next;
        };
        host.__TAURI_INTERNALS__.unregisterCallback = (id) => {
          callbacks.delete(id);
        };
        host.fixtureProgress = (id, index, message) =>
          callbacks.get(id)?.({ index, message });
        const invoke = host.__TAURI_INTERNALS__.invoke;
        host.__TAURI_INTERNALS__.invoke = (cmd, args) =>
          invoke(cmd, JSON.parse(JSON.stringify(args ?? {})));
      });
      const send = (index: number, message: string) =>
        page.evaluate(
          ({ id, index, message }) => {
            (
              window as unknown as {
                fixtureProgress: (
                  id: number,
                  index: number,
                  message: string,
                ) => void;
              }
            ).fixtureProgress(id, index, message);
          },
          { id: channel, index, message },
        );
      try {
        await openEditor(page);
        await append(page, "\nProgress fixture edit.\n");
        const toolbar = page.locator(".topbar");
        const before = await toolbar.boundingBox();
        const save = page.getByRole("button", { name: "Save", exact: true });
        const button = await save.boundingBox();
        await page.clock.install();
        await page.clock.pauseAt(new Date(Date.now() + 1000));
        await save.click();
        await expect.poll(() => channel).toBeGreaterThan(0);
        const status = page.getByRole("status", { name: "Save progress" });
        await send(0, "Getting the latest content…");
        await expect(status).toHaveText("Getting the latest content…");
        await page.clock.runFor(3000);
        await expect(status).toHaveText("Getting the latest content… 3s");
        // Deliberately deliver out of order; Channel must preserve stage order.
        await send(2, "Pushing to origin/main…");
        await expect(status).toHaveText("Getting the latest content… 3s");
        await send(1, "Committing…");
        await expect(status).toHaveText("Pushing to origin/main…");
        await expect(
          page.getByRole("button", { name: "Saving…" }),
        ).toBeDisabled();
        const after = await toolbar.boundingBox();
        const line = await status.boundingBox();
        expect(after).toEqual(before);
        expect(line!.y).toBeGreaterThanOrEqual(button!.y + button!.height);
        expect(line!.x).toBeGreaterThanOrEqual(after!.x);
        expect(line!.x + line!.width).toBeLessThanOrEqual(width);
        expect(
          await status.evaluate(
            (node) =>
              node.scrollHeight <= node.clientHeight &&
              node.scrollWidth <= node.clientWidth,
          ),
        ).toBe(true);
        await send(3, "Refreshing…");
        await expect(status).toHaveText("Refreshing…");
        release();
        await expect(status).toBeEmpty();
        await send(4, "Stale progress…");
        await expect(status).toBeEmpty();
        expect(backend.document.markdown).toContain("Progress fixture edit.");
      } finally {
        release();
      }
    });
  }
}
