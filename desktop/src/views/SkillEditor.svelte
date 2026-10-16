<script lang="ts">
  import { onMount } from "svelte";
  import {
    api,
    agentName,
    title,
    errorText,
    emptyAnalysis,
    type Snapshot,
    type Skill,
    type SkillDocument,
    type EditorView,
    type Analysis,
    type Range,
    type Block,
    type PreviewFile,
    type ValueOption,
    type Resource,
    type Value,
  } from "../api";
  import { createLivePreview } from "../live-preview";
  import MarkdownEditor from "../components/MarkdownEditor.svelte";
  import Chips from "../components/Chips.svelte";
  import Dialog from "../components/Dialog.svelte";
  import OnlyPicker from "../components/OnlyPicker.svelte";
  let {
    skill,
    workspace,
    onupdate,
    onerror,
    ondirty,
    onrenamed,
  }: {
    skill: Skill;
    workspace: Snapshot;
    onupdate: (w: Snapshot) => void;
    onerror: (e: unknown) => void;
    ondirty: (v: boolean) => void;
    onrenamed: (skill: Skill) => void;
  } = $props();
  let document = $state<SkillDocument | null>(null);
  let markdown = $state("");
  let metadata = $state("");
  let info = $state<EditorView | null>(null);
  let analysis = $state<Analysis>(emptyAnalysis);
  let preview = $state<PreviewFile[]>([]);
  let changedLines = $state<number[]>([]);
  let values = $state<ValueOption[]>([]);
  let resources = $state<Resource[]>([]);
  let previewError = $state("");
  let installPath = $state("");
  let validating = $state(true);
  let busy = $state(false);
  const agents = $derived([
    ...new Set(workspace.preview_targets.map((t) => t.harness)),
  ]);
  const machines = $derived([
    ...new Set(workspace.preview_targets.map((t) => t.machine)),
  ]);
  let machine = $state("");
  let harness = $state("");
  let raw = $state(false);
  let selected = $state<Range>({ from: 0, to: 0 });
  let picker = $state<"only" | "value" | "rename" | "variant" | null>(null);
  let block = $state<Block | null>(null);
  let editor = $state<MarkdownEditor>();
  let newName = $state("");
  let variantFile = $state("");
  let variantSystem = $state("windows");
  let extraAgent = $state("");
  let extraKey = $state("");
  let extraValue = $state("");
  let description = $state("");
  const dirty = $derived(
    !!document &&
      (markdown !== document.markdown ||
        metadata !== document.metadata ||
        description !== info?.description),
  );
  $effect(() => ondirty(dirty || busy || pendingEdits > 0 || picker !== null));
  const validation = createLivePreview(
    async (input: {
      markdown: string;
      metadata: string;
      machine: string;
      harness: string;
    }) => {
      const a = await api.analyze(
        skill.name,
        input.markdown,
        input.machine,
        input.harness,
      );
      const v = await api.view(input.markdown, input.metadata);
      const options = await api.values(input.machine, input.harness);
      // Rendering is authoritative. Inactive or excluded drafts deliberately return no files.
      let files: PreviewFile[] = [];
      let alternate: PreviewFile[][] = [];
      let error = "";
      try {
        files = await api.preview(
          skill,
          input.markdown,
          input.metadata,
          input.machine,
          input.harness,
        );
        alternate = await Promise.all(
          workspace.preview_targets
            .filter(
              (t) => t.machine === input.machine && t.harness !== input.harness,
            )
            .map((t) =>
              api.preview(
                skill,
                input.markdown,
                input.metadata,
                t.machine,
                t.harness,
              ),
            ),
        );
      } catch (e) {
        error = errorText(e);
      }
      const destination = await api.command<string>("preview_install_path", {
        name: skill.name,
        project: skill.project,
        machine: input.machine,
        harness: input.harness,
      });
      const changed = await api.command<number[]>("different_preview_lines", {
        text: files.find((f) => f.path === "SKILL.md")?.body ?? "",
        versions: alternate.map(
          (files) => files.find((f) => f.path === "SKILL.md")?.body ?? "",
        ),
      });
      return { a, v, options, files, changed, error, destination };
    },
    (result) => {
      analysis = result.a;
      info = result.v;
      if (raw) description = result.v.description;
      installPath = result.destination;
      values = result.options;
      preview = result.files;
      changedLines = result.changed;
      previewError = result.error;
      validating = false;
    },
    (e) => {
      previewError = errorText(e);
      preview = [];
      changedLines = [];
      validating = false;
    },
    150,
  );
  $effect(() => {
    if (document && harness) {
      validating = true;
      previewError = "";
      validation.schedule({ markdown, metadata, machine, harness });
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
      const d = await api.readSkill(skill);
      const v = await api.view(d.markdown, d.metadata);
      document = d;
      markdown = d.markdown;
      metadata = d.metadata;
      info = v;
      description = v.description;
      resources = await api.resources(skill);
      selected = { from: v.frontmatter.to, to: v.frontmatter.to };
    } catch (e) {
      onerror(e);
    }
  }
  let editQueue = Promise.resolve();
  let pendingEdits = $state(0);
  function edit(operation: Record<string, unknown>): Promise<void> {
    pendingEdits++;
    const next = editQueue
      .then(async () => {
        let originalMarkdown = markdown;
        let originalMetadata = metadata;
        let result = await api.edit(
          originalMarkdown,
          originalMetadata,
          operation,
        );
        if (markdown !== originalMarkdown || metadata !== originalMetadata) {
          if (operation.kind !== "description" && operation.kind !== "metadata")
            throw new Error(
              "The selection changed during the edit. Try the command again.",
            );
          originalMarkdown = markdown;
          originalMetadata = metadata;
          result = await api.edit(
            originalMarkdown,
            originalMetadata,
            operation,
          );
          if (markdown !== originalMarkdown || metadata !== originalMetadata)
            throw new Error("The draft changed during the edit. Try again.");
        }
        if (result.markdown !== markdown) {
          editor?.replace(result.markdown);
          markdown = result.markdown;
        }
        metadata = result.metadata;
        info = await api.view(markdown, metadata);
      })
      .catch(onerror)
      .finally(() => pendingEdits--);
    editQueue = next;
    return next;
  }
  export async function save() {
    if (!document || busy) return;
    await editQueue;
    if (
      validating ||
      previewError ||
      analysis.diagnostics.some((d) => d.severity === "error")
    ) {
      onerror("Fix validation errors before saving.");
      return;
    }
    busy = true;
    try {
      const w = await api.command<Snapshot>("save_skill_draft", {
        name: skill.name,
        project: skill.project,
        markdown,
        metadata,
        markdownHash: document.markdown_hash,
        metadataHash: document.metadata_hash,
      });
      onupdate(w);
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
  function target(key: string): string[] {
    return (
      ((info?.metadata.target as Record<string, Value> | undefined)?.[key] as
        string[] | undefined) ?? []
    );
  }
  function setTarget(key: string, value: string[]) {
    void edit({
      kind: "metadata",
      path: ["target", key],
      value: value.length ? value : null,
    });
  }
  function openOnly(b: Block | null = null) {
    block = b;
    picker = "only";
  }
  function onblock(b: Block, remove: boolean) {
    if (remove) void edit({ kind: "remove-only", from: b.opening.from });
    else openOnly(b);
  }
  function changeMachine(value: string) {
    machine = value;
    if (
      !workspace.preview_targets.some(
        (t) => t.machine === machine && t.harness === harness,
      )
    )
      harness =
        workspace.preview_targets.find((t) => t.machine === machine)?.harness ??
        "";
  }
  async function rename() {
    if (!document) return;
    busy = true;
    try {
      onupdate(
        await api.command<Snapshot>("rename_source_skill", {
          name: skill.name,
          project: skill.project,
          newName,
          expectedHash: document.markdown_hash,
        }),
      );
      picker = null;
      onrenamed({ ...skill, name: newName });
    } catch (e) {
      onerror(e);
    } finally {
      busy = false;
    }
  }
  async function addResource(variant: boolean) {
    try {
      const from = await api.command<string | null>("choose_resource");
      if (!from) return;
      const file = variant
        ? `variants/os=${variantSystem}/${variantFile}`
        : from.split(/[\\/]/).at(-1)!;
      onupdate(
        await api.command<Snapshot>("add_skill_resource", {
          name: skill.name,
          project: skill.project,
          file,
          from,
        }),
      );
      resources = await api.resources(skill);
      picker = null;
    } catch (e) {
      onerror(e);
    }
  }
  const extras = $derived(
    (info?.metadata.frontmatter ?? {}) as Record<string, Record<string, Value>>,
  );
  const capabilityOptions = $state<string[]>([]);
  onMount(() => {
    void Promise.all(
      workspace.source_files
        .filter((f) => f.startsWith("machines/"))
        .map((f) => api.source(f)),
    )
      .then((profiles) => {
        capabilityOptions.push(
          ...new Set(
            profiles.flatMap((p) => (p.values.capabilities ?? []) as string[]),
          ),
        );
      })
      .catch(onerror);
  });
</script>

{#if document && info}
  <div class="editor-layout">
    <section class="editor-source">
      <div class="page-heading compact">
        <div>
          <p class="eyebrow">
            {skill.project ? "PROJECT SKILL" : "GLOBAL SKILL"}
          </p>
          <h1>{skill.name}</h1>
        </div>
        <div class="heading-actions">
          <span
            class="status-pill {analysis.diagnostics.some(
              (d) => d.severity === 'error',
            ) || previewError
              ? 'warning'
              : 'success'}"
            >{validating
              ? "Checking…"
              : previewError
                ? "Needs attention"
                : analysis.diagnostics.some((d) => d.severity === "error")
                  ? "Fix errors"
                  : "✓ Valid"}</span
          ><button
            class="subtle"
            disabled={dirty}
            onclick={() => {
              newName = skill.name;
              picker = "rename";
            }}>Rename</button
          >
        </div>
      </div>
      <label class="description-field"
        ><span>Description <small>{[...description].length} / 1024</small></span
        ><textarea
          aria-label="Description"
          rows="2"
          bind:value={description}
          oninput={(e) => {
            void edit({ kind: "description", value: e.currentTarget.value });
          }}></textarea></label
      >
      <div class="targeting">
        <p class="eyebrow">
          APPLIES TO <span>Leave a group empty for everyone</span>
        </p>
        <Chips
          label="Agents"
          options={agents}
          selected={target("harnesses")}
          dimension="agent"
          onchange={(v) => setTarget("harnesses", v)}
        /><Chips
          label="Machines"
          options={machines}
          selected={target("machines")}
          dimension="machine"
          onchange={(v) => setTarget("machines", v)}
        />{#if capabilityOptions.length}<Chips
            label="Needs"
            options={capabilityOptions}
            selected={target("capabilities")}
            dimension="machine"
            onchange={(v) => setTarget("capabilities", v)}
          />{/if}<Chips
          label="System"
          options={["macos", "windows", "linux"]}
          selected={target("os")}
          dimension="os"
          onchange={(v) => setTarget("os", v)}
        />
      </div>
      <div class="editor-frame">
        <div
          class="format-toolbar"
          role="toolbar"
          aria-label="Markdown formatting"
        >
          <button aria-label="Bold" onclick={() => editor?.format("bold")}
            ><b>B</b></button
          ><button
            aria-label="Heading"
            onclick={() => editor?.format("heading")}>H2</button
          ><button aria-label="List" onclick={() => editor?.format("list")}
            >☷</button
          ><button aria-label="Code" onclick={() => editor?.format("code")}
            >‹/›</button
          ><span class="toolbar-divider"></span><button
            class="accent-text"
            onclick={() => openOnly()}>＋ Only for…</button
          ><button class="accent-text" onclick={() => (picker = "value")}
            >＋ Insert value…</button
          ><button
            class="source-toggle"
            aria-pressed={raw}
            onclick={() => (raw = !raw)}>Markdown source</button
          >
        </div>
        <MarkdownEditor
          bind:this={editor}
          value={markdown}
          {analysis}
          frontmatter={info.frontmatter}
          {raw}
          onchange={(text) => (markdown = text)}
          onselection={(range) =>
            (selected = { from: range.from, to: range.to })}
          {onblock}
        />
      </div>
      {#if previewError}<p role="alert" class="error-message">
          {previewError}
        </p>{/if}
      <details class="advanced">
        <summary>Advanced <span>Per-agent frontmatter</span></summary>
        <p class="muted">
          Extra fields for one agent. Identity and description stay shared.
        </p>
        {#each Object.entries(extras) as [agent, entries]}{#each Object.entries(entries) as [key, value]}<div
              class="key-value-row"
            >
              <span class="chip {agent}">{agentName(agent)}</span><label
                >{key}<input
                  aria-label={`${agentName(agent)} ${key}`}
                  value={typeof value === "string"
                    ? value
                    : JSON.stringify(value)}
                  onchange={(e) => {
                    let next: Value = e.currentTarget.value;
                    if (typeof value !== "string") {
                      try {
                        next = JSON.parse(e.currentTarget.value);
                      } catch (error) {
                        onerror(error);
                        return;
                      }
                    }
                    void edit({
                      kind: "metadata",
                      path: ["frontmatter", agent, key],
                      value: next,
                    });
                  }}
                /></label
              ><button
                aria-label={`Remove ${key}`}
                onclick={() =>
                  edit({
                    kind: "metadata",
                    path: ["frontmatter", agent, key],
                    value: null,
                  })}>×</button
              >
            </div>{/each}{/each}
        <div class="form-row">
          <label
            >Agent<select bind:value={extraAgent}
              ><option value="">Choose agent</option>{#each agents as a}<option
                  value={a}>{agentName(a)}</option
                >{/each}</select
            ></label
          ><label>Field name<input bind:value={extraKey} /></label><label
            >Value<input bind:value={extraValue} /></label
          ><button
            disabled={!extraAgent ||
              !extraKey ||
              ["name", "description"].includes(extraKey)}
            onclick={() => {
              void edit({
                kind: "metadata",
                path: ["frontmatter", extraAgent, extraKey],
                value: extraValue,
              });
              extraKey = "";
              extraValue = "";
            }}>Add field</button
          >
        </div>
      </details>
      <section class="resources">
        <div class="section-heading">
          <h2>Files <span class="count">{resources.length}</span></h2>
          <div>
            <button
              class="subtle"
              disabled={dirty}
              onclick={() => addResource(false)}>Add file</button
            ><button
              class="subtle"
              disabled={dirty}
              onclick={() => (picker = "variant")}>Add variant</button
            >
          </div>
        </div>
        {#each resources as file}<div class="resource-row">
            <span>▤</span><span title={file.path}>{file.label}</span><small
              >{file.path.startsWith("variants/") ? "Variant" : "Source"}</small
            >
          </div>{/each}
      </section>
    </section>
    <aside class="live-preview">
      <p class="eyebrow">LIVE PREVIEW <span class="live-dot"></span></p>
      <div class="preview-selects">
        <label
          >Agent<select aria-label="Preview agent" bind:value={harness}
            >{#each workspace.preview_targets.filter((t) => t.machine === machine) as t}<option
                value={t.harness}>{agentName(t.harness)}</option
              >{/each}</select
          ></label
        ><label
          >Machine<select
            aria-label="Preview machine"
            value={machine}
            onchange={(e) => changeMachine(e.currentTarget.value)}
            >{#each machines as m}<option value={m}>{title(m)}</option
              >{/each}</select
          ></label
        >
      </div>
      <div class="preview-document" aria-label="Rendered preview">
        {#if validating}<span class="muted">Updating preview…</span
          >{:else if previewError}<p class="muted">
            Preview will return when the errors are fixed.
          </p>{:else if !preview.length}<p class="muted">
            This skill does not apply to this agent and machine.
          </p>{:else}{#each (preview.find((f) => f.path === "SKILL.md")?.body ?? "").split("\n") as line, index}<div
              class:changed-line={!!line.trim() && changedLines.includes(index)}
            >
              {line || " "}
            </div>{/each}{/if}
      </div>
      <p class="install-path">
        ✓ {workspace.plan.mode === "staging" ? "Staged for" : "Installs for"}
        {agentName(harness)} on {title(machine)}
      </p>
      <details>
        <summary>Installation details</summary>
        <p class="path">{installPath}</p>
      </details>
      <p class="muted small">
        Highlighted lines differ from the other agents’ versions.
      </p>
      <div class="preview-files">
        {#each preview as f}<span>▤ {f.path}</span>{/each}
      </div>
    </aside>
  </div>
{:else}<div class="empty-state">Opening skill…</div>{/if}
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
{#if picker === "value"}<Dialog
    title="Insert a value"
    onclose={() => (picker = null)}
    ><p class="muted">Resolved for {title(machine)} · {agentName(harness)}</p>
    <div class="value-options">
      {#each values as value}<button
          onclick={() => {
            void edit({ kind: "variable", ...selected, name: value.name });
            picker = null;
          }}
          ><strong>▣ {title(value.label)}</strong><span>{value.value}</span
          ></button
        >{/each}
    </div></Dialog
  >{/if}
{#if picker === "rename"}<Dialog
    title="Rename skill"
    onclose={() => (picker = null)}
    ><p class="muted">
      Updates the directory and its name together. Installed copies change on
      the next Apply.
    </p>
    <label>New name<input bind:value={newName} /></label>
    <footer class="dialog-actions">
      <button
        class="primary"
        disabled={busy || newName === skill.name || !newName}
        onclick={rename}>Rename skill</button
      ><button onclick={() => (picker = null)}>Cancel</button>
    </footer></Dialog
  >{/if}
{#if picker === "variant"}<Dialog
    title="Add a system variant"
    onclose={() => (picker = null)}
    ><p class="muted">
      Choose a replacement file for a specific system. The shared file stays
      unchanged.
    </p>
    <label
      >System<select bind:value={variantSystem}
        >{#each ["windows", "macos", "linux"] as os}<option value={os}
            >{title(os)}</option
          >{/each}</select
      ></label
    ><label
      >Replaces file<select bind:value={variantFile}
        ><option value="">Choose resource</option
        >{#each resources.filter((f) => !f.path.startsWith("variants/")) as f}<option
            value={f.path}>{f.label}</option
          >{/each}</select
      ></label
    >
    <footer class="dialog-actions">
      <button
        class="primary"
        disabled={!variantFile}
        onclick={() => addResource(true)}>Choose replacement file…</button
      >
    </footer></Dialog
  >{/if}
