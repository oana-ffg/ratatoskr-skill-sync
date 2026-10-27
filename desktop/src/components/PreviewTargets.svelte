<script lang="ts">
  import { agentName, title, type Snapshot } from "../api";
  let {
    targets,
    machine = $bindable(),
    harness = $bindable(),
  }: {
    targets: Snapshot["preview_targets"];
    machine: string;
    harness: string;
  } = $props();
  const machines = $derived([...new Set(targets.map((t) => t.machine))]);
  function changeMachine(value: string) {
    machine = value;
    // Keep the agent when the new machine enables it.
    if (!targets.some((t) => t.machine === machine && t.harness === harness))
      harness = targets.find((t) => t.machine === machine)?.harness ?? "";
  }
</script>

<div class="preview-selects">
  <label
    >Agent<select aria-label="Preview agent" bind:value={harness}
      >{#each targets.filter((t) => t.machine === machine) as t}<option
          value={t.harness}>{agentName(t.harness)}</option
        >{/each}</select
    ></label
  ><label
    >Machine<select
      aria-label="Preview machine"
      value={machine}
      onchange={(e) => changeMachine(e.currentTarget.value)}
      >{#each machines as m}<option value={m}>{title(m)}</option>{/each}</select
    ></label
  >
</div>
