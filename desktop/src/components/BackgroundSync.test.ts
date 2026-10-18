import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { test, expect, vi, beforeEach } from "vitest";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import BackgroundSync from "./BackgroundSync.svelte";
import type { BackgroundStatus } from "../api";
const active: BackgroundStatus = {
  paused: false,
  interval_seconds: 60,
  running: false,
  last_success: 1792173600,
  skills: 29,
  error: null,
  migration_error: null,
};
beforeEach(() => clearMocks());
test("tray status shows last success and skill count; pause and resume persist through the controller", async () => {
  let status = { ...active };
  const calls = vi.fn((cmd: string, args: unknown) => {
    if (cmd === "background_status") return status;
    if (cmd === "set_background_preferences") {
      const preferences = args as { paused: boolean; intervalSeconds: number };
      status = {
        ...status,
        paused: preferences.paused,
        interval_seconds: preferences.intervalSeconds,
      };
      return null;
    }
    throw new Error(`Unexpected command: ${cmd}`);
  });
  mockIPC(calls);
  render(BackgroundSync, { onerror: vi.fn() });
  expect(await screen.findByText("Active")).toBeVisible();
  expect(screen.getByText(/Last success:.*29 skills/)).toBeVisible();
  await fireEvent.click(screen.getByRole("button", { name: "Pause sync" }));
  expect(await screen.findByText("Paused")).toBeVisible();
  await fireEvent.click(screen.getByRole("button", { name: "Resume sync" }));
  expect(await screen.findByText("Active")).toBeVisible();
  await fireEvent.input(screen.getByLabelText("Sync interval (seconds)"), {
    target: { value: "120" },
  });
  await fireEvent.click(screen.getByRole("button", { name: "Save interval" }));
  await waitFor(() =>
    expect(calls).toHaveBeenCalledWith("set_background_preferences", {
      paused: false,
      intervalSeconds: 120,
    }),
  );
});
test("migration refusals are visible and can be retried without a modal", async () => {
  const error =
    "Legacy task has no ownership record; preserving it for manual review";
  const calls = vi.fn((cmd: string) =>
    cmd === "background_status" ? { ...active, migration_error: error } : null,
  );
  mockIPC(calls);
  render(BackgroundSync, { onerror: vi.fn() });
  expect(await screen.findByRole("alert")).toHaveTextContent(error);
  expect(screen.getByText("Blocked")).toBeVisible();
  expect(screen.queryByRole("dialog")).toBeNull();
  await fireEvent.click(
    screen.getByRole("button", { name: "Retry task migration" }),
  );
  await waitFor(() =>
    expect(calls).toHaveBeenCalledWith("migrate_background", {}),
  );
});
test("macOS has no tray settings and command errors are surfaced", async () => {
  const onerror = vi.fn();
  mockIPC(() => null);
  const view = render(BackgroundSync, { onerror });
  await waitFor(() => expect(screen.queryByLabelText("Tray sync")).toBeNull());
  view.unmount();
  mockIPC(() => Promise.reject("Cannot read tray status"));
  render(BackgroundSync, { onerror });
  await waitFor(() =>
    expect(onerror).toHaveBeenCalledWith("Cannot read tray status"),
  );
});
