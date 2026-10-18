<script lang="ts">
  import BackgroundSync from "../components/BackgroundSync.svelte";
  import { onMount } from "svelte";
  import {
    api,
    type Snapshot,
    type LocalSettings,
    type SecretReference,
  } from "../api";
  let {
    workspace,
    onupdate,
    onerror,
    ondirty,
    onunsaved,
  }: {
    workspace: Snapshot | null;
    onupdate: (w: Snapshot) => void;
    onerror: (e: unknown) => void;
    ondirty: (v: boolean) => void;
    onunsaved?: (v: boolean) => void;
  } = $props();
  let trayMode = $state(false);
  let updates = $state<Snapshot["updates"] | null>(null);
  let settings = $state<LocalSettings | null>(null);
  let original = $state("");
  let configHash = $state("");
  let login = $state(false);
  let savedLogin = $state(false);
  let message = $state("");
  let updateAvailable = $state(false);
  let updateInstalled = $state(false);
  let busy = $state(false);
  let alertKind = $state("none");
  let defaultSecret = $state<SecretReference>({
    kind: "environment",
    variable: "",
  });
  let token = $state<SecretReference>({ kind: "environment", variable: "" });
  let user = $state<SecretReference>({ kind: "environment", variable: "" });
  let topic = $state("");
  let authenticated = $state(false);
  import SecretField from "../components/SecretField.svelte";
  function changeDelivery() {
    token = { ...defaultSecret };
    user = { ...defaultSecret };
    topic = "";
    authenticated = false;
  }
  let command = $state<string[]>([]);
  const serialized = $derived(
    JSON.stringify({
      settings,
      login,
      alertKind,
      token,
      user,
      topic,
      authenticated,
      command,
    }),
  );
  const dirty = $derived(!!original && serialized !== original);
  $effect(() => onunsaved?.(dirty));
  $effect(() => ondirty(dirty || busy));
  onMount(() => {
    void load();
  });
  async function load() {
    try {
      updates = await api.command<Snapshot["updates"]>("read_update_settings");
      if (!workspace) return;
      const [candidate, enabled, profile] = await Promise.all([
        api.command<{ settings: LocalSettings; hash: string }>(
          "read_local_settings",
        ),
        api.command<boolean>("login_enabled"),
        api.source(`machines/${workspace.plan.machine}.toml`),
      ]);
      settings = candidate.settings;
      configHash = candidate.hash;
      login = enabled;
      savedLogin = login;
      defaultSecret = (
        profile.values.capabilities as string[] | undefined
      )?.includes("gopass")
        ? { kind: "gopass", entry: "" }
        : profile.values.os === "macos"
          ? { kind: "keychain", service: "", account: "" }
          : profile.values.os === "windows"
            ? { kind: "windows-credential", target: "" }
            : { kind: "environment", variable: "" };
      const alert = settings.alert;
      alertKind =
        alert?.kind ?? (settings.alert_command.length ? "custom" : "none");
      token =
        alert && alert.kind !== "system" && alert.token
          ? alert.token
          : { ...defaultSecret };
      user = alert?.kind === "pushover" ? alert.user : { ...defaultSecret };
      topic = alert?.kind === "ntfy" ? alert.url : "";
      authenticated = alert?.kind === "ntfy" && !!alert.token;
      command = [...settings.alert_command];
      original = JSON.stringify({
        settings,
        login,
        alertKind,
        token,
        user,
        topic,
        authenticated,
        command,
      });
    } catch (e) {
      onerror(e);
    }
  }
  export async function discard() {
    await load();
  }
  export async function save() {
    if (!settings) return;
    busy = true;
    try {
      settings.alert =
        alertKind === "pushover"
          ? { kind: "pushover", token, user }
          : alertKind === "ntfy"
            ? { kind: "ntfy", url: topic, ...(authenticated ? { token } : {}) }
            : alertKind === "system"
              ? { kind: "system" }
              : null;
      settings.alert_command = alertKind === "custom" ? command : [];
      onupdate(
        await api.command<Snapshot>("save_local_settings", {
          candidate: settings,
          expectedHash: configHash,
        }),
      );
      if (login !== savedLogin)
        login = await api.command<boolean>("set_login_enabled", {
          enabled: login,
        });
      await load();
    } finally {
      busy = false;
    }
  }
  async function update(install: boolean) {
    if (dirty) {
      onerror("Save or discard your changes before updating.");
      return;
    }
    busy = true;
    try {
      const result = await api.command<{
        message: string;
        available: boolean;
        installed: boolean;
      }>("update_desktop", { install });
      message = result.message;
      updateAvailable = result.available;
      updateInstalled ||= result.installed;
    } catch (e) {
      onerror(e);
    } finally {
      busy = false;
    }
  }
</script>

<section class="page settings-page">
  <div class="page-heading">
    <div>
      <h1>Settings</h1>
    </div>
  </div>
  {#if updates}<article class="panel">
      <div class="section-heading">
        <h2>Updates</h2>
        <span
          class="status-pill {updates.override_active ? 'warning' : 'success'}"
          >{updates.override_active
            ? "Custom override active"
            : "Official signed updates"}</span
        >
      </div>
      <p class="muted">
        Signed desktop and helper releases from the configured update source.
      </p>
      {#if settings}<label class="checkbox-label"
          ><input
            type="checkbox"
            bind:checked={settings.updates.automatic}
          />Check for updates automatically</label
        ><label class="time-field"
          >Nightly time <small>Local time on this machine</small><input
            type="time"
            bind:value={settings.updates.time}
          /></label
        >
      {:else}<p class="muted">
          Automatic updates are {updates.automatic ? "on" : "off"}. Nightly
          time: {updates.time} local time. Connect a workspace to customize the schedule.
        </p>{/if}
      <details>
        <summary>Update source</summary>
        <p class="path">{updates.endpoint}</p>
      </details>
      <div class="form-row">
        <button disabled={busy || dirty} onclick={() => update(false)}
          >Check for updates</button
        >{#if updateAvailable && !updateInstalled}<button
            disabled={busy || dirty}
            onclick={() => update(true)}>Install signed update</button
          >{/if}{#if updateInstalled}<button
            disabled={busy || dirty}
            onclick={() => api.command("restart_desktop").catch(onerror)}
            >Restart app</button
          >{/if}
      </div>
      {#if message}<p role="status" class="success-message">{message}</p>{/if}
    </article>{/if}
  {#if settings}<BackgroundSync
      {onerror}
      onstatus={(status) => (trayMode = status !== null)}
    />
    <article class="panel">
      <div class="section-heading">
        <div>
          <h2>Startup</h2>
        </div>
      </div>
      <label class="checkbox-label"
        ><input type="checkbox" bind:checked={login} />Open Ratatoskr at login</label
      >
    </article>
    <article class="panel">
      <h2>{trayMode ? "Additional alerts" : "Alerts"}</h2>
      <p class="muted">
        {trayMode
          ? "Tray notifications report errors and recovery. Remote and custom alerts are optional."
          : "Get a message when background work fails or recovers."}
      </p>
      <label
        >Delivery<select bind:value={alertKind} onchange={changeDelivery}
          ><option value="none"
            >{trayMode ? "No additional alerts" : "No notifications"}</option
          ><option value="system">System notification</option><option
            value="pushover">Pushover</option
          ><option value="ntfy">ntfy</option><option value="custom"
            >Custom command</option
          ></select
        ></label
      >
      {#if alertKind === "pushover"}
        <SecretField
          label="Application token"
          value={token}
          onchange={(value) => (token = value)}
        />
        <SecretField
          label="User key"
          value={user}
          onchange={(value) => (user = value)}
        />
      {:else if alertKind === "ntfy"}
        <p class="muted">
          ntfy sends push notifications to a topic URL you subscribe to in the
          ntfy app.
        </p>
        <label>Topic URL<input type="url" bind:value={topic} /></label>
        <label class="checkbox-label"
          ><input type="checkbox" bind:checked={authenticated} />Topic requires
          a bearer token</label
        >
        {#if authenticated}<SecretField
            label="Bearer token"
            value={token}
            onchange={(value) => (token = value)}
          />{/if}
      {:else if alertKind === "custom"}<label
          >Executable<input
            value={command[0] ?? ""}
            oninput={(e) =>
              (command = [e.currentTarget.value, ...command.slice(1)])}
          /></label
        >{#each command.slice(1) as arg, i}<div class="key-value-row">
            <label
              >Argument {i + 1}<input
                value={arg}
                oninput={(e) => (command[i + 1] = e.currentTarget.value)}
              /></label
            ><button
              onclick={() =>
                (command = command.filter((_, index) => index !== i + 1))}
              >Remove</button
            >
          </div>{/each}<button onclick={() => (command = [...command, ""])}
          >Add argument</button
        >
        <p class="muted small">
          The alert message is appended as the last argument. No shell is used.
        </p>{/if}
      {#if alertKind === "pushover" || alertKind === "ntfy"}<p
          class="muted small"
        >
          Only secret references are saved. Ratatoskr reads credentials from the
          selected store each time it sends an alert. Resolution failures appear
          in Activity.
        </p>{/if}
    </article>{/if}
</section>
