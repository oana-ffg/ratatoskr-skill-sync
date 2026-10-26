import type { Page } from "@playwright/test";
import { fixtureIPC } from "../tests/fixture";

export async function installFixture(
  page: Page,
  invoke: (cmd: string, args: Record<string, unknown>) => unknown = fixtureIPC,
) {
  await page.exposeFunction("fixtureIPC", invoke);
  await page.addInitScript(() => {
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      value: {
        invoke: (cmd: string, args: Record<string, unknown>) =>
          (
            window as unknown as {
              fixtureIPC: (
                cmd: string,
                args: Record<string, unknown>,
              ) => Promise<unknown>;
            }
          ).fixtureIPC(cmd, args),
      },
    });
  });
}
