<script lang="ts">
  import {
    SourceText,
    minimalChange,
    sourceField,
    sourceExtension,
    setSource,
  } from "../editor/source";
  import { onMount } from "svelte";
  import {
    Compartment,
    EditorSelection,
    EditorState,
    Transaction,
  } from "@codemirror/state";
  import {
    EditorView,
    keymap,
    lineNumbers,
    drawSelection,
    highlightActiveLine,
  } from "@codemirror/view";
  import {
    defaultKeymap,
    history,
    historyKeymap,
    indentWithTab,
    redo,
  } from "@codemirror/commands";
  import { search, searchKeymap, openSearchPanel } from "@codemirror/search";
  import { markdown } from "@codemirror/lang-markdown";
  import { syntaxHighlighting, HighlightStyle } from "@codemirror/language";
  import { tags } from "@lezer/highlight";
  import { structuredDecorations } from "../editor/decorations";
  import { present } from "../editor/presentation";
  import { type Analysis, type Block, type Range } from "../api";
  let {
    label,
    value,
    analysis,
    frontmatter,
    raw = false,
    onchange,
    onselection,
    onblock,
  }: {
    label: string;
    value: string;
    analysis: Analysis;
    frontmatter: Range | null;
    raw?: boolean;
    onchange: (text: string) => void;
    onselection: (range: Range) => void;
    onblock: (block: Block, remove: boolean) => void;
  } = $props();
  const gutter = new Compartment();
  const undoHistory = new Compartment();
  let host: HTMLDivElement;
  let view = $state<EditorView>();

  export function replace(text: string) {
    if (!view) return;
    const change = minimalChange(
      view.state.doc.toString(),
      new SourceText(text).normalized,
    );
    const { main } = view.state.selection;
    view.dispatch({
      changes: change,
      // A selection the edit rewrote collapses instead of selecting the new markup, so the next
      // keystroke cannot replace it.
      selection:
        !main.empty && main.from >= change.from && main.to <= change.to
          ? EditorSelection.cursor(change.from)
          : undefined,
      effects: setSource.of(text),
      userEvent: "input",
    });
  }
  export function find() {
    if (view) openSearchPanel(view);
  }
  export function format(kind: string) {
    if (!view) return;
    const s = view.state.selection.main;
    const chosen = view.state.sliceDoc(s.from, s.to);
    const pair =
      kind === "bold"
        ? ["**", "**"]
        : kind === "code"
          ? ["`", "`"]
          : kind === "heading"
            ? ["## ", ""]
            : ["- ", ""];
    view.dispatch({
      changes: { from: s.from, to: s.to, insert: pair[0] + chosen + pair[1] },
      userEvent: "input",
    });
    view.focus();
  }
  onMount(() => {
    // Explicit line separator preserves CRLF and final newlines in the actual document.
    const sourceText = new SourceText(value);
    view = new EditorView({
      parent: host,
      state: EditorState.create({
        doc: sourceText.normalized,
        selection: {
          anchor: raw ? 0 : sourceText.toEditor(frontmatter?.to ?? 0),
        },
        extensions: [
          EditorState.lineSeparator.of("\n"),
          sourceExtension(value),
          gutter.of(raw ? lineNumbers() : []),
          drawSelection(),
          highlightActiveLine(),
          undoHistory.of(history()),
          search({ top: true }),
          keymap.of([
            ...defaultKeymap,
            ...historyKeymap,
            // CodeMirror binds this only on macOS and Linux; on Windows it would fall through to
            // the webview's own redo, which edits the DOM behind CodeMirror's history.
            { key: "Mod-Shift-z", run: redo, preventDefault: true },
            ...searchKeymap,
            indentWithTab,
          ]),
          // Keep the caret clear of the toolbar that stays pinned above the editor.
          // Keep the caret clear of the toolbar that stays pinned above the editor.
          EditorView.scrollMargins.of((view) => ({
            top:
              parseFloat(
                getComputedStyle(view.dom).getPropertyValue(
                  "--format-toolbar-height",
                ),
              ) || 0,
          })),
          markdown(),
          syntaxHighlighting(
            HighlightStyle.define([
              { tag: tags.heading, color: "var(--accent)", fontWeight: "700" },
              { tag: tags.strong, color: "var(--text)", fontWeight: "700" },
              { tag: tags.emphasis, fontStyle: "italic" },
              { tag: tags.monospace, color: "var(--purple)" },
              { tag: tags.link, color: "var(--blue)" },
              { tag: tags.meta, color: "var(--muted)" },
            ]),
          ),
          EditorView.lineWrapping,
          structuredDecorations,
          EditorView.contentAttributes.of({
            "aria-label": label,
          }),
          EditorView.updateListener.of((update) => {
            const sourceText = update.state.field(sourceField);
            if (
              update.docChanged &&
              !update.transactions.every((t) =>
                t.annotation(Transaction.remote),
              )
            )
              onchange(sourceText.source);
            if (update.selectionSet) {
              const range = update.state.selection.main;
              onselection({
                from: sourceText.toSource(range.from),
                to: sourceText.toSource(range.to),
              });
            }
          }),
        ],
      }),
    });
    updatePresentation();
    return () => {
      view?.destroy();
      view = undefined;
    };
  });
  function updatePresentation() {
    view?.dispatch({
      effects: [
        gutter.reconfigure(raw ? lineNumbers() : []),
        present.of({
          analysis,
          frontmatter,
          raw,
          onblock,
          source: view.state.field(sourceField).source,
        }),
      ],
    });
  }
  $effect(() => {
    if (view && value !== view.state.field(sourceField).source) {
      view.dispatch({
        changes: minimalChange(
          view.state.doc.toString(),
          new SourceText(value).normalized,
        ),
        // Edits to the replaced text cannot be undone on this one: mapped through this change they
        // would apply at the wrong place.
        effects: [setSource.of(value), undoHistory.reconfigure([])],
        annotations: [
          Transaction.remote.of(true),
          Transaction.addToHistory.of(false),
        ],
      });
      view.dispatch({ effects: undoHistory.reconfigure(history()) });
    }
  });
  $effect(() => updatePresentation());
</script>

<div class="markdown-editor" bind:this={host}></div>
