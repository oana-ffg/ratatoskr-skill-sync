<script lang="ts">
  import { api, basename, type Snapshot } from "../api";
  import Dialog from "../components/Dialog.svelte";
  let {
    workspace,
    onupdate,
    onerror,
    onbrowse,
    ondirty,
  }: {
    workspace: Snapshot;
    onupdate: (w: Snapshot) => void;
    onerror: (e: unknown) => void;
    onbrowse: (root: string) => void;
    ondirty: (v: boolean) => void;
  } = $props();
  let removing = $state<string | null>(null);
  let selected = $state("");
  let source = $state("agent-skills");
  $effect(() => ondirty(!!selected || !!removing));
  async function choose() {
    try {
      selected = (await api.folder()) ?? "";
    } catch (e) {
      onerror(e);
    }
  }
  async function add() {
    try {
      onupdate(
        await api.command<Snapshot>("add_monitored_project", {
          root: selected,
          source,
        }),
      );
      selected = "";
    } catch (e) {
      onerror(e);
    }
  }
  async function remove() {
    try {
      onupdate(
        await api.command<Snapshot>("remove_monitored_project", {
          root: removing,
        }),
      );
      removing = null;
    } catch (e) {
      onerror(e);
    }
  }
</script>

<section class="page">
  <div class="page-heading">
    <div>
      <h1>
        Projects <span class="count">{workspace.plan.projects.length}</span>
      </h1>
    </div>
    <button class="primary" onclick={choose}>＋ Add project…</button>
  </div>
  <div class="project-grid">
    {#each workspace.plan.projects as project}<article
        class="panel project-card"
      >
        <div class="project-symbol">▱</div>
        <div class="section-heading">
          <h2>{basename(project.root)}</h2>
          <span class="status-pill {project.error ? 'warning' : 'success'}"
            >{project.error ? "Needs attention" : "Connected"}</span
          >
        </div>
        <p class="muted">
          {project.changed_files} uncommitted skill {project.changed_files === 1
            ? "file"
            : "files"} · {project.outgoing_commits ?? "Unknown"} unpushed {project.outgoing_commits ===
          1
            ? "commit"
            : "commits"}
        </p>
        {#if project.error}<p role="alert" class="error-message">
            {project.error}
          </p>{/if}
        <div class="project-actions">
          <button onclick={() => onbrowse(project.root)}>View skills →</button
          ><button class="subtle" onclick={() => (removing = project.root)}
            >Remove…</button
          >
        </div>
        <details>
          <summary>Project details</summary>
          <p class="path">{project.root}</p>
          <p>Upstream: {project.upstream ?? "Not configured"}</p>
        </details>
      </article>{/each}
  </div>
  {#if !workspace.plan.projects.length}<div class="empty-state">
      <h2>No projects connected</h2>
      <p>Choose a repository to monitor its project skills.</p>
      <button onclick={choose}>Choose project folder…</button>
    </div>{/if}
</section>
{#if selected}<Dialog
    title={`Add ${basename(selected)}`}
    onclose={() => (selected = "")}
    ><p class="muted">
      Choose the relative content folder inside this repository.
    </p>
    <label
      >Skill content folder
      <div class="form-row">
        <input readonly value={source} /><button
          onclick={async () => {
            try {
              source =
                (await api.command<string | null>("choose_project_source", {
                  root: selected,
                })) ?? source;
            } catch (e) {
              onerror(e);
            }
          }}>Choose content folder…</button
        >
      </div></label
    >
    <footer class="dialog-actions">
      <button class="primary" onclick={add}>Add project</button><button
        onclick={() => (selected = "")}>Cancel</button
      >
    </footer></Dialog
  >{/if}
{#if removing}<Dialog
    title="Stop monitoring this project?"
    onclose={() => (removing = null)}
    ><p>
      {basename(removing)} will leave this workspace. Its source files remain in the
      repository.
    </p>
    <footer class="dialog-actions">
      <button class="primary" onclick={remove}>Remove project</button><button
        onclick={() => (removing = null)}>Cancel</button
      >
    </footer></Dialog
  >{/if}
