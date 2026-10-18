import { render, screen } from "@testing-library/svelte";
import { test, expect, vi, beforeEach } from "vitest";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import Activity from "./Activity.svelte";
import { fixture, fixtureIPC } from "../../tests/fixture";
beforeEach(() => clearMocks());
test("Windows activity refreshes the run ledger and replaces scheduled-task controls with tray status", async () => {
  mockIPC((cmd, args) => {
    if (cmd === "background_status")
      return {
        login_enabled: true,
        paused: false,
        interval_seconds: 60,
        running: false,
        last_success: 1792324740,
        skills: 29,
        error: null,
        migration_error: null,
      };
    if (cmd === "read_job_status")
      return {
        ...fixture.workspace.jobs,
        log: "Latest in-process sync outcome",
      };
    return fixtureIPC(cmd, args as Record<string, unknown>);
  });
  render(Activity, {
    workspace: fixture.workspace,
    onupdate: vi.fn(),
    onerror: vi.fn(),
    ondirty: vi.fn(),
  });
  expect(
    await screen.findByText("Latest in-process sync outcome"),
  ).toBeVisible();
  expect(screen.getByText("Tray mode · Recommended on Windows")).toBeVisible();
  expect(screen.queryByRole("button", { name: "Review schedule…" })).toBeNull();
  expect(screen.queryByRole("dialog")).toBeNull();
});
