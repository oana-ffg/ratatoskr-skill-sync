<script lang="ts">
  import { agentName, title } from "../api";
  let {
    label,
    options,
    selected = [],
    dimension,
    onchange,
  }: {
    label: string;
    options: string[];
    selected: string[];
    dimension: string;
    onchange: (selected: string[]) => void;
  } = $props();
</script>

<div class="chip-row" role="group" aria-label={label}>
  <span class="chip-label">{label}</span>
  <div class="chips">
    {#each options as option}<button
        type="button"
        class="chip {dimension === 'agent' ? option : dimension}"
        aria-pressed={selected.includes(option)}
        onclick={() =>
          onchange(
            selected.includes(option)
              ? selected.filter((v) => v !== option)
              : [...selected, option],
          )}>{dimension === "agent" ? agentName(option) : title(option)}</button
      >{/each}
  </div>
</div>
