/** Keep the workspace noninteractive until startup installation/relaunch finishes. */
export async function runStartupUpdate(
  update: () => Promise<void>,
  ready: () => Promise<void>,
): Promise<void> {
  try {
    await update();
  } finally {
    await ready();
  }
}
