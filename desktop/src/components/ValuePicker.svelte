<script lang="ts">
  import Dialog from "./Dialog.svelte";
  import { agentName, title, type ValueOption } from "../api";
  let {
    values,
    machine,
    harness,
    onpick,
    onclose,
  }: {
    values: ValueOption[];
    machine: string;
    harness: string;
    onpick: (name: string) => void;
    onclose: () => void;
  } = $props();
</script>

<Dialog title="Insert a value" {onclose}
  ><p class="muted">Resolved for {title(machine)} · {agentName(harness)}</p>
  <div class="value-options">
    {#each values as value}<button onclick={() => onpick(value.name)}
        ><strong>▣ {title(value.label)}</strong><span>{value.value}</span
        ></button
      >{/each}
  </div></Dialog
>
