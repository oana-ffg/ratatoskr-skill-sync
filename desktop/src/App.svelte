<script lang="ts">
  import { onMount, setContext } from "svelte";
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
  } from "./api";
  import { runNightlyUpdate, runStartupUpdate } from "./update-policy";
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
  function update(w: Snapshot) {
    workspace = w;
  }
  async function refresh() {
    const w = await api.workspace();
    workspace = w;
  }
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
  onMount(() => {
    let cleanup: (() => void) | undefined;
    void runStartupUpdate(
      async () => {
        if (isTauri()) await api.command("startup_update");
      },
      async () => {
        try {
          await refresh();
        } finally {
          ready = true;
        }
      },
    ).catch(fail);
    const interval = setInterval(() => {
      if (!isTauri() || !workspace || busy || !ready) return;
      void runNightlyUpdate(
        () => api.command<boolean>("nightly_update", { install: false }),
        () => !dirty && !busy && !pending,
        async () => {
          busy = true;
          try {
            await api.command("nightly_update", { install: true });
          } finally {
            busy = false;
          }
        },
      ).catch(fail);
    }, 60000);
    if (isTauri())
      void getCurrentWindow()
        .onCloseRequested((event) => {
          if (dirty) {
            event.preventDefault();
            closeAfterDiscard = true;
            pending = () => {
              void getCurrentWindow().destroy().catch(fail);
            };
          }
        })
        .then((fn) => (cleanup = fn))
        .catch(fail);
    function beforeUnload(e: BeforeUnloadEvent) {
      if (dirty) {
        e.preventDefault();
        e.returnValue = "";
      }
    }
    window.addEventListener("beforeunload", beforeUnload);
    return () => {
      clearInterval(interval);
      cleanup?.();
      window.removeEventListener("beforeunload", beforeUnload);
    };
  });
  async function connect() {
    busy = true;
    try {
      workspace = await api.command<Snapshot>("connect_workspace", {
        source,
        machine,
        create,
        remote: remote || null,
        cloneName: !create && remote ? cloneName : null,
      });
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
        <span class="live-dot" class:failing={!!workspace?.jobs.current_error}
        ></span><span
          >{workspace?.jobs.current_error
            ? "Sync needs attention"
            : workspace?.jobs.last_run?.finished_at
              ? `Synced ${relativeTime(workspace.jobs.last_run.finished_at).toLowerCase()}`
              : "Ready to sync"}</span
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
          </div>{:else if !workspace}<section class="page onboarding">
            <p class="eyebrow">WELCOME TO RATATOSKR</p>
            <h1>Your knowledge.<br />Every agent.</h1>
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
                  void refresh().catch(fail);
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
            {workspace}
            onupdate={update}
            onerror={fail}
            ondirty={(v) => (dirty = v)}
          />{:else if screen === "Settings"}<Settings
            bind:this={settings}
            onunsaved={(value) => (unsaved = value)}
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
