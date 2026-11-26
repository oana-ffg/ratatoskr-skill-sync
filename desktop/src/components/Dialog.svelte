<script lang="ts">
  import { onMount, getContext, type Snippet } from "svelte";
  let {
    title,
    children,
    onclose,
    wide = false,
    showError = true,
  }: {
    title: string;
    children: Snippet;
    onclose: () => void;
    wide?: boolean;
    showError?: boolean | ((message: string) => boolean);
  } = $props();
  const errors = getContext<{ message: () => string } | undefined>(
    "ratatoskr-errors",
  );
  const visibleError = $derived.by(() => {
    const message = errors?.message() ?? "";
    if (!message) return "";
    return typeof showError === "function"
      ? showError(message)
        ? message
        : ""
      : showError
        ? message
        : "";
  });
  let element: HTMLDialogElement;
  onMount(() => {
    element.showModal();
    return () => element.close();
  });
</script>

<dialog
  bind:this={element}
  class:wide
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
  {#if visibleError}<p role="alert" class="error-message">
      {visibleError}
    </p>{/if}
</dialog>
