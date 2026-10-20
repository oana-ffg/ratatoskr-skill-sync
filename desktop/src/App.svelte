<script lang="ts">
  import { onMount, setContext, untrack } from "svelte";
  import { isTauri } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import {
    api,
    title,
    basename,
    errorText,
    relativeTime,
    reviewSummary,
    type Snapshot,
    type Skill,
    type BackgroundStatus,
  } from "./api";
  import { watchWindow } from "./window-refresh";
  import { runStartupUpdate } from "./update-policy";
  import { listen } from "@tauri-apps/api/event";
  import Library from "./views/Library.svelte";
  import type SkillEditor from "./views/SkillEditor.svelte";
  let editorModule: ReturnType<typeof importEditor> | undefined;
  const importEditor = () => import("./views/SkillEditor.svelte");
  function loadEditor() {
    return (editorModule ??= importEditor().catch((e) => {
      editorModule = undefined;
      throw e;
    }));
  }
  import Review from "./views/Review.svelte";
  import Projects from "./views/Projects.svelte";
  import Agents from "./views/Agents.svelte";
  import Activity from "./views/Activity.svelte";
  import Settings from "./views/Settings.svelte";
  import Dialog from "./components/Dialog.svelte";
  type Screen =
    | "Skills"
    | "Review changes"
    | "Projects"
    | "Agents & machines"
    | "Activity"
    | "Settings";
  let workspace = $state<Snapshot | null>(null);
  let ready = $state(false);
  let trayMode = $state(false);
  let screen = $state<Screen>("Skills");
  let skill = $state<Skill | null>(null);
  let scope = $state("all");
  let dirty = $state(false);
  let unsaved = $state(false);
  let epoch = $state(0);
  let busy = $state(false);
  let error = $state("");
  setContext("ratatoskr-errors", { message: () => error });
  let pending = $state<(() => void) | null>(null);
  let closeAfterDiscard = $state(false);
  let editor = $state<SkillEditor>();
  let agents = $state<Agents>();
  let settings = $state<Settings>();
  let source = $state("");
  let machine = $state("");
  let create = $state(false);
  let remote = $state("");
  let cloneName = $state("");
  const items: { name: Screen; icon: string }[] = [
    { name: "Skills", icon: "◇" },
    { name: "Review changes", icon: "⇄" },
    { name: "Projects", icon: "▱" },
    { name: "Agents & machines", icon: "◈" },
    { name: "Activity", icon: "↗" },
    { name: "Settings", icon: "⚙" },
  ];
  function fail(e: unknown) {
    error = errorText(e);
  }
  let snapshotAt = $state<number | null>(null);
  let now = $state(Date.now());
  let refreshError = $state("");
  let deferred = $state(false);
  let windowVisible = $state(false);
  let refreshing: Promise<void> | undefined;
  let revision = 0;
  let disposed = false;
  const protectedState = () => dirty || unsaved || busy || !!pending;
  function update(w: Snapshot | null, asOf = Date.now()) {
    revision++;
    workspace = w;
    snapshotAt = asOf;
    refreshError = "";
    deferred = false;
    now = Date.now();
  }
  function refresh(): Promise<void> {
    if (protectedState()) {
      deferred = true;
      return Promise.resolve();
    }
    if (refreshing) return refreshing;
    deferred = false;
    const current = revision;
    const started = Date.now();
    refreshing = (async () => {
      try {
        const w = await api.workspace();
        if (disposed || current !== revision) return;
        // An edit or mutation can begin while IPC is in flight.
        if (protectedState()) deferred = true;
        else update(w, started);
      } catch (e) {
        if (!disposed && current === revision) refreshError = errorText(e);
      } finally {
        refreshing = undefined;
      }
    })();
    return refreshing;
  }
  $effect(() => {
    if (ready && windowVisible && deferred && !protectedState())
      untrack(() => void refresh());
  });
  const snapshotAge = $derived.by(() => {
    // Keep relative labels advancing even when refreshes fail or are deferred.
    void now;
    return snapshotAt === null
      ? "No good snapshot yet"
      : `As of ${relativeTime(snapshotAt / 1000).toLowerCase()}`;
  });
  function navigate(action: () => void) {
    const leave = () => {
      unsaved = false;
      action();
    };
    if (dirty) pending = leave;
    else leave();
  }
  function open(s: Skill) {
    navigate(() => {
      skill = s;
      screen = "Skills";
      dirty = false;
    });
  }
  async function discard() {
    try {
      if (skill) await editor?.discard();
      else if (screen === "Agents & machines") await agents?.discard();
      else if (screen === "Settings") await settings?.discard();
      else epoch++;
      dirty = false;
    } catch (e) {
      fail(e);
    }
  }
  async function save() {
    busy = true;
    try {
      if (skill) await editor?.save();
      else if (screen === "Agents & machines") await agents?.save();
      else if (screen === "Settings") await settings?.save();
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
  }
  $effect(() => {
    if (isTauri() && ready)
      void api
        .command("set_desktop_busy", { busy: dirty || busy || !!pending })
        .catch(fail);
  });
  onMount(() => {
    let cleanup: (() => void) | undefined;
    void runStartupUpdate(
      async () => {
        if (isTauri()) {
          trayMode =
            (await api.command<BackgroundStatus | null>(
              "background_status",
            )) !== null;
          await api.command("startup_update");
        }
      },
      async () => {
        try {
          await refresh();
        } finally {
          ready = true;
        }
      },
    ).catch(fail);
    let stopUpdateErrors: (() => void) | undefined;
    const ageTimer = setInterval(() => (now = Date.now()), 10_000);
    const stopRefresh = watchWindow(
      () => {
        if (ready) void refresh();
      },
      (e) => fail(`Automatic workspace refresh: ${errorText(e)}`),
      { onVisibility: (visible) => (windowVisible = visible) },
    );
    if (isTauri())
      void listen<string | null>("desktop-update-error", (event) => {
        if (event.payload) fail(event.payload);
      })
        .then((stop) => {
          if (disposed) stop();
          else stopUpdateErrors = stop;
        })
        .catch(fail);
    if (isTauri())
      void getCurrentWindow()
        .onCloseRequested((event) => {
          if (trayMode) return;
          if (dirty) {
            event.preventDefault();
            closeAfterDiscard = true;
            pending = () => {
              void getCurrentWindow().destroy().catch(fail);
            };
          }
        })
        .then((stop) => {
          if (disposed) stop();
          else cleanup = stop;
        })
        .catch(fail);
    function beforeUnload(e: BeforeUnloadEvent) {
      if (!trayMode && dirty) {
        e.preventDefault();
        e.returnValue = "";
      }
    }
    window.addEventListener("beforeunload", beforeUnload);
    return () => {
      disposed = true;
      clearInterval(ageTimer);
      stopRefresh();
      stopUpdateErrors?.();
      cleanup?.();
      window.removeEventListener("beforeunload", beforeUnload);
    };
  });
  async function connect() {
    busy = true;
    try {
      update(
        await api.command<Snapshot>("connect_workspace", {
          source,
          machine,
          create,
          remote: remote || null,
          cloneName: !create && remote ? cloneName : null,
        }),
      );
    } catch (e) {
      fail(e);
    } finally {
      busy = false;
    }
  }
</script>

<div class="app-shell">
  <aside class="sidebar">
    <div class="brand">
      <img src="/ratatoskr.png" alt="" />
      <div><strong>Ratatoskr</strong><span>SKILL SYNC</span></div>
    </div>
    <nav aria-label="Main navigation">
      {#each items as item}<button
          class:active={screen === item.name}
          aria-current={screen === item.name ? "page" : undefined}
          onclick={() =>
            navigate(() => {
              screen = item.name;
              skill = null;
              dirty = false;
            })}
          ><span class="nav-icon">{item.icon}</span
          >{item.name}{#if item.name === "Skills" && workspace}<span
              class="nav-count">{workspace.skills.length}</span
            >{:else if item.name === "Review changes" && workspace && reviewSummary(workspace).total}<span
              class="nav-count review-count"
              >{reviewSummary(workspace).total}</span
            >{/if}</button
        >{/each}
    </nav>
    <footer class="sidebar-footer">
      <div class="machine-avatar">
        {workspace?.plan.machine[0]?.toUpperCase() ?? "R"}
      </div>
      <div>
        <strong
          >{workspace
            ? title(workspace.plan.machine)
            : "Local workspace"}</strong
        ><span>{workspace ? "This machine" : "Not connected"}</span>
      </div>
      <i class="live-dot"></i>
    </footer>
  </aside>
  <div class="workspace">
    <header class="topbar">
      <div class="sync-indicator">
        <span
          class="live-dot"
          class:failing={!!workspace?.jobs.current_error ||
            !!refreshError ||
            deferred}
        ></span><span
          >{refreshError || deferred
            ? "Status may be out of date"
            : !workspace
              ? "Status unavailable"
              : workspace.jobs.current_error
                ? "Sync needs attention"
                : workspace?.jobs.last_run?.finished_at
                  ? `Synced ${relativeTime(workspace.jobs.last_run.finished_at).toLowerCase()}`
                  : "Ready to sync"}</span
        ><small
          class="snapshot-age"
          title={snapshotAt === null
            ? undefined
            : new Date(snapshotAt).toLocaleString()}>{snapshotAge}</small
        >
      </div>
      <span class="mode-badge"
        >{workspace?.plan.mode === "live" ? "Live" : "Staging"}</span
      >
      <div class="breadcrumb">
        <span
          >{skill
            ? skill.project
              ? basename(skill.project)
              : "Global"
            : screen}</span
        >{#if skill}<span>›</span><strong>{skill.name}</strong>{/if}
      </div>
      <div class="top-actions">
        {#if unsaved && (skill || screen === "Agents & machines" || screen === "Settings")}<span
            class="unsaved-indicator">Unsaved changes</span
          ><button disabled={busy} onclick={discard}>Discard</button><button
            class="primary"
            disabled={busy}
            onclick={save}>{busy ? "Saving…" : "Save"}</button
          >{/if}
      </div>
    </header>
    {#if refreshError || deferred}<div
        class="global-error"
        role={refreshError ? "alert" : "status"}
      >
        <div>
          <strong
            >{refreshError
              ? "Workspace refresh failed"
              : "Refresh paused while editing"}</strong
          >
          <p>
            {refreshError ||
              "Save or discard your changes to refresh. The displayed data may be out of date."}
            {snapshotAge}.{#if snapshotAt !== null}{" "}Last good snapshot: {new Date(
                snapshotAt,
              ).toLocaleString()}.{/if}
          </p>
        </div>
        <button
          disabled={dirty || unsaved || busy || !!pending}
          onclick={() => void refresh()}>Retry refresh</button
        >
      </div>{/if}
    {#if error}<div class="global-error" role="alert">
        <div>
          <strong>Something needs attention</strong>
          <p>{error}</p>
        </div>
        <button aria-label="Dismiss error" onclick={() => (error = "")}
          >×</button
        >
      </div>{/if}
    <main class="main-content" aria-busy={!ready || busy} inert={busy}>
      {#key epoch}
        {#if !ready}<div class="empty-state">
            Opening workspace…
          </div>{:else if screen === "Settings"}<Settings
            bind:this={settings}
            onunsaved={(value) => (unsaved = value)}
            {workspace}
            onupdate={update}
            onerror={fail}
            ondirty={(v) => (dirty = v)}
          />{:else if !workspace}<section class="page onboarding">
            <h1>Connect a workspace</h1>
            <p class="muted">
              Connect a content repository or create one to start sharing
              skills.
            </p>
            <label
              >Content folder
              <div class="form-row">
                <input readonly value={source} /><button
                  onclick={async () => {
                    try {
                      source = (await api.folder()) ?? source;
                    } catch (e) {
                      fail(e);
                    }
                  }}>Choose folder…</button
                >
              </div></label
            ><label>Machine identifier<input bind:value={machine} /></label
            ><label class="checkbox-label"
              ><input type="checkbox" bind:checked={create} />Create a new
              content repository</label
            >{#if !create}<label
                >Git remote (optional)<input bind:value={remote} /></label
              >{#if remote}<label
                  >New checkout folder name<input
                    bind:value={cloneName}
                  /></label
                >
                <p class="muted small">
                  The chosen content folder is the parent for this new checkout.
                </p>{/if}{/if}<button
              class="primary"
              disabled={!source ||
                !machine ||
                busy ||
                (!create && !!remote && !cloneName)}
              onclick={connect}>Connect workspace</button
            >{#if error}<button
                onclick={() => {
                  void refresh();
                }}>Retry loading workspace</button
              >{/if}
          </section>{:else if screen === "Skills"}{#if skill}{#key `${skill.project}:${skill.name}`}{#await loadEditor()}
                <p class="empty-state">Opening editor…</p>
              {:then { default: SkillEditor }}<SkillEditor
                  bind:this={editor}
                  {skill}
                  {workspace}
                  onupdate={update}
                  onerror={fail}
                  ondirty={(v) => (dirty = v)}
                  onrenamed={(s) => (skill = s)}
                  onunsaved={(value) => (unsaved = value)}
                />{:catch e}<p role="alert" class="error-message">
                  Editor failed to load: {errorText(e)}
                </p>{/await}{/key}{:else}<Library
              {workspace}
              initialScope={scope}
              onopen={open}
              onupdate={update}
              onerror={fail}
              ondirty={(v) => (dirty = v)}
            />{/if}{:else if screen === "Review changes"}<Review
            {workspace}
            onupdate={update}
            onerror={fail}
            ondirty={(v) => (dirty = v)}
          />{:else if screen === "Projects"}<Projects
            {workspace}
            onupdate={update}
            onerror={fail}
            ondirty={(v) => (dirty = v)}
            onbrowse={(root) => {
              scope = root;
              screen = "Skills";
            }}
          />{:else if screen === "Agents & machines"}<Agents
            bind:this={agents}
            onunsaved={(value) => (unsaved = value)}
            {workspace}
            onupdate={update}
            onerror={fail}
            ondirty={(v) => (dirty = v)}
          />{:else if screen === "Activity"}<Activity
            onrefresh={refresh}
            {workspace}
            onupdate={update}
            onerror={fail}
            ondirty={(v) => (dirty = v)}
          />{/if}
      {/key}
    </main>
  </div>
</div>
{#if pending}<Dialog
    title="You have unsaved changes"
    onclose={() => {
      pending = null;
      closeAfterDiscard = false;
    }}
    ><p>
      Save your work first, or discard your changes to {closeAfterDiscard
        ? "close Ratatoskr"
        : "leave this screen"}.
    </p>
    <footer class="dialog-actions">
      <button
        class="primary"
        onclick={() => {
          const action = pending;
          pending = null;
          dirty = false;
          action?.();
        }}>Discard and continue</button
      ><button
        onclick={() => {
          pending = null;
          closeAfterDiscard = false;
        }}>Keep editing</button
      >
    </footer></Dialog
  >{/if}
