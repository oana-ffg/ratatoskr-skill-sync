<script lang="ts">
  import { onMount } from "svelte";
  import BackgroundSync from "../components/BackgroundSync.svelte";
  import {
    api,
    relativeTime,
    type Snapshot,
    type BackgroundStatus,
  } from "../api";
  import Dialog from "../components/Dialog.svelte";
  let {
    workspace,
    onupdate,
    onerror,
    ondirty,
    onrefresh,
  }: {
    workspace: Snapshot;
    onupdate: (w: Snapshot) => void;
    onerror: (e: unknown) => void;
    ondirty: (v: boolean) => void;
    onrefresh: () => Promise<void>;
  } = $props();
  const jobs = $derived(workspace.jobs);
  let trayMode = $state<boolean | null>(null);
  onMount(() => {
    void api
      .command<BackgroundStatus | null>("background_status")
      .then((status) => (trayMode = status !== null))
      .catch(onerror);
  });
  let seconds = $state(300);
  let schedule = $state<unknown>(null);
  let message = $state("");
  let busy = $state(false);
  $effect(() => ondirty(busy || !!schedule));
  async function action(command: string, args?: Record<string, unknown>) {
    busy = true;
    try {
      message = await api.command<string>(command, args);
      const w = await api.workspace();
      if (w) onupdate(w);
    } catch (e) {
      onerror(e);
    } finally {
      busy = false;
    }
  }
</script>

<section class="page">
  <div class="page-heading">
    <div>
      <h1>Activity</h1>
    </div>
    <button
      disabled={busy}
      onclick={async () => {
        busy = true;
        try {
          onupdate(await api.command<Snapshot>("sync_workspace"));
        } catch (e) {
          onerror(e);
        } finally {
          busy = false;
        }
      }}>Sync now</button
    >
  </div>
  <div class="stats-grid">
    <article class="panel">
      <p class="eyebrow">LAST RUN</p>
      <h2>{relativeTime(jobs.last_run?.finished_at)}</h2>
      <p class="muted">
        {jobs.last_run?.finished_at
          ? "Completed"
          : jobs.last_run
            ? "Completion not recorded"
            : "No runs recorded"}
      </p>
    </article>
    <article class="panel">
      <p class="eyebrow">RESULT</p>
      <h2 class:warning-text={!!jobs.last_run?.error}>
        {jobs.last_run?.error
          ? "Failed"
          : jobs.last_run?.finished_at
            ? "✓ Successful"
            : "Waiting"}
      </h2>
      <p class="muted">
        {jobs.last_run?.error ?? "Background sync status"}
      </p>
    </article>
    <article class="panel">
      <p class="eyebrow">HELPER</p>
      <h2>{workspace.helper ? "✓ Detected" : "Unavailable"}</h2>
      <details>
        <summary>Helper details</summary>
        <p class="path">{workspace.helper ?? workspace.helper_error}</p>
      </details>
    </article>
  </div>
  {#each [jobs.current_error, jobs.helper_update_error, jobs.desktop_update_error, jobs.background_error].filter(Boolean) as error}<p
      role="alert"
      class="error-message"
    >
      {error}
    </p>{/each}
  {#if trayMode}<BackgroundSync
      {onerror}
      onchanged={onrefresh}
    />{:else if trayMode === false}<article class="panel">
      <div class="section-heading">
        <div>
          <h2>Background schedule</h2>
          <p class="muted">
            Uses the detected helper. Review the job before installing it.
          </p>
        </div>
      </div>
      <div class="form-row">
        <label
          >Run every<select bind:value={seconds}
            ><option value={60}>1 minute</option><option value={300}
              >5 minutes</option
            ><option value={900}>15 minutes</option><option value={3600}
              >1 hour</option
            ></select
          ></label
        ><button
          disabled={!workspace.helper || busy}
          onclick={async () => {
            try {
              schedule = await api.command("preview_schedule", { seconds });
            } catch (e) {
              onerror(e);
            }
          }}>Review schedule…</button
        ><button disabled={busy} onclick={() => action("disable_schedule")}
          >Remove schedule</button
        >
      </div>
      {#if message}<p role="status" class="success-message">{message}</p>{/if}
    </article>
  {/if}
  <article class="panel log-panel">
    <div class="section-heading">
      <h2>Recent log</h2>
    </div>
    <pre>{jobs.log || "No log entries yet."}</pre>
  </article>
</section>
{#if schedule}<Dialog
    title="Install background schedule"
    onclose={() => (schedule = null)}
    ><pre>{JSON.stringify(schedule, null, 2)}</pre>
    <footer class="dialog-actions">
      <button
        class="primary"
        onclick={() => {
          schedule = null;
          void action("enable_schedule", { seconds });
        }}>Install schedule</button
      ><button onclick={() => (schedule = null)}>Cancel</button>
    </footer></Dialog
  >{/if}
