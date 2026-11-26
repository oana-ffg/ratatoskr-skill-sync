<script lang="ts">
  import Dialog from "./Dialog.svelte";
  import Diff from "./Diff.svelte";
  import { unifiedDiff } from "../editor/merge";

  export interface ConflictFile {
    label: string;
    latest: string;
    draft: string;
  }

  let {
    title = "Source changed on disk",
    files,
    canCombine,
    busy = false,
    oncombine,
    onreload,
    onkeepdraft,
    onclose,
  }: {
    title?: string;
    files: ConflictFile[];
    canCombine: boolean;
    busy?: boolean;
    oncombine?: () => void;
    onreload: () => void;
    onkeepdraft: () => void;
    onclose: () => void;
  } = $props();

  const changed = $derived(files.filter((file) => file.latest !== file.draft));
  const identical = $derived(changed.length === 0);
</script>

<Dialog {title} {onclose} wide showError={false}>
  {#if identical}
    <p class="muted">
      Your draft already matches the latest version on disk. Reload to update
      the editor state and clear the warning.
    </p>
  {:else if canCombine}
    <p class="muted">
      This source changed on disk since you opened it. Your edits do not overlap
      the newer lines, so you can combine both into your draft and review before
      saving.
    </p>
  {:else}
    <p class="muted">
      This source changed on disk since you opened it, and both versions
      modified the same section. Review the differences below before choosing
      how to continue.
    </p>
  {/if}

  {#each changed as file}
    {#if changed.length > 1}
      <p class="eyebrow">{file.label}</p>
    {/if}
    <Diff
      diff={unifiedDiff(file.latest, file.draft, file.label, file.label)}
      left="Latest on disk"
      right="Your draft"
      neutral
    />
  {/each}

  <footer class="dialog-actions">
    {#if identical}
      <button class="primary" disabled={busy} onclick={onreload}
        >Reload latest</button
      >
    {:else}
      {#if canCombine && oncombine}
        <button class="primary" disabled={busy} onclick={oncombine}
          >Combine into draft</button
        >
      {/if}
      <button disabled={busy} onclick={onreload}
        >Reload latest (discard draft)</button
      >
      <button disabled={busy} onclick={onkeepdraft}
        >Keep my draft to overwrite</button
      >
    {/if}
    <button disabled={busy} onclick={onclose}>Cancel</button>
  </footer>
</Dialog>
