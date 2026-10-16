<script lang="ts">
  import { onMount } from "svelte";
  import Dialog from "./Dialog.svelte";
  import Chips from "./Chips.svelte";
  import { agentName, title } from "../api";
  let {
    agents,
    machines,
    initial = {},
    onapply,
    onclose,
  }: {
    agents: string[];
    machines: string[];
    initial?: Record<string, string[]>;
    onapply: (selectors: Record<string, string[]>) => void;
    onclose: () => void;
  } = $props();
  let selectors = $state<Record<string, string[]>>({});
  onMount(() => (selectors = structuredClone($state.snapshot(initial))));
  function set(key: string, value: string[]) {
    selectors = { ...selectors, [key]: value };
    if (!value.length) delete selectors[key];
  }
  const sentence = $derived(
    `Shows for: ${selectors.harness?.map(agentName).join(" or ") || "every agent"} on ${selectors.os?.map(title).join(" or ") || "every system"}, ${selectors.machine?.map(title).join(" or ") || "every machine"}.`,
  );
</script>

<Dialog title="Only include this section for…" {onclose}>
  <p class="muted">Choose where this part of the skill appears.</p>
  <Chips
    label="Agents"
    options={agents}
    selected={selectors.harness ?? []}
    dimension="agent"
    onchange={(v) => set("harness", v)}
  />
  <Chips
    label="System"
    options={["macos", "windows", "linux"]}
    selected={selectors.os ?? []}
    dimension="os"
    onchange={(v) => set("os", v)}
  />
  <Chips
    label="Machines"
    options={machines}
    selected={selectors.machine ?? []}
    dimension="machine"
    onchange={(v) => set("machine", v)}
  />
  <p class="condition-summary" aria-live="polite">{sentence}</p>
  <footer class="dialog-actions">
    <button
      class="primary"
      disabled={!Object.keys(selectors).length}
      onclick={() => onapply(selectors)}>Apply condition</button
    ><button onclick={onclose}>Cancel</button>
  </footer>
</Dialog>
