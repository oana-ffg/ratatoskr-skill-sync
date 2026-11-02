import type { Page } from "@playwright/test";
import { fixtureIPC } from "../tests/fixture";

export async function installFixture(
  page: Page,
  invoke: (cmd: string, args: Record<string, unknown>) => unknown = fixtureIPC,
) {
  await page.exposeFunction("fixtureIPC", invoke);
  await page.addInitScript(() => {
    const host = window as unknown as {
      fixtureIPC: (
        cmd: string,
        args: Record<string, unknown>,
      ) => Promise<unknown>;
      fixtureProgress: (id: number, index: number, message: string) => void;
    };
    const callbacks = new Map<number, (value: unknown) => void>();
    let next = 0;
    host.fixtureProgress = (id, index, message) =>
      callbacks.get(id)?.({ index, message });
    Object.defineProperty(window, "__TAURI_INTERNALS__", {
      value: {
        // JSON turns a Channel into the id native code receives.
        invoke: (cmd: string, args?: Record<string, unknown>) =>
          host.fixtureIPC(cmd, JSON.parse(JSON.stringify(args ?? {}))),
        transformCallback: (callback: (value: unknown) => void) => {
          callbacks.set(++next, callback);
          return next;
        },
        unregisterCallback: (id: number) => {
          callbacks.delete(id);
        },
      },
    });
  });
}
