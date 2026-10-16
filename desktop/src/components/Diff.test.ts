import { render, screen, fireEvent } from "@testing-library/svelte";
import { test, expect } from "vitest";
import Diff from "./Diff.svelte";
import { diffRows } from "./diff";
import { createPatch } from "diff";

test("neutral conflicts put rendered source on the left and highlight only changed words", () => {
  const { container } = render(Diff, {
    diff: createPatch(
      "skill",
      "Check the complete recording.\n",
      "Check the original recording.\n",
    ),
    reverse: true,
    neutral: true,
    left: "Your source",
    right: "Edited copy",
  });
  const panes = container.querySelectorAll(".diff-grid > div");
  expect(panes[0]).toHaveTextContent(
    "Your source Check the original recording.",
  );
  expect(panes[1]).toHaveTextContent(
    "Edited copy Check the complete recording.",
  );
  expect(
    Array.from(container.querySelectorAll(".word-change")).map(
      (span) => span.textContent,
    ),
  ).toEqual(["original", "complete"]);
  expect(container.querySelector(".diff-add, .diff-remove")).toBeNull();
});

test("context is readable, long runs collapse and expand, and hunk gaps are explicit", async () => {
  const before =
    Array.from({ length: 20 }, (_, i) => `Context ${i}`).join("\n") +
    "\nOld line\n";
  const after = before.replace("Old line", "New line");
  const { container } = render(Diff, {
    diff: createPatch("skill", before, after, "", "", { context: 25 }),
  });
  expect(container.querySelector(".diff-context")).toHaveTextContent(
    "Context 0",
  );
  expect(screen.getAllByText(/14 unchanged lines/)).toHaveLength(2);
  await fireEvent.click(
    screen.getByRole("button", { name: "Expand available context" }),
  );
  expect(screen.getAllByText("Context 10")).toHaveLength(2);
  expect(diffRows(createPatch("skill", before, after))[0].omitted).toBe(16);
});

test("additions, deletions, blank context and missing final newlines retain content", () => {
  for (const [before, after] of [
    ["", "New"],
    ["Old", ""],
    ["Heading\n\nOld", "Heading\n\nNew"],
  ]) {
    const rows = diffRows(createPatch("skill", before, after));
    const text = (side: "left" | "right") =>
      rows
        .filter((row) => row[side].kind !== "missing")
        .map((row) =>
          row[side].segments.map((segment) => segment.text).join(""),
        )
        .join("\n");
    expect(text("left")).toBe(before);
    expect(text("right")).toBe(after);
  }
});
