import { test, expect } from "vitest";
import { diffRows } from "../components/diff";
import { mergeThreeWay, splitLines, unifiedDiff } from "./merge";

test("splitLines preserves exact line terminators and round-trips", () => {
  for (const sample of [
    "",
    "single line",
    "a\nb\n",
    "a\r\nb\r\n",
    "a\r\nb\nc",
    "\n\n",
  ]) {
    expect(splitLines(sample).join("")).toBe(sample);
  }
});

test("mergeThreeWay combines disjoint edits including adjacent update + insertion", () => {
  const base = [
    "# Rules",
    "- Old quota line",
    "<!-- /only -->",
    "## Do it for me",
    "",
  ].join("\n");
  const theirs = [
    "# Rules",
    "- Updated quota line",
    "<!-- /only -->",
    "## Do it for me",
    "",
  ].join("\n");
  const ours = [
    "# Rules",
    "- Old quota line",
    "- New subagent notes bullet",
    "<!-- /only -->",
    "## Do it for me",
    "",
  ].join("\n");

  expect(mergeThreeWay(base, theirs, ours)).toEqual({
    ok: true,
    text: [
      "# Rules",
      "- Updated quota line",
      "- New subagent notes bullet",
      "<!-- /only -->",
      "## Do it for me",
      "",
    ].join("\n"),
  });
});

test("mergeThreeWay preserves CRLF line endings", () => {
  const base = "line 1\r\nline 2\r\nline 3\r\n";
  const theirs = "line 1 updated\r\nline 2\r\nline 3\r\n";
  const ours = "line 1\r\nline 2\r\nline 3 updated\r\n";

  expect(mergeThreeWay(base, theirs, ours)).toEqual({
    ok: true,
    text: "line 1 updated\r\nline 2\r\nline 3 updated\r\n",
  });
});

test("mergeThreeWay accepts identical changes on both sides", () => {
  const base = "a\nb\nc\n";
  const changed = "a\nb-edited\nc\n";
  expect(mergeThreeWay(base, changed, changed)).toEqual({
    ok: true,
    text: changed,
  });
});

test("mergeThreeWay refuses conflicting edits on the same line", () => {
  const base = "a\nb\nc\n";
  const theirs = "a\nb-theirs\nc\n";
  const ours = "a\nb-ours\nc\n";
  expect(mergeThreeWay(base, theirs, ours)).toEqual({ ok: false });
});

test("mergeThreeWay refuses different insertions at the same position", () => {
  const base = "a\nc\n";
  const theirs = "a\nb-theirs\nc\n";
  const ours = "a\nb-ours\nc\n";
  expect(mergeThreeWay(base, theirs, ours)).toEqual({ ok: false });
});

test("mergeThreeWay refuses an insertion inside a deleted span", () => {
  const base = "a\nb\nc\nd\n";
  const theirs = "a\nd\n";
  const ours = "a\nb\ninserted\nc\nd\n";
  expect(mergeThreeWay(base, theirs, ours)).toEqual({ ok: false });
});

test("unifiedDiff produces patches understood by diffRows", () => {
  expect(unifiedDiff("same\n", "same\n")).toBe("");
  const patch = unifiedDiff("one\ntwo\n", "one\nthree\n");
  const rows = diffRows(patch);
  expect(rows.some((r) => r.left.kind === "remove")).toBe(true);
  expect(rows.some((r) => r.right.kind === "add")).toBe(true);
});
