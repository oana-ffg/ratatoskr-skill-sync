<script lang="ts">
  import { onMount, getContext, type Snippet } from "svelte";
  let {
    title,
    children,
    onclose,
  }: { title: string; children: Snippet; onclose: () => void } = $props();
  const errors = getContext<{ message: () => string } | undefined>(
    "ratatoskr-errors",
  );
  let element: HTMLDialogElement;
  onMount(() => {
    element.showModal();
    return () => element.close();
  });
</script>

<dialog
  bind:this={element}
  oncancel={(e) => {
    e.preventDefault();
    onclose();
  }}
  aria-label={title}
>
  <header class="dialog-heading">
    <h2>{title}</h2>
    <button class="icon-button" aria-label="Close dialog" onclick={onclose}
      >×</button
    >
  </header>
  {@render children()}
  {#if errors?.message()}<p role="alert" class="error-message">
      {errors.message()}
    </p>{/if}
</dialog>
