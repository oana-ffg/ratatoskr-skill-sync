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
    /conditional|unclosed|closing/i,
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
