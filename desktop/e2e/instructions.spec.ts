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
