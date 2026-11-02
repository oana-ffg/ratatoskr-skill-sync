import { Channel, invoke } from "@tauri-apps/api/core";

// One boundary for source edits, local settings, publication, apply and sync.
// `progress_commands_match_the_frontend_list` keeps this in step with the Rust commands.
const commands = new Set([
  "apply_workspace",
  "sync_workspace",
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
  if (!commands.has(name)) return invoke<T>(name, args);

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
    // A message queued before the reply can arrive after it.
    active = false;
  }
}
