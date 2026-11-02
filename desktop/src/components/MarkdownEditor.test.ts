import { render, waitFor } from "@testing-library/svelte";
import { test, expect, vi } from "vitest";
import { EditorView, runScopeHandlers } from "@codemirror/view";
import { splitLine, undo } from "@codemirror/commands";
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

test("a command rewritten at a box edge keeps the caret it asked for", async () => {
  const open = "<!-- only harness=codex -->";
  const close = "<!-- /only -->";
  const value = `${open}\nInside.\n${close}\nAfter.\n`;
  const opening = { from: 0, to: open.length };
  const closing = {
    from: value.indexOf(close),
    to: value.indexOf(close) + close.length,
  };
  const { view } = mountSource(value, {
    blocks: [
      {
        range: { from: 0, to: closing.to },
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
    expect(view.dom.querySelector(".only-header")).not.toBeNull(),
  );
  const from = value.indexOf(".\n" + close);
  view.dispatch({
    selection: { anchor: from, head: value.indexOf("After.") + 1 },
  });
  // splitLine leaves the caret before the line break it inserts.
  splitLine(view);
  expect(view.state.selection.main.head).toBe(from);
  expect(view.state.field(sourceField).source).toBe(
    `${open}\nInside\n\n${close}\nfter.\n`,
  );
});
