import { render, waitFor } from "@testing-library/svelte";
import { test, expect, vi } from "vitest";
import { EditorView } from "@codemirror/view";
import { undo } from "@codemirror/commands";
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
