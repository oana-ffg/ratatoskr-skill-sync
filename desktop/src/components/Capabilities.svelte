<script lang="ts">
  import { title } from "../api";
  let {
    selected,
    onchange,
  }: { selected: string[]; onchange: (values: string[]) => void } = $props();
  let name = $state("");
  function add() {
    const value = name.trim();
    if (!value || selected.includes(value)) return;
    onchange([...selected, value]);
    name = "";
  }
</script>

<div class="chip-row" role="group" aria-label="Capabilities">
  <span class="chip-label">Capabilities</span>
  <div class="chips">
    {#each selected as capability}<button
        type="button"
        class="chip machine"
        aria-label={`Remove ${title(capability)} capability`}
        onclick={() =>
          onchange(selected.filter((value) => value !== capability))}
        >{title(capability)} ×</button
      >{/each}
  </div>
</div>
<form
  class="form-row"
  onsubmit={(event) => {
    event.preventDefault();
    add();
  }}
>
  <label>New capability<input bind:value={name} /></label><button
    disabled={!name.trim() || selected.includes(name.trim())}
    >Add capability</button
  >
</form>
