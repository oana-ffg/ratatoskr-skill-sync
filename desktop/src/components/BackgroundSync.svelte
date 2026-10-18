<script lang="ts">
  import { onMount } from "svelte";
  import { api, relativeTime, type BackgroundStatus } from "../api";
  let { onerror }: { onerror: (error: unknown) => void } = $props();
  let status = $state<BackgroundStatus | null>(null);
  let seconds = $state(60);
  let busy = $state(false);
  let loaded = false;
  async function refresh() {
    status = await api.command<BackgroundStatus | null>("background_status");
    if (status && !loaded) {
      seconds = status.interval_seconds;
      loaded = true;
    }
  }
  onMount(() => {
    void refresh().catch(onerror);
    const timer = setInterval(() => {
      void refresh().catch(onerror);
    }, 2000);
    return () => clearInterval(timer);
  });
  async function save(paused: boolean) {
    busy = true;
    try {
      await api.command("set_background_preferences", {
        paused,
        intervalSeconds: seconds,
      });
      await refresh();
    } catch (error) {
      onerror(error);
    } finally {
      busy = false;
    }
  }
  async function migrate() {
    busy = true;
    try {
      await api.command("migrate_background");
      await refresh();
    } catch (error) {
      onerror(error);
    } finally {
      busy = false;
    }
  }
</script>

{#if status}
  <article class="panel" aria-label="Tray sync">
    <div class="section-heading">
      <h2>Background sync</h2>
      <span
        class="status-pill {status.error || status.migration_error
          ? 'warning'
          : 'success'}"
      >
        {status.migration_error
          ? "Blocked"
          : status.paused
            ? "Paused"
            : status.running
              ? "Syncing"
              : "Active"}
      </span>
    </div>
    <p>Tray mode · Recommended on Windows</p>
    <p class="muted">
      Closing the window keeps sync running. Quit from the tray to stop.
    </p>
    <p class="muted">
      Last success: {relativeTime(status.last_success)} · {status.skills} skills
    </p>
    {#if status.migration_error}<p class="error-message" role="alert">
        {status.migration_error}
      </p>
      <button disabled={busy} onclick={migrate}>Retry task migration</button>
    {:else if status.error}<p class="error-message" role="alert">
        {status.error}
      </p>{/if}
    <div class="form-row">
      <label
        >Sync interval (seconds)<input
          type="number"
          min="60"
          max="86400"
          step="1"
          bind:value={seconds}
        /></label
      >
      <button
        disabled={busy ||
          !Number.isInteger(seconds) ||
          seconds < 60 ||
          seconds > 86400 ||
          seconds === status.interval_seconds}
        onclick={() => save(status!.paused)}>Save interval</button
      >
      <button disabled={busy} onclick={() => save(!status!.paused)}
        >{status.paused ? "Resume sync" : "Pause sync"}</button
      >
    </div>
  </article>
{/if}
