<script lang="ts">
  import { onMount } from "svelte";
  import {
    api,
    agentName,
    basename,
    errorText,
    type Snapshot,
    type Skill,
  } from "../api";
  import Dialog from "../components/Dialog.svelte";
  let {
    workspace,
    onopen,
    onupdate,
    onerror,
    ondirty,
    initialScope = "all",
  }: {
    initialScope?: string;
    workspace: Snapshot;
    onopen: (skill: Skill) => void;
    onupdate: (w: Snapshot) => void;
    onerror: (e: unknown) => void;
    ondirty: (v: boolean) => void;
  } = $props();
  let search = $state("");
  let scope = $state("all");
  let creating = $state(false);
  let newName = $state("");
  let description = $state("");
  let statuses = $state<Record<string, Record<string, boolean>>>({});
  let statusError = $state("");
  onMount(() => {
    scope = initialScope;
  });
  $effect(() => {
    workspace.skills;
    void api
      .command<Record<string, Record<string, boolean>>>("skill_target_statuses")
      .then((s) => {
        statuses = s;
        statusError = "";
      })
      .catch((e) => (statusError = errorText(e)));
  });
  $effect(() => ondirty(creating));
  const visible = $derived(
    workspace.skills.filter(
      (s) =>
        s.name.toLowerCase().includes(search.toLowerCase()) &&
        (scope === "all" ||
          (scope === "global" && !s.project) ||
          s.project === scope),
    ),
  );
  async function toggle(skill: Skill) {
    try {
      onupdate(
        await api.command<Snapshot>("set_skill_enabled", {
          name: skill.name,
          project: skill.project,
          enabled: !skill.enabled,
        }),
      );
    } catch (e) {
      onerror(e);
    }
  }
  async function create() {
    try {
      const project = scope !== "all" && scope !== "global" ? scope : null;
      onupdate(
        await api.command<Snapshot>("create_desktop_skill", {
          name: newName,
          project,
          description,
        }),
      );
      creating = false;
      onopen({ name: newName, project, enabled: true });
    } catch (e) {
      onerror(e);
    }
  }
</script>

<section class="page">
  <div class="page-heading">
    <div>
      <p class="eyebrow">YOUR SHARED KNOWLEDGE</p>
      <h1>Skills <span class="count">{workspace.skills.length}</span></h1>
      <p class="muted">One source. The right instructions for every agent.</p>
    </div>
    <button class="primary" onclick={() => (creating = true)}
      >＋ New skill</button
    >
  </div>
  <div class="library-tools">
    <label class="search-field"
      ><span>⌕</span><input
        type="search"
        aria-label="Search skills"
        placeholder="Search skills"
        bind:value={search}
      /></label
    ><label
      >Scope<select aria-label="Skill scope" bind:value={scope}
        ><option value="all">All skills</option><option value="global"
          >Global</option
        >{#each workspace.plan.projects as p}<option value={p.root}
            >{basename(p.root)}</option
          >{/each}</select
      ></label
    >
  </div>
  {#if statusError}<p role="alert" class="error-message">
      Target status unavailable: {statusError}
    </p>{/if}
  <div class="skill-list">
    <div class="list-heading">
      <span>SKILL</span><span>SCOPE</span><span>APPLIES TO</span><span
        >ENABLED</span
      >
    </div>
    {#each visible as skill}<div class="skill-row">
        <button class="skill-title" onclick={() => onopen(skill)}
          ><span class="skill-icon">◇</span><span
            ><strong>{skill.name}</strong><small
              >{skill.enabled ? "Ready to share" : "Disabled"}</small
            ></span
          ><span class="row-arrow">↗</span></button
        ><span class="muted"
          >{skill.project ? basename(skill.project) : "Global"}</span
        >
        <div class="target-dots">
          {#each [...new Set(workspace.preview_targets.map((t) => t.harness))] as agent}<span
              class="target-indicator"
              title={`${agentName(agent)}: ${statuses[`${skill.project ?? ""}:${skill.name}`]?.[agent] === undefined ? "checking" : statuses[`${skill.project ?? ""}:${skill.name}`]?.[agent] ? "included" : "excluded"}`}
              ><i
                class="dot {agent}"
                class:off={!statuses[`${skill.project ?? ""}:${skill.name}`]?.[
                  agent
                ]}
              ></i>{agentName(agent)}</span
            >{/each}
        </div>
        <button
          class="toggle"
          role="switch"
          aria-label={`Enable ${skill.name}`}
          aria-checked={skill.enabled}
          onclick={() => toggle(skill)}><span></span></button
        >
      </div>{/each}
  </div>
  {#if !visible.length}<div class="empty-state">
      <h2>
        {workspace.skills.length
          ? "No matching skills"
          : "A home for your skills"}
      </h2>
      <p>Add a skill to share it across your agents and machines.</p>
    </div>{/if}
  <div class="library-footer">
    <span>◈ Source controlled · local first</span><span
      >{workspace.preview_targets.length} agent × machine targets</span
    >
  </div>
</section>
{#if creating}<Dialog title="Create a skill" onclose={() => (creating = false)}
    ><label
      >Name<input
        bind:value={newName}
        pattern="[a-z0-9]+(-[a-z0-9]+)*"
      /></label
    ><label
      >Description<textarea bind:value={description} maxlength="1024"
      ></textarea></label
    >
    <p class="muted">
      Creates an empty Markdown body. Add your instructions in the editor.
    </p>
    <footer class="dialog-actions">
      <button
        class="primary"
        disabled={!/^[a-z0-9]+(-[a-z0-9]+)*$/.test(newName) ||
          !description.trim()}
        onclick={create}>Create skill</button
      ><button onclick={() => (creating = false)}>Cancel</button>
    </footer></Dialog
  >{/if}
