import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { test, expect, vi, beforeEach } from "vitest";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import Settings from "./Settings.svelte";
import { fixture, fixtureIPC } from "../../tests/fixture";
import type { LocalSettings } from "../api";
beforeEach(() => clearMocks());
test("gopass-capable machines default credentials to gopass and save references only", async () => {
  let saved: LocalSettings | undefined;
  mockIPC((cmd, args) => {
    if (cmd === "save_local_settings") {
      saved = (args as { candidate: LocalSettings }).candidate;
      return fixture.workspace;
    }
    return fixtureIPC(cmd, args as Record<string, unknown>);
  });
  const { component } = render(Settings, {
    workspace: fixture.workspace,
    onupdate: vi.fn(),
    onerror: vi.fn(),
    ondirty: vi.fn(),
  });
  const delivery = await screen.findByLabelText("Delivery");
  await fireEvent.change(delivery, { target: { value: "pushover" } });
  expect(screen.getByLabelText("Application token secret store")).toHaveValue(
    "gopass",
  );
  expect(screen.getByLabelText("User key secret store")).toHaveValue("gopass");
  await fireEvent.input(
    screen.getByLabelText("Application token gopass entry"),
    { target: { value: "pushover/codex-approvals/app-token" } },
  );
  await fireEvent.input(screen.getByLabelText("User key gopass entry"), {
    target: { value: "pushover/codex-approvals/user-key" },
  });
  await component.save();
  expect(saved?.alert).toEqual({
    kind: "pushover",
    token: { kind: "gopass", entry: "pushover/codex-approvals/app-token" },
    user: { kind: "gopass", entry: "pushover/codex-approvals/user-key" },
  });
});
test("ntfy explains subscriptions and all stores are selectable without reading credentials", async () => {
  const calls = vi.fn((cmd: string, args: unknown) =>
    fixtureIPC(cmd, args as Record<string, unknown>),
  );
  mockIPC(calls);
  render(Settings, {
    workspace: fixture.workspace,
    onupdate: vi.fn(),
    onerror: vi.fn(),
    ondirty: vi.fn(),
  });
  await fireEvent.change(await screen.findByLabelText("Delivery"), {
    target: { value: "ntfy" },
  });
  expect(screen.getByText(/ntfy sends push notifications/)).toBeVisible();
  await fireEvent.click(screen.getByLabelText("Topic requires a bearer token"));
  const store = screen.getByLabelText("Bearer token secret store");
  for (const [kind, label] of [
    ["keychain", "Bearer token Keychain service"],
    ["windows-credential", "Bearer token Windows credential entry"],
    ["environment", "Bearer token environment variable"],
    ["gopass", "Bearer token gopass entry"],
  ]) {
    await fireEvent.change(store, { target: { value: kind } });
    expect(screen.getByLabelText(label)).toBeVisible();
  }
  await waitFor(() =>
    expect(calls).not.toHaveBeenCalledWith(
      "save_local_settings",
      expect.anything(),
    ),
  );
});
