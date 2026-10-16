<script lang="ts">
  import { api } from "../api";
  let {
    label,
    value,
    onchange,
    onerror,
  }: {
    label: string;
    value: string;
    onchange: (value: string) => void;
    onerror: (error: unknown) => void;
  } = $props();
  async function choose() {
    try {
      const folder = await api.folder();
      if (folder !== null) onchange(folder);
    } catch (error) {
      onerror(error);
    }
  }
</script>

<label
  >{label}
  <div class="folder-picker">
    <input
      aria-label={label}
      {value}
      oninput={(event) => onchange(event.currentTarget.value)}
    /><button
      type="button"
      aria-label={`Choose ${label} folder`}
      onclick={choose}>Choose…</button
    >
  </div></label
>
