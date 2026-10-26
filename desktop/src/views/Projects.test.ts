import { render, screen, fireEvent, within } from "@testing-library/svelte";
import { test, expect, vi } from "vitest";
import Projects from "./Projects.svelte";
import { projectWorkspace } from "../../tests/fixture";

const callbacks = () => ({
  onupdate: vi.fn(),
  onerror: vi.fn(),
  onbrowse: vi.fn(),
  ondirty: vi.fn(),
});

test.each([0, 1, 2])(
  "shows %i project skills independently of Git change counts",
  async (count) => {
    const workspace = projectWorkspace();
    const project = workspace.plan.projects[0];
    workspace.skills = workspace.skills
      .filter((s) => s.project === project.root)
      .slice(0, count);
    // Include an unrelated global skill with the same name to catch over-counts.
    workspace.skills.push({ name: "mix-audio", enabled: true, project: null });
    const props = callbacks();
    render(Projects, { workspace, ...props });
    const card = within(
      screen.getByRole("heading", { name: "audio-tools" }).closest("article")!,
    );
    expect(
      card.getByText(`${count} ${count === 1 ? "skill" : "skills"}`, {
        exact: true,
      }),
    ).toBeVisible();
    expect(card.getByText("No uncommitted or unpushed changes")).toBeVisible();
    expect(card.queryByText(/0 uncommitted/)).toBeNull();
    await fireEvent.click(card.getByRole("button", { name: "View skills →" }));
    expect(props.onbrowse).toHaveBeenCalledWith(project.root);
  },
);

test("keeps unpublished changes distinct from the skill total", () => {
  const workspace = projectWorkspace();
  workspace.plan.projects[0].changed_files = 3;
  workspace.plan.projects[0].outgoing_commits = null;
  render(Projects, { workspace, ...callbacks() });
  expect(screen.getByText("2 skills", { exact: true })).toBeVisible();
  expect(
    screen.getByText(/3 uncommitted skill files · Unknown unpushed commits/),
  ).toBeVisible();
});

test("reports an unavailable list instead of claiming zero skills", () => {
  const workspace = projectWorkspace();
  workspace.skills = [];
  workspace.skills_error =
    "Cannot list source skills: source folder is unreadable";
  render(Projects, { workspace, ...callbacks() });
  expect(screen.getAllByText("Skill count unavailable")).toHaveLength(2);
  expect(screen.queryByText("0 skills", { exact: true })).toBeNull();
  expect(screen.getAllByRole("alert")[0]).toHaveTextContent(
    workspace.skills_error,
  );
});
