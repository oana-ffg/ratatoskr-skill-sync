import { SourceText } from "./source";
import { test, expect } from "vitest";
import { EditorState } from "@codemirror/state";
import { EditorView } from "@codemirror/view";
import { present, structuredDecorations } from "./decorations";
import { fixture, oracle } from "../../tests/fixture";
import type { Analysis } from "../api";
test("CodeMirror decorations never change saved bytes, commands preserve CRLF exactly", () => {
  const original = fixture.document.markdown.replaceAll("\n", "\r\n");
  const analysis = oracle("analyze_skill", { markdown: original }) as Analysis;
  const mapping = new SourceText(original);
  const view = new EditorView({
    state: EditorState.create({
      doc: mapping.normalized,
      extensions: [EditorState.lineSeparator.of("\n"), structuredDecorations],
    }),
    parent: document.body,
  });
  view.dispatch({
    effects: present.of({
      analysis,
      source: original,
      frontmatter: fixture.info.frontmatter,
      raw: false,
      onblock: () => {},
    }),
  });
  expect(
    view.state.sliceDoc(0, view.state.doc.length).replaceAll("\n", "\r\n"),
  ).toBe(original);
  expect(view.dom.querySelectorAll(".only-header")).toHaveLength(2);
  expect(view.dom.querySelector(".value-pill")).toHaveTextContent(
    "Recordings archive",
  );
  const from = original.indexOf("Transcribe");
  const to = from + "Transcribe the audio.".length;
  const result = oracle("edit_skill_draft", {
    markdown: original,
    metadata: fixture.document.metadata,
    edit: {
      kind: "only",
      from,
      to,
      selectors: { harness: ["codex"] },
      existing: false,
    },
  }) as { markdown: string; metadata: string };
  view.dispatch({
    changes: {
      from: 0,
      to: view.state.doc.length,
      insert: new SourceText(result.markdown).normalized,
    },
  });
  expect(
    view.state.sliceDoc(0, view.state.doc.length).replaceAll("\n", "\r\n"),
  ).toBe(
    original.slice(0, from) +
      "<!-- only harness=codex -->\r\n" +
      original.slice(from, to) +
      "\r\n<!-- /only -->" +
      original.slice(to),
  );
  expect(result.metadata).toBe(fixture.document.metadata);
  view.dispatch({
    effects: present.of({
      analysis,
      frontmatter: null,
      raw: true,
      onblock: () => {},
    }),
  });
  expect(
    view.state.sliceDoc(0, view.state.doc.length).replaceAll("\n", "\r\n"),
  ).toBe(result.markdown);
  view.destroy();
});
test("core edits conditions, variables and description without touching unrelated source", () => {
  const md = fixture.document.markdown;
  const block = fixture.analysis.blocks[0];
  const edited = oracle("edit_skill_draft", {
    markdown: md,
    metadata: fixture.document.metadata,
    edit: {
      kind: "only",
      from: block.opening.from,
      to: block.opening.to,
      selectors: { harness: ["claude"] },
      existing: true,
    },
  }) as { markdown: string };
  expect(edited.markdown).toBe(
    md.replace("<!-- only harness=codex -->", "<!-- only harness=claude -->"),
  );
  const variable = fixture.analysis.variables[0];
  const value = oracle("edit_skill_draft", {
    markdown: md,
    edit: {
      kind: "variable",
      ...variable.range,
      name: "paths.recordings_archive",
    },
  }) as { markdown: string };
  expect(value.markdown).toBe(md);
  const description = oracle("edit_skill_draft", {
    markdown: md,
    edit: { kind: "description", value: "A new description" },
  }) as { markdown: string };
  expect(description.markdown).toBe(
    md.replace(
      "description: Process meeting recordings into a searchable archive.",
      'description: "A new description"',
    ),
  );
});

test("mixed line endings and Unicode preserve source/editor range boundaries", () => {
  const original = "🐿 first\r\nsecond\nthird\r\nlast";
  const map = new SourceText(original);
  expect(map.normalized).toBe("🐿 first\nsecond\nthird\nlast");
  for (let p = 0; p <= map.normalized.length; p++)
    expect(map.toEditor(map.toSource(p))).toBe(p);
  const from = map.toSource(map.normalized.indexOf("second"));
  const to = map.toSource(map.normalized.indexOf("second") + 6);
  expect(original.slice(0, from) + "edited" + original.slice(to)).toBe(
    "🐿 first\r\nedited\nthird\r\nlast",
  );
});
