import { test, expect, type Page } from "@playwright/test";
import { instructionsFixture } from "../tests/fixture";
import { installFixture } from "./helpers";

async function openInstructions(page: Page) {
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: /Global instructions/ })
    .click();
  await expect(page.locator(".status-pill")).toHaveText("✓ Valid");
}

async function append(page: Page, text: string) {
  const editor = page.getByRole("textbox", {
    name: "Global instructions editor",
  });
  await editor.click();
  await editor.press("ControlOrMeta+End");
  await page.keyboard.insertText(text);
}

test("previews each agent and machine with its own destination", async ({
  page,
}) => {
  const backend = instructionsFixture();
  await installFixture(page, (cmd, args) => backend.invoke(cmd, args));
  await openInstructions(page);
  const preview = page.getByLabel("Rendered preview");
  await expect(preview).toContainText(
    "Meeting recordings are archived in /archive/recordings.",
  );
  await expect(preview).toContainText("Run commands inside the sandbox");
  await expect(page.locator(".live-preview .path")).toContainText(
    "/staging/codex/AGENTS.md",
  );
  // Codex-only text marks the line that differs from Claude's version.
  await expect(
    page.locator(".changed-line", { hasText: "Run commands inside" }),
  ).toHaveCount(1);
  await page.getByLabel("Preview agent").selectOption("claude");
  await expect(preview).not.toContainText("Run commands inside the sandbox");
  await expect(page.locator(".live-preview .path")).toContainText(
    "/staging/claude/CLAUDE.md",
  );
  await page.getByLabel("Preview machine").selectOption("loki");
  await expect(preview).toContainText("C:\\Users\\Oana\\Recordings");
  await expect(preview).toContainText("Use PowerShell syntax");
  await expect(page.getByLabel("Preview agent")).toHaveValue("claude");
});

test("saves through the core and keeps an invalid draft with its error", async ({
  page,
}) => {
  const backend = instructionsFixture();
  await installFixture(page, (cmd, args) => backend.invoke(cmd, args));
  await openInstructions(page);
  await append(page, "Notes live in {{af:paths.missing}}.\n");
  await expect(page.locator(".status-pill")).toHaveText("Fix errors");
  await expect(page.locator(".inline-diagnostic")).toContainText(
    "Undefined variable: paths.missing",
  );
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByRole("alert").first()).toContainText("paths.missing");
  expect(backend.document.text).not.toContain("Notes live in");
  await expect(
    page.getByRole("textbox", { name: "Global instructions editor" }),
  ).toContainText("Notes live in");

  await page.getByRole("button", { name: "Discard", exact: true }).click();
  await expect(page.locator(".status-pill")).toHaveText("✓ Valid");
  await append(page, "Prefer small, reviewable pull requests.\n");
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByText("Unsaved changes", { exact: true })).toHaveCount(
    0,
  );
  expect(backend.document.text).toContain(
    "Prefer small, reviewable pull requests.",
  );
});

test("Ctrl/Cmd+F searches the instructions from anywhere on the screen", async ({
  page,
}) => {
  const backend = instructionsFixture();
  await installFixture(page, (cmd, args) => backend.invoke(cmd, args));
  await openInstructions(page);
  await page.getByRole("heading", { name: "Global instructions" }).click();
  await page.keyboard.press("ControlOrMeta+f");
  const field = page.getByRole("textbox", { name: "Find" });
  await expect(field).toBeFocused();
  await field.pressSequentially("sandbox");
  await expect(page.locator(".cm-searchMatch")).toHaveCount(1);
  await page.keyboard.press("Escape");
  await expect(field).toHaveCount(0);
});

test("clean instructions auto-reload on focus and non-overlapping save conflicts combine cleanly", async ({
  page,
}) => {
  const backend = instructionsFixture();
  await installFixture(page, (cmd, args) => backend.invoke(cmd, args));
  await openInstructions(page);
  const editor = page.getByRole("textbox", {
    name: "Global instructions editor",
  });

  // External change while clean is picked up automatically when the window refocuses.
  backend.writeExternal(`${backend.document.text}Synced background line.\n`);
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect(editor).toContainText("Synced background line.");

  // Now edit locally while another commit modifies an earlier line on disk.
  await append(page, "My new subagent guidance.\n");
  backend.writeExternal(
    backend.document.text.replace(
      "Synced background line.",
      "Synced background line (updated remotely).",
    ),
  );

  await page.getByRole("button", { name: "Save", exact: true }).click();
  const alert = page.getByRole("alert");
  await expect(alert).toContainText("Source changed; reload before saving");
  await expect(editor).toContainText("My new subagent guidance.");

  // Review changes opens the side-by-side diff and offers a clean 3-way combine.
  await alert.getByRole("button", { name: "Review changes…" }).click();
  const dialog = page.getByRole("dialog", { name: "Source changed on disk" });
  await expect(dialog).toContainText(
    "Synced background line (updated remotely).",
  );
  await expect(dialog).toContainText("My new subagent guidance.");
  await dialog.getByRole("button", { name: "Combine into draft" }).click();

  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(editor).toContainText(
    "Synced background line (updated remotely).",
  );
  await expect(editor).toContainText("My new subagent guidance.");
  await expect(
    page.getByText("Unsaved changes", { exact: true }),
  ).toBeVisible();

  // Saving the reviewed combined draft now succeeds against the latest hash.
  await page.getByRole("button", { name: "Save", exact: true }).click();
  await expect(page.getByText("Unsaved changes", { exact: true })).toHaveCount(
    0,
  );
  expect(backend.document.text).toContain(
    "Synced background line (updated remotely).",
  );
  expect(backend.document.text).toContain("My new subagent guidance.");
});

test("overlapping instructions conflicts explain the overlap and let the user reload or keep draft", async ({
  page,
}) => {
  const backend = instructionsFixture();
  await installFixture(page, (cmd, args) => backend.invoke(cmd, args));
  await openInstructions(page);
  const editor = page.getByRole("textbox", {
    name: "Global instructions editor",
  });

  // Both sides append different text at the exact same spot.
  await append(page, "Local conflicting bullet.\n");
  backend.writeExternal(`${backend.document.text}Remote conflicting bullet.\n`);

  await page.getByRole("button", { name: "Save", exact: true }).click();
  const alert = page.getByRole("alert");
  await expect(alert).toContainText("Source changed; reload before saving");
  await alert.getByRole("button", { name: "Review changes…" }).click();

  const dialog = page.getByRole("dialog", { name: "Source changed on disk" });
  await expect(dialog).toContainText("modified the same section");
  await expect(
    dialog.getByRole("button", { name: "Combine into draft" }),
  ).toHaveCount(0);
  await dialog
    .getByRole("button", { name: "Reload latest (discard draft)" })
    .click();

  await expect(dialog).toHaveCount(0);
  await expect(page.getByRole("alert")).toHaveCount(0);
  await expect(editor).toContainText("Remote conflicting bullet.");
  await expect(editor).not.toContainText("Local conflicting bullet.");
  await expect(page.getByText("Unsaved changes", { exact: true })).toHaveCount(
    0,
  );
});
