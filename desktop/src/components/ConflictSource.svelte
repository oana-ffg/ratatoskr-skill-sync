<script lang="ts">
  import { onMount } from "svelte";
  import {
    api,
    emptyAnalysis,
    type Snapshot,
    type Conflict,
    type Analysis,
    type EditorView,
    type Block,
    type Range,
    type Resolution,
    type ValueOption,
  } from "../api";
  import { createLivePreview } from "../live-preview";
  import MarkdownEditor from "./MarkdownEditor.svelte";
  import Dialog from "./Dialog.svelte";
  import OnlyPicker from "./OnlyPicker.svelte";
  import ValuePicker from "./ValuePicker.svelte";
  let {
    conflict,
    workspace,
    onclose,
    onpreview,
    onerror,
  }: {
    conflict: Conflict;
    workspace: Snapshot;
    onclose: () => void;
    onpreview: (preview: Resolution) => void;
    onerror: (error: unknown) => void;
  } = $props();
  let text = $state("");
  let loaded = $state(false);
  let info = $state<EditorView | null>(null);
  let analysis = $state<Analysis>(emptyAnalysis);
  let selected = $state<Range>({ from: 0, to: 0 });
  let condition = $state(false);
  let editing = $state<Block | null>(null);
  let editor = $state<MarkdownEditor>();
  let raw = $state(false);
  let busy = $state(false);
  let valuePicker = $state(false);
  let values = $state<ValueOption[]>([]);
  let editQueue = Promise.resolve();
  const isSkill = $derived(conflict.source?.endsWith("SKILL.md") ?? false);
  const name = $derived(
    conflict.skill ??
      conflict.path
        .split(/[\\/]/)
        .at(conflict.path.split(/[\\/]/).lastIndexOf("skills") + 1) ??
      "",
  );
  const target = $derived(
    workspace.preview_targets.find((t) => t.machine === workspace.plan.machine),
  );
  const check = createLivePreview(
    async (value: string) => {
      if (!isSkill || !target) return null;
      return {
        view: await api.view(value, ""),
        analysis: await api.analyze(
          name,
          value,
          target.machine,
          target.harness,
        ),
      };
    },
    (result) => {
      info = result?.view ?? null;
      analysis = result?.analysis ?? emptyAnalysis;
    },
    (e) => onerror(e),
    150,
  );
  onMount(() => {
    void api
      .command<{ text: string }>("read_conflict_source", {
        path: conflict.path,
      })
      .then((document) => {
        text = document.text;
        loaded = true;
      })
      .catch(onerror);
    return () => check.cancel();
  });
  $effect(() => {
    if (loaded) check.schedule(text);
  });
  function edit(operation: Record<string, unknown>): Promise<void> {
    const next = editQueue
      .then(async () => {
        const before = text;
        const result = await api.edit(before, "", operation);
        if (text !== before)
          throw new Error("Source changed during the command. Try again.");
        editor?.replace(result.markdown);
        text = result.markdown;
      })
      .catch(onerror);
    editQueue = next;
    return next;
  }
  async function preview() {
    await editQueue;
    busy = true;
    try {
      onpreview(
        await api.command<Resolution>("preview_source_edit", {
          path: conflict.path,
          text,
        }),
      );
    } catch (e) {
      onerror(e);
    } finally {
      busy = false;
    }
  }
</script>

<Dialog title="Edit the mapped source" {onclose}
  ><p class="muted">
    This edits the exact source file behind the conflict, including a selected
    variant. Review and back up the change before saving.
  </p>
  {#if loaded}{#if info}<label
        >Description<textarea
          value={info.description}
          oninput={(e) =>
            edit({ kind: "description", value: e.currentTarget.value })}
        ></textarea></label
      >{/if}
    <div class="format-toolbar">
      {#if isSkill}<button
          onclick={async () => {
            if (!target) return;
            try {
              values = await api.values(target.machine, target.harness);
              valuePicker = true;
            } catch (e) {
              onerror(e);
            }
          }}>Insert value…</button
        >{/if}
      <button onclick={() => editor?.format("bold")}>Bold</button
      >{#if isSkill}<button
          onclick={() => {
            editing = null;
            condition = true;
          }}>Only for…</button
        >{/if}<button aria-pressed={raw} onclick={() => (raw = !raw)}
        >Markdown source</button
      >
    </div>
    <MarkdownEditor
      bind:this={editor}
      label="Source Markdown editor"
      value={text}
      {analysis}
      frontmatter={info?.frontmatter ?? null}
      {raw}
      onchange={(v) => (text = v)}
      onselection={(r) => (selected = r)}
      onblock={(b, remove) => {
        if (remove) void edit({ kind: "remove-only", from: b.opening.from });
        else {
          editing = b;
          condition = true;
        }
      }}
    />
    <footer class="dialog-actions">
      <button class="primary" disabled={busy} onclick={preview}
        >Review source change</button
      ><button onclick={onclose}>Cancel</button>
    </footer>{:else}<p>Opening source…</p>{/if}</Dialog
>
{#if condition}<OnlyPicker
    agents={[...new Set(workspace.preview_targets.map((t) => t.harness))]}
    machines={[...new Set(workspace.preview_targets.map((t) => t.machine))]}
    initial={editing?.selectors ?? {}}
    onclose={() => (condition = false)}
    onapply={(selectors) => {
      void edit({
        kind: "only",
        ...(editing?.range ?? selected),
        selectors,
        existing: !!editing,
      });
      condition = false;
    }}
  />{/if}

{#if valuePicker && target}<ValuePicker
    {values}
    machine={target.machine}
    harness={target.harness}
    onclose={() => (valuePicker = false)}
    onpick={(name) => {
      void edit({ kind: "variable", ...selected, name });
      valuePicker = false;
    }}
  />{/if}
