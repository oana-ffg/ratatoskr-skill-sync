import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { test, expect, vi, beforeEach } from "vitest";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import App from "./App.svelte";
import { fixtureIPC } from "../tests/fixture";
beforeEach(() => {
  clearMocks();
  HTMLDialogElement.prototype.showModal = function () {
    this.open = true;
  };
  HTMLDialogElement.prototype.close = function () {
    this.open = false;
  };
  mockIPC((cmd, args) => fixtureIPC(cmd, args as Record<string, unknown>));
});
test("routes screens, guards unsaved settings, and discards only on explicit action", async () => {
  render(App);
  await screen.findByRole("heading", { name: /Skills/ });
  await fireEvent.click(screen.getByRole("button", { name: /Settings/ }));
  await screen.findByLabelText("Open Ratatoskr at login");
  expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
  await fireEvent.click(screen.getByLabelText("Open Ratatoskr at login"));
  expect(await screen.findByRole("button", { name: "Save" })).toBeEnabled();
  await fireEvent.click(screen.getByRole("button", { name: /Projects/ }));
  expect(
    await screen.findByRole("dialog", { name: "You have unsaved changes" }),
  ).toBeVisible();
  await fireEvent.click(screen.getByRole("button", { name: "Keep editing" }));
  expect(screen.getByRole("heading", { name: "Settings" })).toBeVisible();
  await fireEvent.click(screen.getByRole("button", { name: /Projects/ }));
  await fireEvent.click(
    screen.getByRole("button", { name: "Discard and continue" }),
  );
  expect(
    await screen.findByRole("heading", { name: /Projects/ }),
  ).toBeVisible();
});
test("IPC errors are visible and not converted into success", async () => {
  const calls = vi.fn((cmd: string, args: unknown) => {
    if (cmd === "set_skill_enabled")
      throw new Error("Source changed; reload before saving");
    return fixtureIPC(cmd, args as Record<string, unknown>);
  });
  mockIPC(calls);
  render(App);
  await screen.findByRole("heading", { name: /Skills/ });
  await fireEvent.click(
    screen.getByRole("switch", { name: "Enable recordings" }),
  );
  await waitFor(() =>
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Source changed; reload before saving",
    ),
  );
});

test("update actions follow the native updater result", async () => {
  let checked = false;
  mockIPC((cmd, args) => {
    if (cmd === "update_desktop") {
      const install = (args as { install: boolean }).install;
      expect(install ? checked : true).toBe(true);
      checked = true;
      return {
        message: install ? "Installed" : "Available",
        available: !install,
        installed: install,
      };
    }
    return fixtureIPC(cmd, args as Record<string, unknown>);
  });
  render(App);
  await screen.findByRole("heading", { name: /Skills/ });
  await fireEvent.click(screen.getByRole("button", { name: /Settings/ }));
  const check = await screen.findByRole("button", {
    name: "Check for updates",
  });
  expect(
    screen.queryByRole("button", { name: "Install signed update" }),
  ).toBeNull();
  expect(screen.queryByRole("button", { name: "Restart app" })).toBeNull();
  await fireEvent.click(check);
  await fireEvent.click(
    await screen.findByRole("button", { name: "Install signed update" }),
  );
  expect(
    await screen.findByRole("button", { name: "Restart app" }),
  ).toBeEnabled();
  expect(
    screen.queryByRole("button", { name: "Install signed update" }),
  ).toBeNull();
  expect(screen.queryByRole("button", { name: "Save" })).toBeNull();
});
