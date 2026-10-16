/** Recheck work after the network response; editing can start while a check is pending. */
export async function runNightlyUpdate(
  check: () => Promise<boolean>,
  idle: () => boolean,
  install: () => Promise<void>,
): Promise<void> {
  if ((await check()) && idle()) await install();
}

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
