import type { Plan, StalePreview } from "../src/api";
import { test, expect } from "@playwright/test";
import { mkdirSync } from "node:fs";
import {
  fixtureIPC,
  projectWorkspace,
  fixture,
  oracle,
} from "../tests/fixture";
import { installFixture } from "./helpers";
test("all main screens", async ({ page }) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  const errors: string[] = [];
  page.on("pageerror", (e) => errors.push(e.message));
  await page.clock.install({ time: new Date("2026-10-16T18:00:10Z") });
  let refreshError = false;
  await installFixture(page, (cmd, args) => {
    if (cmd === "read_workspace" && refreshError)
      throw new Error("Fixture sync history is unavailable");
    return fixtureIPC(cmd, args);
  });
  mkdirSync("../docs/screenshots", { recursive: true });
  async function capture(name: string) {
    const viewport = page.viewportSize()!;
    if (!(await page.getByRole("dialog").count())) {
      const height = await page
        .locator("main")
        .evaluate((main) => main.scrollHeight + 72);
      await page.setViewportSize({
        width: viewport.width,
        height: Math.max(viewport.height, height),
      });
    }
    await page.screenshot({
      path: `../docs/screenshots/${name}-dark.png`,
      fullPage: true,
      animations: "disabled",
    });
    await page.setViewportSize(viewport);
  }
  // Window-sized crop of a long screen, for the README.
  async function captureOverview(name: string) {
    await page.screenshot({
      path: `../docs/screenshots/${name}-overview-dark.png`,
      animations: "disabled",
    });
  }
  await page.goto("/");
  await expect(page.getByRole("heading", { name: /Skills/ })).toBeVisible();
  await expect(
    page.getByRole("button", { name: "Save", exact: true }),
  ).toHaveCount(0);
  await expect(page.locator(".snapshot-age")).toHaveText("As of just now");
  await capture("skills");
  await page.getByRole("button", { name: /recordings Enabled/ }).click();
  await page.clock.runFor(250);
  await expect(page.locator(".only-header").first()).toBeVisible();
  await expect(page.locator(".inline-diagnostic")).toContainText(
    "matches no enabled",
  );
  await expect(page.getByLabel("Rendered preview")).toContainText(
    "Use the automation tools",
  );
  await expect(page.locator(".value-pill")).toContainText("Recordings archive");
  const bands = page.locator(".cm-line.only-band");
  await expect(
    bands.filter({ hasText: "Use the automation tools" }),
  ).toHaveCount(1);
  await expect(bands.filter({ hasText: "Use Get-FileHash" })).toHaveCount(1);
  await expect(
    page.getByRole("button", { name: "Save", exact: true }),
  ).toHaveCount(0);
  await capture("skill-editor");
  await page
    .locator(".format-toolbar")
    .getByRole("button", { name: "＋ Only for…", exact: true })
    .click();
  await page
    .getByRole("dialog")
    .getByRole("button", { name: "Codex", exact: true })
    .click();
  await expect(page.getByRole("dialog")).toContainText("Shows for: Codex");
  await capture("only-for-picker");
  await page.getByRole("button", { name: "Cancel", exact: true }).click();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: /Skills/ })
    .click();
  await page
    .getByRole("button", { name: /aws-project-migration Enabled/ })
    .click();
  await page.clock.runFor(250);
  await expect(page.getByLabel("Description", { exact: true })).toHaveValue(
    /Inspect, migrate, deploy/,
  );
  const description = page.getByLabel("Description", { exact: true });
  await expect
    .poll(() =>
      description.evaluate(
        (node: HTMLTextAreaElement) => node.scrollHeight - node.clientHeight,
      ),
    )
    .toBeLessThanOrEqual(1);
  await expect(page.locator(".editor-source .eyebrow")).not.toContainText([
    "GLOBAL SKILL",
  ]);
  await expect(page.locator(".cm-gutters")).toHaveCount(0);
  await expect(page.locator(".cm-content")).not.toContainText("description:");
  // A long wrapped YAML description must contribute no hidden block height.
  const offset = await page.locator(".cm-content").evaluate((content) => {
    const line = [...content.querySelectorAll(".cm-line")].find((line) =>
      line.textContent?.includes("# AWS project migration"),
    )!;
    return (
      line.getBoundingClientRect().top - content.getBoundingClientRect().top
    );
  });
  expect(offset).toBeLessThan(35);
  await expect(page.locator(".status-pill.success")).toHaveText("✓ Valid");
  await capture("realistic-skill-editor");
  await page
    .getByRole("button", { name: "Markdown source", exact: true })
    .click();
  await page.clock.runFor(100);
  await expect(page.locator(".cm-lineNumbers")).toBeVisible();
  await expect(page.locator(".cm-content")).toContainText("description:");
  await capture("markdown-source");
  await page
    .getByRole("button", { name: "Markdown source", exact: true })
    .click();
  const originalDescription = await description.inputValue();
  await description.fill(
    Array.from({ length: 30 }, (_, i) => `Description line ${i + 1}`).join(
      "\n",
    ),
  );
  await expect
    .poll(() =>
      description.evaluate((node: HTMLTextAreaElement) => node.clientHeight),
    )
    .toBeGreaterThan(200);
  expect(
    await description.evaluate(
      (node: HTMLTextAreaElement) => node.getBoundingClientRect().height,
    ),
  ).toBeLessThanOrEqual(256);
  expect(
    await description.evaluate(
      (node: HTMLTextAreaElement) => node.scrollHeight,
    ),
  ).toBeGreaterThan(256);
  await description.fill(originalDescription);
  await expect
    .poll(() =>
      description.evaluate(
        (node: HTMLTextAreaElement) => node.scrollHeight - node.clientHeight,
      ),
    )
    .toBeLessThanOrEqual(1);
  await page.getByRole("button", { name: "Discard", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Save", exact: true }),
  ).toHaveCount(0);
  await page
    .getByRole("navigation")
    .getByRole("button", { name: /Global instructions/ })
    .click();
  await page.clock.runFor(250);
  await expect(page.locator(".status-pill.success")).toHaveText("✓ Valid");
  await expect(page.locator(".only-header").first()).toBeVisible();
  await expect(page.getByLabel("Rendered preview")).toContainText(
    "Run commands inside the sandbox",
  );
  await capture("global-instructions");
  for (const [screen, file] of [
    ["Review changes", "review-changes"],
    ["Projects", "projects"],
    ["Agents & machines", "agents-machines"],
    ["Activity", "activity"],
    ["Settings", "settings"],
  ]) {
    await page
      .getByRole("navigation")
      .getByRole("button", { name: new RegExp(screen) })
      .click();
    await expect(
      page.getByRole("heading", { name: new RegExp(screen), level: 1 }),
    ).toBeVisible();
    if (screen === "Review changes") {
      await expect(
        page.getByRole("button", {
          name: "Write 17 files in the staging folder",
          exact: true,
        }),
      ).toBeVisible();
      await expect(page.locator(".page-heading .muted")).toContainText(
        "5 items needing attention · 17 ready files · 2 decisions",
      );
      await expect(
        page.locator(".conflict-card .diff-add, .conflict-card .diff-remove"),
      ).toHaveCount(0);
      await expect(
        page.locator(".conflict-card .word-change").first(),
      ).toBeVisible();
      const claude = page.locator(".change-card").filter({
        has: page.getByRole("heading", {
          name: "aws-project-migration → Claude Code: 8 files",
          exact: true,
        }),
      });
      await claude.locator(".group-files > summary").click();
      await claude
        .locator(".review-file > summary")
        .filter({ hasText: "SKILL.md" })
        .click();
      await expect(
        claude.locator(".review-file .diff-grid h3").first(),
      ).toHaveText("Your source (rendered)");
      await expect(
        page.getByRole("region", { name: "Older Codex copies" }),
      ).toContainText("Nothing changes until you choose");
      await expect(page.locator("#apply-destination")).toContainText(
        "Your real agent folders are not touched",
      );
      await expect(
        page.getByRole("heading", {
          name: /Codex’s copy of recordings was edited on Brunnr/,
        }),
      ).toBeVisible();
    }
    if (screen === "Agents & machines") {
      await expect(
        page.getByRole("heading", { name: "Machine profiles", exact: true }),
      ).toBeVisible();
      const loki = page
        .locator(".machine-card")
        .filter({ has: page.getByRole("heading", { name: "▣ Loki" }) });
      await expect(
        loki.getByRole("textbox", { name: "Home", exact: true }),
      ).toHaveValue("C:\\Users\\Oana");
      await expect(
        loki.getByRole("button", { name: "Choose Home folder" }),
      ).toHaveCount(1);
      const main = page.locator("main");
      await main.evaluate(
        (element) => (element.scrollTop = element.scrollHeight),
      );
      await expect(page.getByRole("navigation")).toBeVisible();
      await expect(page.locator(".topbar")).toBeVisible();
      expect(
        await page.evaluate(() => document.documentElement.scrollTop),
      ).toBe(0);
      await main.evaluate((element) => (element.scrollTop = 0));
    }
    if (screen === "Settings") {
      await expect(page.getByLabel("Open Ratatoskr at login")).toBeVisible();
      await expect(
        page.getByRole("button", { name: "Install signed update" }),
      ).toHaveCount(0);
      await expect(
        page.getByRole("button", { name: "Restart app" }),
      ).toHaveCount(0);
    }
    await expect(
      page.getByRole("button", { name: "Save", exact: true }),
    ).toHaveCount(0);
    if (screen === "Settings") {
      await page
        .getByRole("combobox", { name: "Delivery", exact: true })
        .selectOption("pushover");
      await expect(
        page.getByLabel("Application token secret store"),
      ).toHaveValue("gopass");
      await page
        .getByLabel("Application token gopass entry")
        .fill("pushover/codex-approvals/app-token");
      await page
        .getByLabel("User key gopass entry")
        .fill("pushover/codex-approvals/user-key");
    }
    // Project counts use a dedicated populated fixture below.
    if (screen !== "Projects") await capture(file);
    if (screen === "Review changes" || screen === "Agents & machines") {
      await captureOverview(file);
    }
    if (screen === "Settings") {
      await page
        .getByRole("combobox", { name: "Delivery", exact: true })
        .selectOption("ntfy");
      await page.getByLabel("Topic requires a bearer token").check();
      await capture("ntfy-settings");
    }
  }
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect(page.getByText("Refresh paused while editing")).toHaveCount(0);
  await expect(page.getByText("Status may be out of date")).toHaveCount(0);
  await capture("editing-with-refresh-deferred");
  await page.getByRole("button", { name: "Discard", exact: true }).click();
  await expect(
    page.getByRole("button", { name: "Save", exact: true }),
  ).toHaveCount(0);
  await expect(page.getByText("Refresh paused while editing")).toHaveCount(0);
  refreshError = true;
  await page.evaluate(() => window.dispatchEvent(new Event("focus")));
  await expect(page.getByRole("alert")).toContainText(
    "Fixture sync history is unavailable",
  );
  await expect(page.getByRole("alert")).toContainText("Last good snapshot:");
  await capture("refresh-error");
  refreshError = false;
  await page.getByRole("button", { name: "Retry refresh" }).click();
  await expect(page.getByRole("alert")).toHaveCount(0);
  expect(errors).toEqual([]);
});

test("project cards with source skills", async ({ page }) => {
  await page.clock.install({ time: new Date("2026-10-16T18:00:10Z") });
  const workspace = projectWorkspace();
  await installFixture(page, (cmd, args) =>
    cmd === "read_workspace" ? workspace : fixtureIPC(cmd, args),
  );
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: /Projects/ })
    .click();
  await expect(page.getByText("2 skills", { exact: true })).toBeVisible();
  mkdirSync("../docs/screenshots", { recursive: true });
  await page.screenshot({
    path: "../docs/screenshots/projects-dark.png",
    fullPage: true,
  });
});

test("review former target decisions and their exact archive preview", async ({
  page,
}) => {
  const stale = oracle("stale_fixture") as {
    plan: Plan;
    preview: StalePreview;
  };
  const workspace = structuredClone(fixture.workspace);
  workspace.plan = stale.plan;
  workspace.plan.content_status = fixture.workspace.plan.content_status;
  workspace.legacy = [];
  await installFixture(page, (cmd, args) => {
    if (cmd === "read_workspace") return workspace;
    if (cmd === "preview_stale_target") return stale.preview;
    return fixtureIPC(cmd, args);
  });
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: /Review changes/ })
    .click();
  const section = page.getByRole("region", { name: "Former target locations" });
  await expect(
    section.getByRole("heading", { name: "Codex", exact: true }),
  ).toHaveCount(2);
  await expect(section).toContainText(
    "Hand-edited, permission-changed or missing",
  );
  await expect(
    page.getByRole("button", { name: "No files to apply" }),
  ).toBeDisabled();
  mkdirSync("../docs/screenshots", { recursive: true });
  await page.screenshot({
    path: "../docs/screenshots/former-targets-dark.png",
    fullPage: true,
  });
  await section
    .getByRole("button", {
      name: `Remove from ${stale.preview.location.path}…`,
      exact: true,
    })
    .click();
  const dialog = page.getByRole("dialog");
  await expect(dialog).toContainText("Nothing is deleted");
  await expect(dialog).toContainText("Untracked file");
  await page.screenshot({
    path: "../docs/screenshots/former-target-preview-dark.png",
    fullPage: true,
  });
  await dialog.getByRole("button", { name: "Cancel", exact: true }).click();
});
