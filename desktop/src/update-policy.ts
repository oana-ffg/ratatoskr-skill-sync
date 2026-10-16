export interface UpdateStatus {
  endpoint: string;
  override_active: boolean;
  automatic: boolean;
  time: string;
}

export function updateStatusText(settings: UpdateStatus): string {
  return `${settings.override_active ? "Custom update override active" : "Official signed updates"}: ${settings.endpoint}. ${settings.automatic ? `Automatic at ${settings.time} local time` : "Automatic updates off"}.`;
}

/** Recheck work after the network response; editing can start while a check is pending. */
export async function runNightlyUpdate(
  check: () => Promise<boolean>,
  idle: () => boolean,
  install: () => Promise<void>,
): Promise<void> {
  if ((await check()) && idle()) await install();
}

export interface EditorState {
  configuration: { text: string; saved: string };
  source: { text: string; saved: string } | null;
  skill: {
    markdown: string;
    metadata: string;
    savedMarkdown: string;
    savedMetadata: string;
  } | null;
  newSkill: { name: string; markdown: string };
  schedule: {
    automatic: boolean;
    savedAutomatic: boolean;
    time: string;
    savedTime: string;
  };
}

export function hasUnsavedEdits(state: EditorState): boolean {
  return (
    state.configuration.text !== state.configuration.saved ||
    (!!state.source && state.source.text !== state.source.saved) ||
    (!!state.skill &&
      (state.skill.markdown !== state.skill.savedMarkdown ||
        state.skill.metadata !== state.skill.savedMetadata)) ||
    state.newSkill.name.length > 0 ||
    state.newSkill.markdown.length > 0 ||
    state.schedule.automatic !== state.schedule.savedAutomatic ||
    state.schedule.time !== state.schedule.savedTime
  );
}
