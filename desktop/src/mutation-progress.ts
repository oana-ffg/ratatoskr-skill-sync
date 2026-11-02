import { Channel, invoke } from "@tauri-apps/api/core";

// One boundary for source edits, local settings and explicit publication.
const commands = new Set([
  "push_content",
  "save_global_instructions",
  "set_skill_enabled",
  "archive_source_skill",
  "restore_workspace",
  "resolve_conflict",
  "add_agent_target",
  "save_skill_draft",
  "save_structured_source",
  "add_skill_resource",
  "rename_source_skill",
  "save_local_settings",
  "create_desktop_skill",
]);
const listeners = new Set<(message: string | null) => void>();
let latest = 0;

export function watchMutationProgress(
  listener: (message: string | null) => void,
) {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export async function command<T>(
  name: string,
  args?: Record<string, unknown>,
): Promise<T> {
  // The browser fixture can implement invoke without native callback transport.
  const internals = (
    window as unknown as {
      __TAURI_INTERNALS__?: { transformCallback?: unknown };
    }
  ).__TAURI_INTERNALS__;
  if (!commands.has(name) || typeof internals?.transformCallback !== "function")
    return invoke<T>(name, args);

  const operation = ++latest;
  let active = true;
  const notify = (message: string | null) => {
    if (active && operation === latest)
      for (const listener of listeners) listener(message);
  };
  notify(null);
  const onProgress = new Channel<string>((message) => notify(message));
  try {
    return await invoke<T>(name, { ...args, onProgress });
  } finally {
    notify(null);
    active = false;
    // Channel ordering handles stages; the active guard also rejects messages
    // queued before the command's reply but delivered after it settled.
  }
}
