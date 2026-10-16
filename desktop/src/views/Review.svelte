<script lang="ts">
  import {
    api,
    agentName,
    basename,
    type Snapshot,
    type Conflict,
    type Resolution,
  } from "../api";
  import { errorText, title, reviewSummary } from "../api";
  const loadConflictEditor = () =>
    import("../components/ConflictSource.svelte");
  import Diff from "../components/Diff.svelte";
  import Dialog from "../components/Dialog.svelte";
  let {
    workspace,
    onupdate,
    onerror,
    ondirty,
  }: {
    workspace: Snapshot;
    onupdate: (w: Snapshot) => void;
    onerror: (e: unknown) => void;
    ondirty: (v: boolean) => void;
  } = $props();
  const summary = $derived(reviewSummary(workspace));
  let sourceConflict = $state<Conflict | null>(null);
  let resolution = $state<Resolution | null>(null);
  let busy = $state(false);
  let backup = $state("");
  let legacyBackup = $state("");
  let legacy = $state<Snapshot["legacy"][number] | null>(null);
  let legacyPreview = $state<unknown>(null);
  $effect(() =>
    ondirty(
      busy || resolution !== null || legacy !== null || sourceConflict !== null,
    ),
  );
  function identity(c: Conflict) {
    const parts = c.path.split(/[\\/]/);
    const i = parts.lastIndexOf("skills");
    const agent = [
      ...new Set(workspace.preview_targets.map((t) => t.harness)),
    ].find((a) => parts.includes(a) || parts.includes(`.${a}`));
    return {
      name: c.skill ?? (i >= 0 ? parts[i + 1] : basename(c.path)),
      agent: c.harness
        ? agentName(c.harness)
        : agent
          ? agentName(agent)
          : "An agent",
    };
  }
  async function choose(conflict: Conflict, action: string) {
    try {
      busy = true;
      resolution = await api.command<Resolution>("preview_conflict", {
        path: conflict.path,
        action,
      });
    } catch (e) {
      onerror(e);
    } finally {
      busy = false;
    }
  }
  async function resolve() {
    if (!resolution) return;
    busy = true;
    try {
      backup = await api.command<string>("resolve_conflict", {
        expected: JSON.stringify(resolution),
      });
      resolution = null;
      const w = await api.workspace();
      if (w) onupdate(w);
    } catch (e) {
      onerror(e);
    } finally {
      busy = false;
    }
  }
  async function apply() {
    busy = true;
    try {
      onupdate(
        await api.command<Snapshot>("apply_workspace", {
          expectedPlan: JSON.stringify(workspace.plan),
        }),
      );
    } catch (e) {
      onerror(e);
    } finally {
      busy = false;
    }
  }
  async function previewLegacy(item: Snapshot["legacy"][number]) {
    try {
      legacyPreview = await api.command("preview_legacy_resolution", {
        name: item.name,
      });
      legacy = item;
    } catch (e) {
      onerror(e);
    }
  }
  async function archiveLegacy() {
    try {
      legacyBackup = await api.command<string>("resolve_legacy", {
        expected: JSON.stringify(legacyPreview),
      });
      legacy = null;
      const w = await api.workspace();
      if (w) onupdate(w);
    } catch (e) {
      onerror(e);
    }
  }
</script>

<section class="page">
  <div class="page-heading">
    <div>
      <p class="eyebrow">YOU’RE IN CONTROL</p>
      <h1>
        Review changes <span class="count">{summary.total}</span>
      </h1>
      <p class="muted">
        {summary.readyChanges.length} ready · {summary.decisions}
        {summary.decisions === 1 ? "needs" : "need"} your decision. The badge counts
        both; Apply installs ready changes after all decisions are resolved.
      </p>
    </div>
    <button
      class="primary"
      disabled={busy ||
        workspace.plan.conflicts.length > 0 ||
        !workspace.plan.changes.length ||
        workspace.legacy.length > 0}
      onclick={apply}
      >Apply {summary.readyChanges.length}
      {summary.readyChanges.length === 1 ? "change" : "changes"}</button
    >
  </div>
  {#if backup}<p role="status" class="success-message">
      Saved a recovery backup. <button
        onclick={async () => {
          try {
            onupdate(
              await api.command<Snapshot>("restore_workspace", { backup }),
            );
            backup = "";
          } catch (e) {
            onerror(e);
          }
        }}>Undo resolution</button
      >
      <details><summary>Backup location</summary><code>{backup}</code></details>
    </p>{/if}
  <p class="small muted">
    {#if legacyBackup}Legacy copy safely archived at <code>{legacyBackup}</code
      >. Move this directory back to its original location to recover it, after
      checking for name collisions.{/if}
  </p>
  {#each workspace.legacy as item}<article class="conflict-card">
      <span class="overline warning-text">DUPLICATE CODEX SKILL</span>
      <h2>Codex can see two copies of {item.name}</h2>
      <p class="muted">
        A same-named skill is still in Codex’s legacy folder. Review and archive
        that copy before installing here.
      </p>
      <button onclick={() => previewLegacy(item)}>Review legacy copy…</button>
      <details><summary>File details</summary><code>{item.path}</code></details>
    </article>{/each}
  {#each workspace.plan.conflict_details as conflict}{@const who =
      identity(conflict)}
    <article class="conflict-card">
      <span class="overline warning-text">NEEDS YOUR DECISION</span>
      <h2>
        {who.agent}’s copy of {who.name}
        {conflict.kind === "removed"
          ? "was removed"
          : conflict.kind.startsWith("unmanaged")
            ? "already exists outside Ratatoskr"
            : "was edited"} on {title(workspace.plan.machine)}
      </h2>
      <p class="muted">
        Your source hasn’t changed. Choose which version to keep.
      </p>
      <Diff
        diff={conflict.diff}
        reverse
        left="Your source"
        right="Edited copy"
        neutral
      />
      <div class="conflict-actions">
        <button
          class="primary"
          disabled={!!conflict.copy_back_blocked || busy}
          title={conflict.copy_back_blocked ??
            "Preserve the installed edit in your source"}
          onclick={() => choose(conflict, "copy-back")}
          >Keep edit → update source</button
        ><button disabled={busy} onclick={() => choose(conflict, "reinstall")}
          >Restore from source</button
        ><button
          disabled={!conflict.source}
          onclick={() => (sourceConflict = conflict)}>Edit source…</button
        ><button disabled={busy} onclick={() => choose(conflict, "adopt")}
          >Take over as-is</button
        >
      </div>
      {#if conflict.copy_back_blocked}<p class="small muted">
          Keep edit unavailable: {conflict.copy_back_blocked}
        </p>{/if}
      <details>
        <summary>File details</summary><code>{conflict.path}</code>
      </details>
    </article>{/each}
  {#each workspace.plan.conflicts.filter((message) => !workspace.plan.conflict_details.some( (c) => message.includes(c.path) )) as message}<p
      role="alert"
      class="error-message"
    >
      {message}
    </p>{/each}
  {#each summary.readyChanges as change}<article class="change-card">
      <div class="section-heading">
        <h2>{basename(change.path)}</h2>
        <span class="status-pill">{change.action}</span>
      </div>
      <Diff
        diff={change.diff}
        reverse
        left="Your source (rendered)"
        right="Installed copy"
      />
      <details>
        <summary>File details</summary><code>{change.path}</code>
        <p>
          Before: {change.before_sha256 ?? "No file"}<br />After: {change.after_sha256 ??
            "Removed"}
        </p>
      </details>
    </article>{/each}
  {#if !workspace.plan.changes.length && !workspace.plan.conflicts.length && !workspace.legacy.length}<div
      class="empty-state"
    >
      <span class="large-check">✓</span>
      <h2>Everything is in sync</h2>
      <p>Your installed skills match their source.</p>
    </div>{/if}
</section>
{#if resolution}<Dialog
    title="Confirm this resolution"
    onclose={() => (resolution = null)}
    ><p>
      Ratatoskr will back up the installed copy and any affected source before
      making these changes.
    </p>
    {#each resolution.effects as effect}<h3>{basename(effect.path)}</h3>
      <Diff diff={effect.diff ?? null} left="Before" right="After" />{/each}
    <details>
      <summary>Full preview</summary>
      <pre>{JSON.stringify(resolution, null, 2)}</pre>
    </details>
    <footer class="dialog-actions">
      <button class="primary" disabled={busy} onclick={resolve}
        >Back up and confirm</button
      ><button onclick={() => (resolution = null)}>Cancel</button>
    </footer></Dialog
  >{/if}
{#if legacy}<Dialog
    title={`Archive legacy ${legacy.name}`}
    onclose={() => (legacy = null)}
    ><p>
      The entire legacy skill directory will move to a dated backup outside
      Codex’s scanned folders. Nothing is deleted.
    </p>
    <div class="resource-row"><code>{legacy.path}</code></div>
    {#each legacy.files as file}<div class="resource-row">
        ▤ {file.path}
      </div>{/each}
    <details>
      <summary>Verified preview</summary>
      <pre>{JSON.stringify(legacyPreview, null, 2)}</pre>
    </details>
    <footer class="dialog-actions">
      <button class="primary" onclick={archiveLegacy}
        >Back up and archive legacy copy</button
      ><button onclick={() => (legacy = null)}>Cancel</button>
    </footer></Dialog
  >{/if}

{#if sourceConflict}{#await loadConflictEditor()}<p role="status">
      Opening source editor…
    </p>{:then { default: ConflictSource }}<ConflictSource
      conflict={sourceConflict}
      {workspace}
      onclose={() => (sourceConflict = null)}
      {onerror}
      onpreview={(preview) => {
        sourceConflict = null;
        resolution = preview;
      }}
    />{:catch e}<p role="alert" class="error-message">
      Source editor failed to load: {errorText(e)}
    </p>{/await}{/if}
