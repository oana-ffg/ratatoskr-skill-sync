<script lang="ts">
  import { diffRows, collapseContext } from "./diff";
  let {
    diff,
    left = "Your source",
    right = "Installed copy",
    reverse = false,
    neutral = false,
  }: {
    diff: string | null;
    left?: string;
    right?: string;
    reverse?: boolean;
    neutral?: boolean;
  } = $props();
  let expanded = $state(false);
  const rows = $derived(diffRows(diff ?? "", reverse));
  const visible = $derived(collapseContext(rows, expanded));
  const collapsible = $derived(visible.length < rows.length);
</script>

{#if diff}<div class="diff-grid" class:neutral>
    {#each ["left", "right"] as side}<div>
        <h3>{side === "left" ? left : right}</h3>
        <pre>{#each visible as row}{@const cell =
              side === "left" ? row.left : row.right}<div
              class="diff-line"
              class:diff-context={cell.kind === "context"}
              class:diff-remove={!neutral && cell.kind === "remove"}
              class:diff-add={!neutral && cell.kind === "add"}
              class:diff-edited={neutral &&
                (cell.kind === "remove" || cell.kind === "add")}
              class:diff-missing={cell.kind ===
                "missing"}>{#if row.omitted}⋯ {row.omitted} unchanged {row.omitted ===
                1
                  ? "line"
                  : "lines"} ⋯{:else}{#each cell.segments as segment}<span
                    class:word-change={segment.changed &&
                      cell.kind !== "missing"}>{segment.text}</span
                  >{/each}{/if}</div>{/each}</pre>
      </div>{/each}
  </div>
  {#if collapsible && !expanded}<button
      class="subtle"
      onclick={() => (expanded = true)}>Expand available context</button
    >{/if}
{:else}<p class="muted">
    Text comparison unavailable for binary or large files. Review the file
    details before applying.
  </p>{/if}
