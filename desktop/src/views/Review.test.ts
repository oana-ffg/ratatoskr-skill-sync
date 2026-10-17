import {
  render,
  screen,
  fireEvent,
  waitFor,
  within,
} from "@testing-library/svelte";
import { test, expect, vi, beforeEach } from "vitest";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import Review from "./Review.svelte";
import { fixture } from "../../tests/fixture";
import { reviewSummary, type Snapshot } from "../api";
beforeEach(() => clearMocks());
const props = (workspace: Snapshot) => ({
  workspace,
  onupdate: vi.fn(),
  onerror: vi.fn(),
  ondirty: vi.fn(),
});
test("attention badge groups eight files per skill and agent, and counts legacy decisions once", async () => {
  const summary = reviewSummary(fixture.workspace);
  expect(
    summary.groups
      .filter((g) => g.name === "aws-project-migration")
      .map((g) => g.changes.length),
  ).toEqual([8, 8]);
  expect(summary.total).toBe(5);
  expect(summary.decisions).toBe(2);
  render(Review, props(fixture.workspace));
  const heading = screen.getByRole("heading", {
    name: "aws-project-migration → Claude Code: 8 files",
  });
  const group = heading.closest("article")!;
  expect(within(group).queryByText("SKILL.md · update")).toBeInTheDocument();
  expect(group.querySelector(".group-files")).not.toHaveAttribute("open");
  expect(
    screen.getByRole("button", { name: summary.applyAction }),
  ).toBeDisabled();
  expect(
    screen.getByText(/Your real agent folders are not touched/),
  ).toHaveTextContent(fixture.workspace.plan.staging_root!);
  expect(
    screen.getByRole("region", { name: "Older Codex copies" }),
  ).toHaveTextContent("/fixture-home/.agents/skills");
});
test.each([false, true])(
  "ready count and Apply agree with conflicts and removals=%s",
  (withRemoval) => {
    const workspace = structuredClone(fixture.workspace);
    if (withRemoval) workspace.plan.changes[0].action = "remove";
    const summary = reviewSummary(workspace);
    expect(summary.readyCount).toBe(17);
    render(Review, props(workspace));
    expect(screen.getByText(/items needing attention ·/)).toHaveTextContent(
      `${summary.readyCount} ready files`,
    );
    const button = screen.getByRole("button", {
      name: /in the staging folder/,
    });
    const counts = [...button.textContent!.matchAll(/(\d+) files?/g)].map((m) =>
      Number(m[1]),
    );
    expect(counts.reduce((total, count) => total + count, 0)).toBe(
      summary.readyCount,
    );
    expect(button).toHaveTextContent(
      withRemoval
        ? "Write 16 files and remove 1 file in the staging folder"
        : "Write 17 files in the staging folder",
    );
  },
);
test("keep both submits only the explicitly reviewed duplicate and surfaces failures", async () => {
  const calls = vi.fn(() => {
    throw new Error("Legacy skill changed since preview; review it again");
  });
  mockIPC(calls);
  const p = props(fixture.workspace);
  render(Review, p);
  expect(calls).not.toHaveBeenCalled();
  await fireEvent.click(
    screen.getByRole("button", { name: "Keep both copies" }),
  );
  await waitFor(() => expect(p.onerror).toHaveBeenCalled());
  expect(calls).toHaveBeenCalledWith("keep_legacy", {
    expected: JSON.stringify(fixture.workspace.legacy[0]),
  });
  expect(p.onupdate).not.toHaveBeenCalled();
});
test("custom Windows roots and project scope stay separate; a live apply names real targets", () => {
  const workspace = structuredClone(fixture.workspace);
  workspace.plan.mode = "live";
  workspace.plan.staging_root = null;
  workspace.plan.conflicts = [];
  workspace.plan.conflict_details = [];
  workspace.legacy = [];
  workspace.plan.install_targets = [null, "C:\\work\\project"].map(
    (project, i) => ({
      harness: "codex",
      project,
      skills_root: i
        ? "C:\\work\\project\\custom-skills"
        : "C:\\agent-data\\custom-skills",
      instructions_file: i
        ? "C:\\work\\project\\AGENTS.md"
        : "C:\\agent-data\\AGENTS.md",
    }),
  );
  const original = workspace.plan.changes[0];
  workspace.plan.changes = workspace.plan.install_targets.flatMap((t) =>
    ["SKILL.md", "references/check.md"].map((file) => ({
      ...original,
      path: `${t.skills_root}\\aws-project-migration\\${file}`,
    })),
  );
  const summary = reviewSummary(workspace);
  expect(summary.total).toBe(2);
  expect(summary.groups.map((g) => g.changes.length)).toEqual([2, 2]);
  render(Review, props(workspace));
  expect(
    screen.getByRole("button", {
      name: "Write 4 files in the real agent folders",
    }),
  ).toBeEnabled();
  expect(screen.getByText("C:\\agent-data\\custom-skills")).toBeVisible();
  expect(screen.getAllByText("C:\\work\\project")).toHaveLength(1);
});
