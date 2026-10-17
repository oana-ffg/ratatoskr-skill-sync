<script lang="ts">
  import {
    api,
    agentName,
    basename,
    type Snapshot,
    type Conflict,
    type Resolution,
  } from "../api";
  import { errorText, title, reviewSummary, applyAction } from "../api";
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
      <h1>Review changes <span class="count">{summary.total}</span></h1>
      <p class="muted">
        {summary.total}
        {summary.total === 1 ? "item" : "items"} needing attention · {summary
          .readyChanges.length} ready files · {summary.decisions}
        {summary.decisions === 1 ? "decision" : "decisions"}
      </p>
    </div>
    <button
      class="primary"
      disabled={busy ||
        workspace.plan.conflicts.length > 0 ||
        !workspace.plan.changes.length ||
        workspace.legacy.length > 0}
      onclick={apply}
      aria-describedby="apply-destination">{applyAction(workspace)}</button
    >
  </div>
  <div id="apply-destination" class="apply-destination">
    {#if workspace.plan.mode === "staging"}
      <p>
        Apply writes only to the staging folder <code
          >{workspace.plan.staging_root}</code
        >. Your real agent folders are not touched.
      </p>
    {:else}
      <p>Apply changes files in these real agent folders:</p>
      <ul>
        {#each workspace.plan.install_targets as target}<li>
            {agentName(target.harness)}: <code>{target.skills_root}</code> and
            <code>{target.instructions_file}</code>
          </li>{/each}
      </ul>
    {/if}
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
  {#if workspace.legacy.length}<section
      class="legacy-section"
      aria-label="Older Codex copies"
    >
      <h2>Older Codex copies</h2>
      <p>Nothing changes until you choose what to do with each older copy.</p>
      {#each workspace.legacy as item}<article class="conflict-card">
          <h3>{item.name}</h3>
          <p>
            Codex also has an older copy of {item.name} in
            <code>{item.path}</code>. When Ratatoskr installs to
            <code>{item.destination}</code>, Codex will see both.
          </p>
          <button disabled={busy} onclick={() => previewLegacy(item)}
            >Back up and retire the old copy…</button
          >
          <button
            disabled={busy}
            onclick={async () => {
              busy = true;
              try {
                onupdate(
                  await api.command<Snapshot>("keep_legacy", {
                    expected: JSON.stringify(item),
                  }),
                );
              } catch (e) {
                onerror(e);
              } finally {
                busy = false;
              }
            }}>Keep both copies</button
          >
        </article>{/each}
    </section>{/if}
  {#each summary.groups as group}<article class="change-card">
      <h2>
        {group.name} → {group.harness
          ? agentName(group.harness)
          : "Other files"}: {group.changes.length + group.conflicts.length}
        {group.changes.length + group.conflicts.length === 1 ? "file" : "files"}
      </h2>
      {#if group.project}<p>Project: <code>{group.project}</code></p>{/if}
      {#if group.conflicts.length}<p class="warning-text">
          Needs your decision
        </p>{/if}
      <details class="group-files" open={group.conflicts.length > 0}>
        <summary>Files</summary>
        {#each group.conflicts as conflict}
          <article class="conflict-card">
            <h2>
              {group.harness ? agentName(group.harness) : "An agent"}’s copy of {group.name}
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
              ><button
                disabled={busy}
                onclick={() => choose(conflict, "reinstall")}
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
        {#each group.changes as change}<details class="review-file">
            <summary
              >{change.path
                .replaceAll("\\", "/")
                .split(`/${group.name}/`)
                .at(-1)} · {change.action}</summary
            >
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
          </details>{/each}
      </details>
    </article>{/each}
  {#each summary.errors as message}<p role="alert" class="error-message">
      {message}
    </p>{/each}
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
    title={`Retire older copy of ${legacy.name}`}
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
        >Back up and retire old copy</button
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
