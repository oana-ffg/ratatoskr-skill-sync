import { render, waitFor } from "@testing-library/svelte";
import { test, expect, vi } from "vitest";
import { EditorView, runScopeHandlers } from "@codemirror/view";
import {
  copyLineDown,
  copyLineUp,
  moveLineDown,
  moveLineUp,
  splitLine,
  transposeChars,
  undo,
} from "@codemirror/commands";
import MarkdownEditor from "./MarkdownEditor.svelte";
import { fixture, oracle } from "../../tests/fixture";
import { sourceField } from "../editor/source";
import type { Analysis, EditorView as Info } from "../api";
test("the actual editor preserves mixed line endings through typing, structured commands and undo", () => {
  const original = fixture.document.markdown
    .replace("## Steps\n", "## Steps\r\n")
    .replace("🐿", "🐿");
  const info = oracle("editor_view", { markdown: original }) as Info;
  const analysis = oracle("analyze_skill", { markdown: original }) as Analysis;
  const changed = vi.fn();
  const { container, component } = render(MarkdownEditor, {
    label: "Skill Markdown editor",
    value: original,
    analysis,
    frontmatter: info.frontmatter,
    onchange: changed,
    onselection: vi.fn(),
    onblock: vi.fn(),
  });
  const view = EditorView.findFromDOM(
    container.querySelector(".cm-editor") as HTMLElement,
  )!;
  const position = view.state.doc.toString().indexOf("Transcribe");
  view.dispatch({
    changes: {
      from: position,
      to: position + 10,
      insert: "Transcribe carefully",
    },
  });
  expect(changed).toHaveBeenLastCalledWith(
    original.replace("Transcribe", "Transcribe carefully"),
  );
  undo(view);
  expect(view.state.field(sourceField).source).toBe(original);
  const command = oracle("edit_skill_draft", {
    markdown: original,
    edit: { kind: "description", value: "Changed description" },
  }) as { markdown: string };
  component.replace(command.markdown);
  expect(changed).toHaveBeenLastCalledWith(command.markdown);
  undo(view);
  expect(view.state.field(sourceField).source).toBe(original);
});

test("realistic multi-paragraph skill hides whole frontmatter blocks and enables line numbers only in source mode", async () => {
  const document = oracle("realistic_document") as { markdown: string };
  const info = oracle("editor_view", {
    markdown: document.markdown,
    metadata: "",
  }) as Info;
  const analysis = oracle("analyze_skill", {
    markdown: document.markdown,
  }) as Analysis;
  const changed = vi.fn();
  const { container, rerender } = render(MarkdownEditor, {
    label: "Skill Markdown editor",
    value: document.markdown,
    analysis,
    frontmatter: info.frontmatter,
    onchange: changed,
    onselection: vi.fn(),
    onblock: vi.fn(),
    raw: false,
  });
  await waitFor(() =>
    expect(container.querySelector(".cm-content")).toHaveTextContent(
      "Inspect the application",
    ),
  );
  expect(container.querySelector(".cm-gutters")).toBeNull();
  expect(container.querySelector(".cm-content")).not.toHaveTextContent(
    "description:",
  );
  const view = EditorView.findFromDOM(
    container.querySelector(".cm-editor") as HTMLElement,
  )!;
  const { decorationField } = await import("../editor/decorations");
  let replacement: { from: number; to: number; block: boolean } | undefined;
  view.state
    .field(decorationField)
    .between(0, view.state.doc.length, (from, to, decoration) => {
      if (from === 0) replacement = { from, to, block: decoration.spec.block };
    });
  expect(replacement).toEqual({
    from: 0,
    to: info.frontmatter.to,
    block: true,
  });
  await rerender({ raw: true });
  await waitFor(() =>
    expect(container.querySelector(".cm-lineNumbers")).not.toBeNull(),
  );
  expect(container.querySelector(".cm-content")).toHaveTextContent(
    "description:",
  );
  await rerender({ raw: false });
  await waitFor(() =>
    expect(container.querySelector(".cm-gutters")).toBeNull(),
  );
  expect(view.state.field(sourceField).source).toBe(document.markdown);
  expect(changed).not.toHaveBeenCalled();
});

function typeBurst(view: EditorView, position: number, text: string) {
  for (const [index, char] of [...text].entries())
    view.dispatch({
      changes: { from: position + index, insert: char },
      selection: { anchor: position + index + 1 },
      userEvent: "input.type",
    });
}

function mountSource(
  value: string,
  analysis: Analysis = { blocks: [], variables: [], diagnostics: [] },
) {
  const changed = vi.fn();
  const rendered = render(MarkdownEditor, {
    label: "Editor",
    value,
    analysis,
    frontmatter: null,
    onchange: changed,
    onselection: vi.fn(),
    onblock: vi.fn(),
  });
  const view = EditorView.findFromDOM(
    rendered.container.querySelector(".cm-editor") as HTMLElement,
  )!;
  return { ...rendered, view, changed };
}

test("undoing a burst of typing restores the exact source, line endings included", () => {
  const original = "first\r\nsecond\nthird\r\n";
  const { view, changed } = mountSource(original);
  typeBurst(view, 5, "abc");
  expect(view.state.field(sourceField).source).toBe(
    "firstabc\r\nsecond\nthird\r\n",
  );
  undo(view);
  expect(view.state.doc.toString()).toBe("first\nsecond\nthird\n");
  expect(view.state.field(sourceField).source).toBe(original);
  expect(changed).toHaveBeenLastCalledWith(original);
  view.dispatch({ changes: { from: 0, insert: "!" }, userEvent: "input.type" });
  expect(view.state.field(sourceField).source).toBe(`!${original}`);
});

test("Shift+Mod+Z redoes on every platform", () => {
  const mac = /Mac/.test(navigator.platform);
  const { view } = mountSource("text");
  typeBurst(view, 4, "!");
  undo(view);
  expect(
    runScopeHandlers(
      view,
      new KeyboardEvent("keydown", {
        key: "Z",
        keyCode: 90,
        shiftKey: true,
        ctrlKey: !mac,
        metaKey: mac,
      }),
      "editor",
    ),
  ).toBe(true);
  expect(view.state.doc.toString()).toBe("text!");
});

test("a reload from outside ends the undo history instead of replaying it on the new text", async () => {
  const { view, rerender } = mountSource("Deleted at the end.");
  view.dispatch({
    changes: { from: 18, to: 19 },
    userEvent: "delete.backward",
  });
  await rerender({ value: "A different file." });
  await waitFor(() =>
    expect(view.state.field(sourceField).source).toBe("A different file."),
  );
  undo(view);
  expect(view.state.doc.toString()).toBe("A different file.");
  expect(view.state.field(sourceField).source).toBe("A different file.");
});

const OPEN = "<!-- only harness=codex -->";
const CLOSE = "<!-- /only -->";

/** Mounts `value`, which holds one box, with the box drawn. */
async function mountBox(value: string) {
  const opening = {
    from: value.indexOf(OPEN),
    to: value.indexOf(OPEN) + OPEN.length,
  };
  const closing = {
    from: value.indexOf(CLOSE),
    to: value.indexOf(CLOSE) + CLOSE.length,
  };
  const mounted = mountSource(value, {
    blocks: [
      {
        range: { from: opening.from, to: closing.to },
        opening,
        body: { from: opening.to + 1, to: closing.from - 1 },
        closing,
        selectors: { harness: ["codex"] },
        active: true,
        reachable: true,
      },
    ],
    variables: [],
    diagnostics: [],
  });
  await waitFor(() =>
    expect(mounted.view.dom.querySelector(".only-header")).not.toBeNull(),
  );
  return mounted;
}

test("a command rewritten at a box edge keeps the caret it asked for", async () => {
  const value = `${OPEN}\nInside.\n${CLOSE}\nAfter.\n`;
  const { view } = await mountBox(value);
  const from = value.indexOf(".\n" + CLOSE);
  view.dispatch({
    selection: { anchor: from, head: value.indexOf("After.") + 1 },
  });
  // splitLine leaves the caret before the line break it inserts.
  splitLine(view);
  expect(view.state.selection.main.head).toBe(from);
  expect(view.state.field(sourceField).source).toBe(
    `${OPEN}\nInside\n\n${CLOSE}\nfter.\n`,
  );
});

test("transpose stops at a box edge and still works inside a line", async () => {
  const value = `Before.\n${OPEN}\nFirst.\nLast.\n${CLOSE}\nAfter.\n`;
  const { view } = await mountBox(value);
  const source = () => view.state.field(sourceField).source;
  for (const position of [
    value.indexOf("First."),
    value.indexOf("Last.") + 5,
    value.indexOf("After."),
    value.indexOf("Before.") + 7,
  ]) {
    view.dispatch({ selection: { anchor: position } });
    transposeChars(view);
    expect(source()).toBe(value);
  }
  view.dispatch({ selection: { anchor: value.indexOf("First.") + 1 } });
  transposeChars(view);
  expect(source()).toBe(value.replace("First.", "iFrst."));
});

test("moving lines stops at a box edge and works everywhere else", async () => {
  const value = `Above.\nBefore.\n${OPEN}\nFirst.\nLast.\n${CLOSE}\nAfter.\nBelow.\n`;
  const refused = [
    [moveLineDown, "Last."],
    [moveLineUp, "First."],
    [moveLineDown, "Before."],
    [moveLineUp, "After."],
  ] as const;
  for (const [move, line] of refused) {
    const { view, unmount } = await mountBox(value);
    view.dispatch({ selection: { anchor: value.indexOf(line) } });
    move(view);
    expect(view.state.field(sourceField).source).toBe(value);
    unmount();
  }
  // Selections holding a marker stop too: moving the opening one past the only line would empty it.
  const only = `Before.\n${OPEN}\nOnly.\n${CLOSE}\nAfter.\n`;
  const { view, unmount } = await mountBox(only);
  view.dispatch({ selection: { anchor: 0, head: only.indexOf("Only.") } });
  moveLineDown(view);
  expect(view.state.field(sourceField).source).toBe(only);
  unmount();
  const allowed = [
    [moveLineDown, "First.", value.replace("First.\nLast.", "Last.\nFirst.")],
    [moveLineUp, "Last.", value.replace("First.\nLast.", "Last.\nFirst.")],
    [
      moveLineDown,
      "Above.",
      value.replace("Above.\nBefore.", "Before.\nAbove."),
    ],
    [moveLineUp, "Below.", value.replace("After.\nBelow.", "Below.\nAfter.")],
  ] as const;
  for (const [move, line, expected] of allowed) {
    const { view, unmount } = await mountBox(value);
    view.dispatch({ selection: { anchor: value.indexOf(line) } });
    move(view);
    expect(view.state.field(sourceField).source).toBe(expected);
    unmount();
  }
});

test("copying lines refuses one marker of a box without the other", async () => {
  const value = `Before.\n${OPEN}\nFirst.\n${CLOSE}\nAfter.\n`;
  const allowed = [
    [
      "First.",
      copyLineDown,
      `Before.\n${OPEN}\nFirst.\nFirst.\n${CLOSE}\nAfter.\n`,
    ],
    [
      "Before.",
      copyLineDown,
      `Before.\nBefore.\n${OPEN}\nFirst.\n${CLOSE}\nAfter.\n`,
    ],
    [
      "After.",
      copyLineUp,
      `Before.\n${OPEN}\nFirst.\n${CLOSE}\nAfter.\nAfter.\n`,
    ],
  ] as const;
  for (const [line, copy, expected] of allowed) {
    const { view, unmount } = await mountBox(value);
    view.dispatch({ selection: { anchor: value.indexOf(line) } });
    copy(view);
    expect(view.state.field(sourceField).source).toBe(expected);
    unmount();
  }
  for (const copy of [copyLineDown, copyLineUp]) {
    const { view, unmount } = await mountBox(value);
    view.dispatch({
      selection: { anchor: 0, head: value.indexOf("First.") + 2 },
    });
    copy(view);
    expect(view.state.field(sourceField).source).toBe(value);
    unmount();
  }
  // The whole box may be copied.
  const { view, unmount } = await mountBox(value);
  view.dispatch({
    selection: { anchor: 0, head: value.indexOf("After.") + 2 },
  });
  copyLineDown(view);
  expect(view.state.field(sourceField).source).toBe(
    `Before.\n${OPEN}\nFirst.\n${CLOSE}\nAfter.\n`.repeat(2),
  );
  unmount();
});
