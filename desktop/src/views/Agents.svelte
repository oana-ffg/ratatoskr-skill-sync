<script lang="ts">
  import { onMount } from "svelte";
  import {
    api,
    agentName,
    title,
    type Snapshot,
    type StructuredSource,
    type Table,
    type Preset,
    type AgentTarget,
  } from "../api";
  import Chips from "../components/Chips.svelte";
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
  let agents = $state<StructuredSource | null>(null);
  let profiles = $state<StructuredSource[]>([]);
  let originals = $state("");
  let adding = $state<"agent" | "machine" | null>(null);
  let target = $state<AgentTarget>({
    tag: "",
    global_folder: "",
    project_folder: "",
    instructions_file: "",
    instructions_folder: null,
  });
  let agentPreview = $state<{ text: string; hash: string } | null>(null);
  let newMachine = $state("");
  let busy = $state(false);
  let valueNames = $state<Record<string, string>>({});
  const serialized = $derived(JSON.stringify([agents, profiles]));
  const dirty = $derived(!!originals && serialized !== originals);
  $effect(() => ondirty(dirty || adding !== null || busy));
  onMount(() => {
    void load();
  });
  async function load() {
    try {
      const a = await api.source("harnesses.toml");
      const p = await Promise.all(
        workspace.source_files
          .filter((f) => f.startsWith("machines/"))
          .map(api.source),
      );
      agents = a;
      profiles = p;
      originals = JSON.stringify([a, p]);
    } catch (e) {
      onerror(e);
    }
  }
  export async function discard() {
    adding = null;
    await load();
  }
  export async function save() {
    busy = true;
    try {
      if (
        agents &&
        JSON.stringify(agents) !== JSON.stringify(JSON.parse(originals)[0])
      )
        onupdate(await api.saveSource(agents));
      for (const profile of profiles) {
        const old = (JSON.parse(originals)[1] as StructuredSource[]).find(
          (p) => p.file === profile.file,
        );
        if (JSON.stringify(profile) !== JSON.stringify(old))
          onupdate(await api.saveSource(profile));
      }
      await load();
    } finally {
      busy = false;
    }
  }
  function preset(p: Preset) {
    target = structuredClone(p.target);
    agentPreview = null;
  }
  async function previewAgent() {
    try {
      agentPreview = await api.command("preview_agent", { target });
    } catch (e) {
      onerror(e);
    }
  }
  async function addAgent() {
    if (!agentPreview) return;
    try {
      onupdate(
        await api.command<Snapshot>("add_agent_target", {
          target,
          expectedHash: agentPreview.hash,
        }),
      );
      adding = null;
      await load();
    } catch (e) {
      onerror(e);
    }
  }
  async function addMachine() {
    try {
      const file = `machines/${newMachine}.toml`;
      const source = await api.source(file);
      source.values = {
        os: "macos",
        mode: "staging",
        harnesses: [Object.keys(agents?.values ?? {})[0]],
        capabilities: [],
        paths: {},
      };
      profiles = [...profiles, source];
      adding = null;
    } catch (e) {
      onerror(e);
    }
  }
  function machineName(profile: StructuredSource) {
    return profile.file.slice(9, -5);
  }
  function array(profile: StructuredSource, key: string) {
    return (profile.values[key] ?? []) as string[];
  }
  function paths(profile: StructuredSource) {
    if (!profile.values.paths) profile.values.paths = {};
    return profile.values.paths as Table;
  }
  function definition(tag: string) {
    return agents!.values[tag] as Table;
  }
</script>

<section class="page">
  <div class="page-heading">
    <div>
      <p class="eyebrow">A PLACE FOR EVERY AGENT</p>
      <h1>Agents & machines</h1>
      <p class="muted">
        Choose who receives your skills, and what each machine can do.
      </p>
    </div>
  </div>
  <div class="section-heading">
    <h2>Agents</h2>
    <button
      disabled={dirty}
      onclick={() => {
        adding = "agent";
        agentPreview = null;
      }}>＋ Add agent</button
    >
  </div>
  <div class="agent-grid">
    {#if agents}{#each Object.keys(agents.values) as tag}<article
          class="panel agent-card"
        >
          <div class="section-heading">
            <h2><span class="agent-mark {tag}">◆</span> {agentName(tag)}</h2>
            <span class="status-pill">{tag}</span>
          </div>
          <p class="muted">
            Enabled on {profiles.filter((p) =>
              array(p, "harnesses").includes(tag),
            ).length} machines
          </p>
          <details>
            <summary>Agent folders & instructions</summary><label
              >Skills folder<input
                value={definition(tag).skills_dir as string}
                oninput={(e) =>
                  (definition(tag).skills_dir = e.currentTarget.value)}
              /></label
            ><label
              >Project folder<input
                value={definition(tag).project_root as string}
                oninput={(e) =>
                  (definition(tag).project_root = e.currentTarget.value)}
              /></label
            ><label
              >Instructions filename<input
                value={definition(tag).global_file as string}
                oninput={(e) =>
                  (definition(tag).global_file = e.currentTarget.value)}
              /></label
            >{#if definition(tag).global_root !== undefined}<label
                >Global instructions folder<input
                  value={definition(tag).global_root as string}
                  oninput={(e) =>
                    (definition(tag).global_root = e.currentTarget.value)}
                /></label
              >{/if}{#each Object.keys((definition(tag).roots ?? {}) as Table) as os}<label
                >{title(os)} global folder<input
                  value={(definition(tag).roots as Table)[os] as string}
                  oninput={(e) =>
                    ((definition(tag).roots as Table)[os] =
                      e.currentTarget.value)}
                /></label
              >{/each}
          </details>
        </article>{/each}{/if}
  </div>
  <div class="section-heading machine-heading">
    <h2>Machine profiles</h2>
    <button
      disabled={dirty}
      onclick={() => {
        adding = "machine";
        newMachine = "";
      }}>＋ Add machine</button
    >
  </div>
  {#each profiles as profile}<article class="panel machine-card">
      <div class="section-heading">
        <h2>
          <span class="machine-symbol">▣</span>
          {title(machineName(profile))}
        </h2>
        <span
          class="status-pill {profile.values.mode === 'live' ? 'warning' : ''}"
          >{profile.values.mode === "live" ? "Live" : "Staging"}{machineName(
            profile,
          ) === workspace.plan.machine
            ? " · This machine"
            : ""}</span
        >
      </div>
      <div class="form-row">
        <label
          >System<select bind:value={profile.values.os}
            ><option value="macos">macOS</option><option value="windows"
              >Windows</option
            ><option value="linux">Linux</option></select
          ></label
        ><label
          >Install mode<select bind:value={profile.values.mode}
            ><option value="staging">Staging</option><option value="live"
              >Live</option
            ></select
          ></label
        ><label>Home folder<input bind:value={profile.values.home} /></label>
      </div>
      <Chips
        label="Agents"
        options={Object.keys(agents?.values ?? {})}
        selected={array(profile, "harnesses")}
        dimension="agent"
        onchange={(v) => (profile.values.harnesses = v)}
      />
      <label
        >Capabilities <small>Separate names with commas</small><input
          value={array(profile, "capabilities").join(", ")}
          onchange={(e) =>
            (profile.values.capabilities = e.currentTarget.value
              .split(",")
              .map((s) => s.trim())
              .filter(Boolean))}
        /></label
      >
      <div class="section-heading">
        <h3>
          Values <span class="muted small">Named paths for this machine</span>
        </h3>
      </div>
      {#each Object.keys((profile.values.paths ?? {}) as Table) as key}<div
          class="key-value-row"
        >
          <label
            >{title(key)}<input
              value={paths(profile)[key] as string}
              oninput={(e) => (paths(profile)[key] = e.currentTarget.value)}
            /></label
          ><button
            class="subtle"
            aria-label={`Remove ${title(key)} value from ${machineName(profile)}`}
            onclick={() => {
              delete paths(profile)[key];
            }}>Remove</button
          >
        </div>{/each}
      <div class="form-row">
        <label
          >New value name<input bind:value={valueNames[profile.file]} /></label
        ><button
          disabled={!valueNames[profile.file]}
          onclick={() => {
            const key = valueNames[profile.file];
            if (Object.hasOwn(paths(profile), key)) {
              onerror("That value already exists.");
              return;
            }
            paths(profile)[key] = "";
            valueNames[profile.file] = "";
          }}>Add value</button
        >
      </div>
    </article>{/each}
</section>
{#if adding === "agent"}<Dialog
    title="Add an agent"
    onclose={() => (adding = null)}
    ><label
      >Start with a preset<select
        onchange={(e) => {
          const p = workspace.presets.find(
            (p) => p.target.tag === e.currentTarget.value,
          );
          if (p) preset(p);
        }}
        ><option value="">Custom agent</option
        >{#each workspace.presets as p}<option value={p.target.tag}
            >{p.title}</option
          >{/each}</select
      ></label
    >
    <div oninput={() => (agentPreview = null)}>
      <label>Agent tag<input bind:value={target.tag} /></label><label
        >Global folder<input bind:value={target.global_folder} /></label
      ><label>Project folder<input bind:value={target.project_folder} /></label
      ><label
        >Instructions filename<input
          bind:value={target.instructions_file}
        /></label
      ><label
        >Separate instructions folder<input
          value={target.instructions_folder ?? ""}
          oninput={(e) =>
            (target.instructions_folder = e.currentTarget.value || null)}
        /></label
      >
    </div>
    {#if agentPreview}<p class="success-message">
        Definition validated. Enable this agent in the intended machine profiles
        after adding it.
      </p>
      <details>
        <summary>Source preview</summary>
        <pre>{agentPreview.text}</pre>
      </details>{/if}
    <footer class="dialog-actions">
      {#if agentPreview}<button class="primary" onclick={addAgent}
          >Add agent</button
        >{:else}<button class="primary" onclick={previewAgent}
          >Review agent</button
        >{/if}<button onclick={() => (adding = null)}>Cancel</button>
    </footer></Dialog
  >{/if}
{#if adding === "machine"}<Dialog
    title="Add a machine profile"
    onclose={() => (adding = null)}
    ><label>Machine identifier<input bind:value={newMachine} /></label>
    <p class="muted">
      The profile starts in staging. Fill in its system, agents and values, then
      save.
    </p>
    <footer class="dialog-actions">
      <button
        class="primary"
        disabled={!/^[a-z0-9]+(-[a-z0-9]+)*$/.test(newMachine)}
        onclick={addMachine}>Create profile</button
      ><button onclick={() => (adding = null)}>Cancel</button>
    </footer></Dialog
  >{/if}
