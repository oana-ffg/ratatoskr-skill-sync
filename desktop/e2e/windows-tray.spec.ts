import { test, expect } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { fixtureIPC } from "../tests/fixture";
test("Windows tray settings in dark mode", async ({ page }) => {
  await page.emulateMedia({ colorScheme: "dark", reducedMotion: "reduce" });
  await page.clock.install({ time: new Date("2026-10-18T12:00:00Z") });
  await page.exposeFunction(
    "fixtureIPC",
    (cmd: string, args: Record<string, unknown>) => {
      if (cmd === "background_status")
        return {
          paused: false,
          interval_seconds: 60,
          running: false,
          last_success: 1792324740,
          skills: 29,
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
  await expect(page.getByLabel("Tray sync")).toContainText("29 skills");
  await expect(page.getByRole("dialog")).toHaveCount(0);
  expect(
    await page
      .locator("body")
      .evaluate((body) => getComputedStyle(body).backgroundColor),
  ).toBe("rgb(16, 21, 16)");
  mkdirSync("../docs/screenshots", { recursive: true });
  await page.screenshot({
    path: "../docs/screenshots/windows-tray-settings-dark.png",
    fullPage: true,
  });
});
