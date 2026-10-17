<script lang="ts">
  import type { SecretReference } from "../api";
  let {
    label,
    value,
    onchange,
  }: {
    label: string;
    value: SecretReference;
    onchange: (value: SecretReference) => void;
  } = $props();
  function choose(kind: SecretReference["kind"]) {
    onchange(
      kind === "gopass"
        ? { kind, entry: "" }
        : kind === "keychain"
          ? { kind, service: "", account: "" }
          : kind === "windows-credential"
            ? { kind, target: "" }
            : { kind, variable: "" },
    );
  }
</script>

<fieldset class="secret-reference">
  <legend>{label}</legend>
  <label
    >Secret store<select
      aria-label={`${label} secret store`}
      value={value.kind}
      onchange={(e) => choose(e.currentTarget.value as SecretReference["kind"])}
    >
      <option value="gopass">gopass entry</option>
      <option value="keychain">macOS Keychain item</option>
      <option value="windows-credential"
        >Windows Credential Manager entry</option
      >
      <option value="environment">Environment variable</option>
    </select></label
  >
  {#if value.kind === "gopass"}
    <label
      >Entry name<input
        aria-label={`${label} gopass entry`}
        value={value.entry}
        oninput={(e) =>
          onchange({ kind: "gopass", entry: e.currentTarget.value })}
      /></label
    >
  {:else if value.kind === "keychain"}
    <label
      >Service<input
        aria-label={`${label} Keychain service`}
        value={value.service}
        oninput={(e) =>
          onchange({
            ...value,
            kind: "keychain",
            account: value.kind === "keychain" ? value.account : "",
            service: e.currentTarget.value,
          })}
      /></label
    >
    <label
      >Account<input
        aria-label={`${label} Keychain account`}
        value={value.account}
        oninput={(e) =>
          onchange({
            ...value,
            kind: "keychain",
            service: value.kind === "keychain" ? value.service : "",
            account: e.currentTarget.value,
          })}
      /></label
    >
  {:else if value.kind === "windows-credential"}
    <label
      >Entry name<input
        aria-label={`${label} Windows credential entry`}
        value={value.target}
        oninput={(e) =>
          onchange({
            kind: "windows-credential",
            target: e.currentTarget.value,
          })}
      /></label
    >
  {:else}
    <label
      >Variable name<input
        aria-label={`${label} environment variable`}
        value={value.variable}
        oninput={(e) =>
          onchange({ kind: "environment", variable: e.currentTarget.value })}
      /></label
    >
  {/if}
</fieldset>
