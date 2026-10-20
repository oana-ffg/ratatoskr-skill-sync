import { render, screen } from "@testing-library/svelte";
import { test, expect, vi, beforeEach } from "vitest";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import Activity from "./Activity.svelte";
import { fixture, fixtureIPC } from "../../tests/fixture";
beforeEach(() => clearMocks());
test("Windows activity uses the shared run ledger and replaces scheduled-task controls with tray status", async () => {
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
    return fixtureIPC(cmd, args as Record<string, unknown>);
  });
  render(Activity, {
    workspace: {
      ...fixture.workspace,
      jobs: {
        ...fixture.workspace.jobs,
        log: "Latest in-process sync outcome",
      },
    },
    onrefresh: vi.fn(),
    onupdate: vi.fn(),
    onerror: vi.fn(),
    ondirty: vi.fn(),
  });
  expect(
    await screen.findByText("Latest in-process sync outcome"),
  ).toBeVisible();
  expect(
    await screen.findByText("Tray mode · Recommended on Windows"),
  ).toBeVisible();
  expect(screen.queryByRole("button", { name: "Review schedule…" })).toBeNull();
  expect(screen.queryByRole("dialog")).toBeNull();
});
