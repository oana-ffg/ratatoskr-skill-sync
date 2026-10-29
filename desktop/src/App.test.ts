import {
  render,
  screen,
  fireEvent,
  waitFor,
  within,
} from "@testing-library/svelte";
import { test, expect, vi, beforeEach } from "vitest";
import { mockIPC, clearMocks, mockWindows } from "@tauri-apps/api/mocks";
import App from "./App.svelte";
import { fixture, fixtureIPC } from "../tests/fixture";
beforeEach(() => {
  clearMocks();
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

test("onboarding Settings checks and installs updates without workspace commands", async () => {
  const calls = vi.fn((cmd: string, args: unknown) => {
    if (cmd === "read_workspace") return null;
    if (cmd === "read_update_settings")
      return { ...fixture.workspace.updates, time: "00:00" };
    if (cmd === "read_local_settings" || cmd === "login_enabled")
      throw new Error("No workspace config");
    if (cmd === "update_desktop") {
      const install = (args as { install: boolean }).install;
      return {
        message: install ? "Installed" : "Available",
        available: !install,
        installed: install,
      };
    }
    return fixtureIPC(cmd, args as Record<string, unknown>);
  });
  mockIPC(calls);
  render(App);
  await screen.findByRole("button", { name: "Connect workspace" });
  await fireEvent.click(screen.getByRole("button", { name: /Settings/ }));
  expect(await screen.findByRole("heading", { name: "Updates" })).toBeVisible();
  expect(screen.getByText(/Nightly time: 00:00 local time/)).toBeVisible();
  await fireEvent.click(
    screen.getByRole("button", { name: "Check for updates" }),
  );
  await fireEvent.click(
    await screen.findByRole("button", { name: "Install signed update" }),
  );
  expect(
    await screen.findByRole("button", { name: "Restart app" }),
  ).toBeEnabled();
  expect(screen.queryByRole("alert")).toBeNull();
  expect(calls.mock.calls.some(([cmd]) => cmd === "read_local_settings")).toBe(
    false,
  );
});

test("native startup gates readiness and unsaved editors notify the native scheduler", async () => {
  vi.stubGlobal("isTauri", true);
  mockWindows("main");
  const calls = vi.fn((cmd: string, args: unknown) => {
    if (cmd === "startup_update") return false;
    if (cmd === "plugin:window|is_visible") return true;
    if (cmd === "plugin:window|is_minimized") return false;
    if (cmd.startsWith("plugin:event|")) return 1;
    return fixtureIPC(cmd, args as Record<string, unknown>);
  });
  mockIPC(calls);
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval"] });
  const { unmount } = render(App);
  try {
    await screen.findByRole("heading", { name: /Skills/ });
    expect(calls).toHaveBeenCalledWith("startup_update", {});
    expect(calls).toHaveBeenCalledWith("set_app_error_listener", {
      active: true,
    });
    const listener = calls.mock.calls.findIndex(
      ([cmd, args]) =>
        cmd === "plugin:event|listen" &&
        (args as { event?: string }).event === "app-error",
    );
    const acknowledged = calls.mock.calls.findIndex(
      ([cmd]) => cmd === "set_app_error_listener",
    );
    expect(listener).toBeGreaterThanOrEqual(0);
    expect(acknowledged).toBeGreaterThan(listener);
    expect(calls).toHaveBeenCalledWith("set_desktop_busy", { busy: false });
    await fireEvent.click(screen.getByRole("button", { name: /Settings/ }));
    await screen.findByLabelText("Open Ratatoskr at login");
    await fireEvent.click(screen.getByLabelText("Open Ratatoskr at login"));
    await waitFor(() =>
      expect(calls).toHaveBeenCalledWith("set_desktop_busy", { busy: true }),
    );
    calls.mockClear();
    await vi.advanceTimersByTimeAsync(60000);
    expect(calls.mock.calls.some(([cmd]) => cmd === "read_workspace")).toBe(
      false,
    );
    unmount();
    expect(calls).toHaveBeenCalledWith("set_app_error_listener", {
      active: false,
    });
    calls.mockClear();
    await vi.advanceTimersByTimeAsync(60000);
    expect(calls).not.toHaveBeenCalled();
  } finally {
    unmount();
    vi.useRealTimers();
    vi.unstubAllGlobals();
  }
});

test("content checkout warning is visible across screens and Push now refreshes it", async () => {
  let workspace = structuredClone(fixture.workspace);
  workspace.plan.content_status = {
    changed_files: 2,
    outgoing_commits: 1,
    upstream: "origin/main",
    error: null,
  };
  const calls = vi.fn((cmd: string, args: unknown) => {
    if (cmd === "read_workspace") return workspace;
    if (cmd === "push_content") {
      workspace = structuredClone(workspace);
      workspace.plan.content_status.outgoing_commits = 0;
      return workspace;
    }
    return fixtureIPC(cmd, args as Record<string, unknown>);
  });
  mockIPC(calls);
  render(App);
  await screen.findByRole("heading", { name: /Skills/ });
  expect(screen.getByText("Content checkout needs attention")).toBeVisible();
  expect(
    screen.getByText(/2 uncommitted files · 1 unpushed commits/),
  ).toBeVisible();
  await fireEvent.click(screen.getByRole("button", { name: /Projects/ }));
  expect(screen.getByText("Content checkout needs attention")).toBeVisible();
  await fireEvent.click(screen.getByRole("button", { name: /Settings/ }));
  const login = await screen.findByLabelText("Open Ratatoskr at login");
  await fireEvent.click(login);
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  await fireEvent.click(screen.getByRole("button", { name: "Push now" }));
  await waitFor(() =>
    expect(screen.queryByRole("button", { name: "Push now" })).toBeNull(),
  );
  expect(login).toBeChecked();
  expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  expect(
    screen.getByText(/2 uncommitted files · 0 unpushed commits/),
  ).toBeVisible();
  expect(
    calls.mock.calls.filter(([cmd]) => cmd === "push_content"),
  ).toHaveLength(1);
});

test("failed publication refreshes content status and exposes retry", async () => {
  const status = {
    changed_files: 0,
    outgoing_commits: 1,
    upstream: "origin/main",
    error: null,
  };
  let failPush = true;
  const calls = vi.fn((cmd: string, args: unknown) => {
    if (cmd === "set_skill_enabled" || (cmd === "push_content" && failPush))
      throw new Error("saved and committed locally but NOT pushed: offline");
    if (cmd === "read_content_status") return status;
    if (cmd === "push_content") {
      const next = structuredClone(fixture.workspace);
      next.plan.content_status = { ...status, outgoing_commits: 0 };
      return next;
    }
    return fixtureIPC(cmd, args as Record<string, unknown>);
  });
  mockIPC(calls);
  render(App);
  await screen.findByRole("heading", { name: /Skills/ });
  await fireEvent.click(
    screen.getByRole("switch", { name: "Enable recordings" }),
  );
  await screen.findByText(
    /saved and committed locally but NOT pushed: offline/,
  );
  await fireEvent.click(
    await screen.findByRole("button", { name: "Push now" }),
  );
  await screen.findByText(
    /saved and committed locally but NOT pushed: offline/,
  );
  expect(screen.getByRole("button", { name: "Push now" })).toBeEnabled();
  failPush = false;
  await fireEvent.click(screen.getByRole("button", { name: "Push now" }));
  await waitFor(() =>
    expect(screen.queryByText("Content checkout needs attention")).toBeNull(),
  );
});

test("creating a skill opens it in the editor without an unsaved-changes prompt", async () => {
  // The new skill is opened by name; reuse the fixture skill so it can be read.
  const created = vi.fn();
  mockIPC((cmd, args) => {
    if (cmd !== "create_desktop_skill")
      return fixtureIPC(cmd, args as Record<string, unknown>);
    created(args);
    return fixture.workspace;
  });
  render(App);
  await screen.findByRole("heading", { name: /Skills/ });
  await fireEvent.click(
    await screen.findByRole("button", { name: "＋ New skill" }),
  );
  const dialog = await screen.findByRole("dialog", { name: "Create a skill" });
  await fireEvent.input(within(dialog).getByLabelText("Name"), {
    target: { value: "recordings" },
  });
  await fireEvent.input(within(dialog).getByLabelText("Description"), {
    target: { value: "A freshly created skill." },
  });
  await fireEvent.click(
    within(dialog).getByRole("button", { name: "Create skill" }),
  );
  expect(
    await screen.findByRole("textbox", { name: "Skill Markdown editor" }),
  ).toBeVisible();
  expect(created).toHaveBeenCalledWith({
    name: "recordings",
    project: null,
    description: "A freshly created skill.",
  });
  expect(
    screen.queryByRole("dialog", { name: "You have unsaved changes" }),
  ).toBeNull();
});
