import { test, expect } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { fixture, fixtureIPC } from "../tests/fixture";
test("Windows tray settings in dark mode", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark", reducedMotion: "reduce" });
  await page.clock.install({ time: new Date("2026-10-18T12:00:00Z") });
  const workspace = structuredClone(fixture.workspace);
  const lastSuccess = Math.floor(
    new Date("2026-10-18T11:59:00Z").getTime() / 1000,
  );
  workspace.plan.machine = "loki";
  workspace.plan.mode = "live";
  workspace.plan.skills = workspace.skills.length;
  workspace.plan.changes = [];
  workspace.plan.conflicts = [];
  workspace.plan.conflict_details = [];
  workspace.legacy = [];
  workspace.jobs.last_run = {
    started_at: lastSuccess - 1,
    finished_at: lastSuccess,
    error: null,
  };
  workspace.jobs.log = "Sync completed successfully";
  await page.exposeFunction(
    "fixtureIPC",
    (cmd: string, args: Record<string, unknown>) => {
      if (cmd === "read_workspace") return workspace;
      if (cmd === "login_enabled") return true;
      if (cmd === "read_local_settings")
        return {
          settings: {
            ...fixture.local_settings,
            machine: workspace.plan.machine,
          },
          hash: "fixture-config-hash",
        };
      if (cmd === "background_status")
        return {
          login_enabled: true,
          paused: false,
          interval_seconds: 60,
          running: false,
          last_success: lastSuccess,
          skills: workspace.skills.length,
          error: null,
          migration_error: null,
        };
      return fixtureIPC(cmd, args);
    },
  );
  await page.addInitScript(() => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      value: {
        invoke: (cmd: string, args: Record<string, unknown>) =>
          (
            window as unknown as {
              fixtureIPC: (
                cmd: string,
                args: Record<string, unknown>,
              ) => Promise<unknown>;
            }
          ).fixtureIPC(cmd, args),
      },
    });
  });
  await page.goto("/");
  await expect(page.getByRole("heading", { name: /Skills/ })).toBeVisible();
  await page
    .getByRole("navigation")
    .getByRole("button", { name: /Settings/ })
    .click();
  await expect(
    page.getByRole("heading", { name: "Background sync" }),
  ).toBeVisible();
  await expect(page.getByLabel("Tray sync")).toContainText("Active");
  await expect(page.getByLabel("Tray sync")).toContainText(
    `${workspace.skills.length} skills`,
  );
  await expect(page.getByLabel("Tray sync")).toContainText(
    "Last success: 1 min ago",
  );
  await expect(page.locator(".topbar")).toContainText("Synced 1 min ago");
  await expect(page.locator(".mode-badge")).toHaveText("Live");
  await expect(
    page
      .getByRole("navigation")
      .getByRole("button", { name: /Skills/ })
      .locator(".nav-count"),
  ).toHaveText(String(workspace.skills.length));
  await expect(page.getByLabel("Open Ratatoskr at login")).toBeChecked();
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(
    await page
      .locator("body")
      .evaluate((body) => getComputedStyle(body).backgroundColor),
  ).toBe("rgb(16, 21, 16)");
  await page.setViewportSize({
    width: 1380,
    height: Math.max(
      960,
      await page.locator("main").evaluate((main) => main.scrollHeight + 72),
    ),
  });
  mkdirSync("../docs/screenshots", { recursive: true });
  await page.screenshot({
    path: "../docs/screenshots/windows-tray-settings-dark.png",
    fullPage: true,
  });
});
