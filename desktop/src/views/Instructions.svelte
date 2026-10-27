<script lang="ts">
  import { onMount } from "svelte";
  import {
    api,
    errorText,
    emptyAnalysis,
    type Snapshot,
    type SourceDocument,
    type Analysis,
    type Block,
    type Range,
    type ValueOption,
  } from "../api";
  import { createLivePreview } from "../live-preview";
  import MarkdownEditor from "../components/MarkdownEditor.svelte";
  import OnlyPicker from "../components/OnlyPicker.svelte";
  import ValuePicker from "../components/ValuePicker.svelte";
  import TemplateToolbar from "../components/TemplateToolbar.svelte";
  import PreviewTargets from "../components/PreviewTargets.svelte";
  let {
    workspace,
    onupdate,
    onerror,
    ondirty,
    onunsaved,
  }: {
    workspace: Snapshot;
    onupdate: (w: Snapshot) => void;
    onerror: (e: unknown) => void;
    ondirty: (v: boolean) => void;
    onunsaved?: (v: boolean) => void;
  } = $props();
  let document = $state<SourceDocument | null>(null);
  let markdown = $state("");
  let analysis = $state<Analysis>(emptyAnalysis);
  let rendered = $state("");
  let changedLines = $state<number[]>([]);
  let values = $state<ValueOption[]>([]);
  let destination = $state("");
  let previewError = $state("");
  let validating = $state(false);
  let busy = $state(false);
  let machine = $state("");
  let harness = $state("");
  let raw = $state(false);
  let selected = $state<Range>({ from: 0, to: 0 });
  let picker = $state<"only" | "value" | null>(null);
  let block = $state<Block | null>(null);
  let editor = $state<MarkdownEditor>();
  let editQueue = Promise.resolve();
  let pendingEdits = $state(0);
  const agents = $derived([
    ...new Set(workspace.preview_targets.map((t) => t.harness)),
  ]);
  const machines = $derived([
    ...new Set(workspace.preview_targets.map((t) => t.machine)),
  ]);
  const dirty = $derived(!!document && markdown !== document.text);
  const hasErrors = $derived(
    analysis.diagnostics.some((d) => d.severity === "error"),
  );
  $effect(() => onunsaved?.(dirty));
  $effect(() => ondirty(dirty || busy || pendingEdits > 0 || picker !== null));
  const validation = createLivePreview(
    async (input: { markdown: string; machine: string; harness: string }) => {
      const a = await api.analyzeInstructions(
        input.markdown,
        input.machine,
        input.harness,
      );
      const options = await api.values(input.machine, input.harness);
      const path = await api.instructionsPath(input.machine, input.harness);
      let text = "";
      let changed: number[] = [];
      let error = "";
      if (!a.diagnostics.some((d) => d.severity === "error")) {
        try {
          text = await api.previewInstructions(
            input.markdown,
            input.machine,
            input.harness,
          );
          const alternate = await Promise.all(
            workspace.preview_targets
              .filter(
                (t) =>
                  t.machine === input.machine && t.harness !== input.harness,
              )
              .map((t) =>
                api.previewInstructions(input.markdown, t.machine, t.harness),
              ),
          );
          changed = await api.command<number[]>("different_preview_lines", {
            text,
            versions: alternate,
          });
        } catch (e) {
          error = errorText(e);
        }
      }
      return { a, options, path, text, changed, error };
    },
    (result) => {
      analysis = result.a;
      values = result.options;
      destination = result.path;
      rendered = result.text;
      changedLines = result.changed;
      previewError = result.error;
      validating = false;
    },
    (e) => {
      previewError = errorText(e);
      rendered = "";
      changedLines = [];
      validating = false;
    },
    150,
  );
  $effect(() => {
    if (document && harness) {
      validating = true;
      previewError = "";
      validation.schedule({ markdown, machine, harness });
    }
  });
  onMount(() => {
    machine = workspace.plan.machine;
    harness =
      workspace.preview_targets.find((t) => t.machine === machine)?.harness ??
      "";
    void load();
    return () => validation.cancel();
  });
  async function load() {
    try {
      const d = await api.readInstructions();
      document = d;
      markdown = d.text;
    } catch (e) {
      onerror(e);
    }
  }
  function edit(operation: Record<string, unknown>): Promise<void> {
    pendingEdits++;
    const next = editQueue
      .then(async () => {
        const original = markdown;
        // Instructions have no skill metadata; only Markdown operations apply.
        const result = await api.edit(original, "", operation);
        if (markdown !== original)
          throw new Error("The draft changed during the edit. Try again.");
        if (result.markdown !== markdown) {
          editor?.replace(result.markdown);
          markdown = result.markdown;
        }
      })
      .catch(onerror)
      .finally(() => pendingEdits--);
    editQueue = next;
    return next;
  }
  export async function save() {
    if (!document || busy) return;
    await editQueue;
    // The core validates this exact draft on every machine × agent before writing.
    busy = true;
    try {
      onupdate(await api.saveInstructions(markdown, document.hash));
      await load();
    } finally {
      busy = false;
    }
  }
  export async function discard() {
    await editQueue;
    picker = null;
    await load();
  }
  function openOnly(b: Block | null = null) {
    block = b;
    picker = "only";
  }
</script>

{#if document}
  <div class="editor-layout">
    <section class="editor-source">
      <div class="page-heading compact">
        <div>
          <h1>Global instructions</h1>
          <p class="muted">
            Shared instructions for every agent, rendered into each agent’s own
            global file, such as CLAUDE.md or AGENTS.md.
          </p>
        </div>
        <div class="heading-actions">
          <span
            class="status-pill {hasErrors || previewError
              ? 'warning'
              : 'success'}"
            >{validating
              ? "Checking…"
              : previewError
                ? "Needs attention"
                : hasErrors
                  ? "Fix errors"
                  : "✓ Valid"}</span
          >
        </div>
      </div>
      {#if !document.text}<p class="muted small">
          Your content repository has no global/AGENTS.md yet. Saving creates
          it. If an agent already has its own instructions file on a machine,
          Review changes asks you to resolve it before anything is written.
        </p>{/if}
      <div class="editor-frame">
        <TemplateToolbar
          {editor}
          bind:raw
          ononly={() => openOnly()}
          onvalue={() => (picker = "value")}
        />
        <MarkdownEditor
          bind:this={editor}
          label="Global instructions editor"
          value={markdown}
          {analysis}
          frontmatter={null}
          {raw}
          onchange={(text) => (markdown = text)}
          onselection={(range) =>
            (selected = { from: range.from, to: range.to })}
          onblock={(b, remove) => {
            if (remove)
              void edit({ kind: "remove-only", from: b.opening.from });
            else openOnly(b);
          }}
        />
      </div>
      {#if previewError}<p role="alert" class="error-message">
          {previewError}
        </p>{/if}
    </section>
    <aside class="live-preview">
      <p class="eyebrow">LIVE PREVIEW <span class="live-dot"></span></p>
      <PreviewTargets
        targets={workspace.preview_targets}
        bind:machine
        bind:harness
      />
      <div class="preview-document" aria-label="Rendered preview">
        {#if !harness}<p class="muted">
            Enable an agent in a machine profile to preview its instructions.
          </p>{:else if validating}<span class="muted">Updating preview…</span
          >{:else if previewError || hasErrors}<p class="muted">
            Preview will return when the errors are fixed.
          </p>{:else if !rendered.trim()}<p class="muted">
            This agent and machine receive an empty instructions file.
          </p>{:else}{#each rendered.split("\n") as line, index}<div
              class:changed-line={!!line.trim() && changedLines.includes(index)}
            >
              {line || " "}
            </div>{/each}{/if}
      </div>
      {#if harness && destination}<p class="eyebrow">DESTINATION</p>
        <p class="path">{destination}</p>{/if}
      <p class="muted small">
        Highlighted lines differ from the other agents’ versions.
      </p>
    </aside>
  </div>
{:else}<div class="empty-state">Opening global instructions…</div>{/if}
{#if picker === "only"}<OnlyPicker
    {agents}
    {machines}
    initial={block?.selectors ?? {}}
    onclose={() => (picker = null)}
    onapply={(selectors) => {
      const range = block?.range ?? selected;
      void edit({
        kind: "only",
        from: range.from,
        to: range.to,
        selectors,
        existing: !!block,
      });
      picker = null;
    }}
  />{/if}
{#if picker === "value"}<ValuePicker
    {values}
    {machine}
    {harness}
    onclose={() => (picker = null)}
    onpick={(name) => {
      void edit({ kind: "variable", ...selected, name });
      picker = null;
    }}
  />{/if}
