<script lang="ts">
  import type MarkdownEditor from "./MarkdownEditor.svelte";
  let {
    editor,
    raw = $bindable(),
    ononly,
    onvalue,
  }: {
    editor: MarkdownEditor | undefined;
    raw: boolean;
    ononly: () => void;
    onvalue: () => void;
  } = $props();
</script>

<div class="format-toolbar" role="toolbar" aria-label="Markdown formatting">
  <button aria-label="Bold" onclick={() => editor?.format("bold")}
    ><b>B</b></button
  ><button aria-label="Heading" onclick={() => editor?.format("heading")}
    >H2</button
  ><button aria-label="List" onclick={() => editor?.format("list")}>☷</button
  ><button aria-label="Code" onclick={() => editor?.format("code")}>‹/›</button
  ><span class="toolbar-divider"></span><button
    class="accent-text"
    onclick={ononly}>＋ Only for…</button
  ><button class="accent-text" onclick={onvalue}>＋ Insert value…</button
  ><button class="source-toggle" aria-pressed={raw} onclick={() => (raw = !raw)}
    >Markdown source</button
  >
</div>
