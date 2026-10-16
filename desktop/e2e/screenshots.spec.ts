import { test, expect } from "@playwright/test";
import { mkdirSync } from "node:fs";
import { fixtureIPC } from "../tests/fixture";
for (const theme of ["light", "dark"] as const) {
  test(`all main screens · ${theme}`, async ({ page }) => {
    await page.emulateMedia({ colorScheme: theme, reducedMotion: "reduce" });
    const errors: string[] = [];
    page.on("pageerror", (e) => errors.push(e.message));
    await page.clock.install({ time: new Date("2026-10-16T18:00:10Z") });
    await page.exposeFunction(
      "fixtureIPC",
      (cmd: string, args: Record<string, unknown>) => fixtureIPC(cmd, args),
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
    mkdirSync("../docs/screenshots", { recursive: true });
    async function capture(name: string) {
      await page.screenshot({
        path: `../docs/screenshots/${name}-${theme}.png`,
        fullPage: true,
        animations: "disabled",
      });
    }
    await page.goto("/");
    await expect(page.getByRole("heading", { name: /Skills/ })).toBeVisible();
    await capture("skills");
    await page.getByRole("button", { name: /recordings Ready/ }).click();
    await page.clock.runFor(250);
    await expect(page.locator(".only-header").first()).toBeVisible();
    await expect(page.locator(".inline-diagnostic")).toContainText(
      "matches no enabled",
    );
    await expect(page.getByLabel("Rendered preview")).toContainText(
      "Use the automation tools",
    );
    await expect(page.locator(".value-pill")).toContainText(
      "Recordings archive",
    );
    await capture("skill-editor");
    await page
      .getByRole("button", { name: "＋ Only for…", exact: true })
      .click();
    await page
      .getByRole("dialog")
      .getByRole("button", { name: "Codex", exact: true })
      .click();
    await expect(page.getByRole("dialog")).toContainText("Shows for: Codex");
    await capture("only-for-picker");
    await page.getByRole("button", { name: "Cancel", exact: true }).click();
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
      if (screen === "Review changes")
        await expect(
          page.getByRole("heading", {
            name: /Codex’s copy of recordings was edited/,
          }),
        ).toBeVisible();
      if (screen === "Agents & machines")
        await expect(
          page.getByRole("heading", { name: "Machine profiles", exact: true }),
        ).toBeVisible();
      if (screen === "Settings")
        await expect(page.getByLabel("Open Ratatoskr at login")).toBeVisible();
      await capture(file);
    }
    expect(errors).toEqual([]);
  });
}
