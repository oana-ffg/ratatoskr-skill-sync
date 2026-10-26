import { test, expect } from "@playwright/test";
import { fixtureIPC, projectWorkspace } from "../tests/fixture";
import { installFixture } from "./helpers";

test("project cards show skill totals and View skills opens the matching count", async ({
  page,
}, testInfo) => {
  const workspace = projectWorkspace();
  await installFixture(page, (cmd, args) =>
    cmd === "read_workspace" ? workspace : fixtureIPC(cmd, args),
  );
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: /Projects/ })
    .click();
  const card = page
    .getByRole("article")
    .filter({ has: page.getByRole("heading", { name: "audio-tools" }) });
  await expect(card.getByText("2 skills", { exact: true })).toBeVisible();
  await expect(
    card.getByText("No uncommitted or unpushed changes"),
  ).toBeVisible();
  const empty = page
    .getByRole("article")
    .filter({ has: page.getByRole("heading", { name: "website" }) });
  await expect(empty.getByText("0 skills", { exact: true })).toBeVisible();
  await page.screenshot({ path: testInfo.outputPath("project-counts.png") });
  await card.getByRole("button", { name: "View skills →" }).click();
  await expect(
    page.getByRole("heading", { name: "Skills 2", exact: true }),
  ).toBeVisible();
  await expect(page.getByLabel("Skill scope")).toHaveValue(
    workspace.plan.projects[0].root,
  );
  await expect(page.locator(".skill-row")).toHaveCount(2);
  await expect(
    page.getByRole("button", { name: /archive-audio Disabled/ }),
  ).toBeVisible();
  await page.getByLabel("Search skills").fill("mix");
  await expect(
    page.getByRole("heading", { name: "Skills 1", exact: true }),
  ).toBeVisible();
});

for (const projectCount of [0, 2]) {
  test(`a failed skill read never masquerades as an empty library (${projectCount} projects)`, async ({
    page,
  }) => {
    const workspace = projectWorkspace();
    workspace.plan.projects = workspace.plan.projects.slice(0, projectCount);
    workspace.skills = [];
    workspace.skills_error =
      "Cannot list source skills: source folder is unreadable";
    await installFixture(page, (cmd, args) =>
      cmd === "read_workspace" ? workspace : fixtureIPC(cmd, args),
    );
    await page.goto("/");
    await expect(
      page.getByRole("heading", { name: "Skills", exact: true }),
    ).toBeVisible();
    await expect(page.getByRole("alert")).toContainText(workspace.skills_error);
    await expect(page.getByText("No skills yet", { exact: true })).toHaveCount(
      0,
    );
    await expect(
      page.getByText(
        "Add a skill to share it across your agents and machines.",
      ),
    ).toHaveCount(0);
    await page
      .getByRole("navigation")
      .getByRole("button", { name: /Projects/ })
      .click();
    await expect(page.getByText("Skill count unavailable")).toHaveCount(
      projectCount,
    );
    await expect(page.getByText("0 skills", { exact: true })).toHaveCount(0);
    await expect(page.getByRole("alert")).toContainText(workspace.skills_error);
  });
}
