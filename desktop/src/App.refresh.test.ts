import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { flushSync } from "svelte";
import { test, expect, vi, beforeEach, afterEach, type Mock } from "vitest";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import { fixture, fixtureIPC } from "../tests/fixture";
import App from "./App.svelte";
import { emit } from "@tauri-apps/api/event";
import type { Snapshot } from "./api";
const native = vi.hoisted(() => ({
  platform: "linux",
  close: new Set<(event: { preventDefault: () => void }) => void>(),
  visible: true,
  minimized: false,
  focus: new Set<(event: { payload: boolean }) => void>(),
  resize: new Set<() => void>(),
  shown: new Set<(event: { payload: boolean }) => void>(),
  stopFocus: vi.fn(),
  stopResize: vi.fn(),
  stopShown: vi.fn(),
}));
vi.mock("@tauri-apps/api/window", () => ({
  getCurrentWindow: () => ({
    isVisible: async () => native.visible,
    isMinimized: async () => native.minimized,
    onFocusChanged: async (callback: (event: { payload: boolean }) => void) => {
      native.focus.add(callback);
      return () => {
        native.focus.delete(callback);
        native.stopFocus();
      };
    },
    onResized: async (callback: () => void) => {
      native.resize.add(callback);
      return () => {
        native.resize.delete(callback);
        native.stopResize();
      };
    },
    listen: async (
      _event: string,
      callback: (event: { payload: boolean }) => void,
    ) => {
      native.shown.add(callback);
      return () => {
        native.shown.delete(callback);
        native.stopShown();
      };
    },
    onCloseRequested: async (
      callback: (event: { preventDefault: () => void }) => void,
    ) => {
      native.close.add(callback);
      return () => native.close.delete(callback);
    },
  }),
}));
let reads: Mock<() => Snapshot | Promise<Snapshot>>;
let current: Snapshot;
beforeEach(() => {
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe() {}
      unobserve() {}
      disconnect() {}
    },
  );
  clearMocks();
  vi.stubGlobal("isTauri", true);
  native.platform = "linux";
  native.close.clear();
  native.visible = true;
  native.minimized = false;
  native.focus.clear();
  native.resize.clear();
  native.shown.clear();
  native.stopFocus.mockClear();
  native.stopResize.mockClear();
  native.stopShown.mockClear();
  current = structuredClone(fixture.workspace);
  reads = vi.fn<() => Snapshot | Promise<Snapshot>>(() => current);
  mockIPC(
    (cmd, args) => {
      if (cmd === "runtime_platform") return native.platform;
      if (cmd === "read_workspace") return reads();
      if (cmd === "startup_update" || cmd === "set_desktop_busy") return false;
      return fixtureIPC(cmd, args as Record<string, unknown>);
    },
    { shouldMockEvents: true },
  );
  vi.useFakeTimers({ toFake: ["setInterval", "clearInterval", "Date"] });
  vi.setSystemTime(new Date("2026-10-20T12:00:00Z"));
});
afterEach(() => {
  vi.useRealTimers();
  vi.unstubAllGlobals();
});
async function start() {
  const view = render(App);
  await screen.findByRole("heading", { name: /Skills/ });
  await waitFor(() => expect(reads).toHaveBeenCalledTimes(1));
  return view;
}
async function focus() {
  native.focus.forEach((callback) => callback({ payload: true }));
  await Promise.resolve();
  await Promise.resolve();
  flushSync();
}
test("native focus and explicit tray show refresh header, review and counts without navigating", async () => {
  await start();
  await fireEvent.click(screen.getByRole("button", { name: /Review changes/ }));
  current = {
    ...current,
    plan: {
      ...current.plan,
      mode: "live",
      changes: [],
      conflicts: [],
      conflict_details: [],
    },
    legacy: [],
  };
  await focus();
  expect(await screen.findByText("Live")).toBeVisible();
  expect(
    screen.getByRole("heading", { name: /^Review changes/ }),
  ).toBeVisible();
  expect(
    screen.queryByRole("region", { name: "Older Codex copies" }),
  ).toBeNull();
  native.visible = false;
  native.shown.forEach((callback) => callback({ payload: false }));
  native.visible = true;
  native.shown.forEach((callback) => callback({ payload: true }));
  await waitFor(() => expect(reads).toHaveBeenCalledTimes(3));
});
test("polls every minute only while natively visible, including minimized and blurred windows", async () => {
  const view = await start();
  await vi.advanceTimersByTimeAsync(30_000);
  native.resize.forEach((callback) => callback());
  await vi.advanceTimersByTimeAsync(30_000);
  expect(reads).toHaveBeenCalledTimes(2);
  native.focus.forEach((callback) => callback({ payload: false }));
  await vi.advanceTimersByTimeAsync(60_000);
  expect(reads).toHaveBeenCalledTimes(3);
  native.visible = false;
  native.shown.forEach((callback) => callback({ payload: false }));
  await vi.advanceTimersByTimeAsync(180_000);
  expect(reads).toHaveBeenCalledTimes(3);
  native.visible = true;
  native.shown.forEach((callback) => callback({ payload: true }));
  await waitFor(() => expect(reads).toHaveBeenCalledTimes(4));
  native.minimized = true;
  native.resize.forEach((callback) => callback());
  await vi.advanceTimersByTimeAsync(180_000);
  expect(reads).toHaveBeenCalledTimes(4);
  native.minimized = false;
  native.resize.forEach((callback) => callback());
  await waitFor(() => expect(reads).toHaveBeenCalledTimes(5));
  view.unmount();
  await vi.advanceTimersByTimeAsync(120_000);
  expect(reads).toHaveBeenCalledTimes(5);
  expect(native.stopFocus).toHaveBeenCalledOnce();
  expect(native.stopResize).toHaveBeenCalledOnce();
  expect(native.stopShown).toHaveBeenCalledOnce();
});
test.each(["Settings", "Agents & machines", "Skill editor"])(
  "preserves %s edits and selection, warns, and catches up after discard",
  async (editor) => {
    await start();
    let input: HTMLElement;
    if (editor === "Skill editor") {
      await fireEvent.click(
        screen.getByRole("button", { name: /recordings.*Enabled/ }),
      );
      input = await screen.findByLabelText("Description");
      await fireEvent.input(input, {
        target: { value: "My unsaved description" },
      });
    } else if (editor === "Settings") {
      await fireEvent.click(screen.getByRole("button", { name: /Settings/ }));
      input = await screen.findByLabelText("Open Ratatoskr at login");
      await fireEvent.click(input);
    } else {
      await fireEvent.click(
        screen.getByRole("button", { name: /Agents & machines/ }),
      );
      input = (await screen.findAllByRole("textbox", { name: "Home" }))[0];
      await fireEvent.input(input, { target: { value: "/my/unsaved/home" } });
    }
    await screen.findByRole("button", { name: "Save" });
    await focus();
    await vi.advanceTimersByTimeAsync(120_000);
    expect(reads).toHaveBeenCalledTimes(1);
    expect(screen.getByText("Refresh paused while editing")).toBeVisible();
    if (editor === "Settings") expect(input).toBeChecked();
    else
      expect(input).toHaveValue(
        editor === "Skill editor"
          ? "My unsaved description"
          : "/my/unsaved/home",
      );
    expect(screen.getByRole("button", { name: "Save" })).toBeVisible();
    await fireEvent.click(screen.getByRole("button", { name: "Discard" }));
    await waitFor(() => expect(reads).toHaveBeenCalledTimes(2));
    expect(screen.queryByText("Refresh paused while editing")).toBeNull();
  },
);
test("as-of ages during a visible refresh failure, retains last good time and recovers", async () => {
  await start();
  expect(screen.getByText("As of just now")).toBeVisible();
  reads.mockRejectedValue(new Error("Cannot read sync history"));
  await vi.advanceTimersByTimeAsync(120_000);
  const alert = await screen.findByRole("alert");
  expect(alert).toHaveTextContent("Workspace refresh failed");
  expect(alert).toHaveTextContent("Cannot read sync history");
  expect(alert).toHaveTextContent("As of 2 min ago");
  expect(alert).toHaveTextContent("Last good snapshot:");
  expect(screen.getByText("Status may be out of date")).toBeVisible();
  reads.mockImplementation(() => current);
  await fireEvent.click(screen.getByRole("button", { name: "Retry refresh" }));
  await waitFor(() => expect(screen.queryByRole("alert")).toBeNull());
  expect(screen.getByText("As of just now")).toBeVisible();
});
test("coalesces slow focus/show/timer refreshes and rejects a response if editing started during IPC", async () => {
  await start();
  let finish!: (workspace: Snapshot) => void;
  reads.mockImplementationOnce(
    () => new Promise<Snapshot>((resolve) => (finish = resolve)),
  );
  await focus();
  await focus();
  native.shown.forEach((callback) => callback({ payload: true }));
  await vi.advanceTimersByTimeAsync(120_000);
  expect(reads).toHaveBeenCalledTimes(2);
  await fireEvent.click(screen.getByRole("button", { name: /Settings/ }));
  await fireEvent.click(
    await screen.findByLabelText("Open Ratatoskr at login"),
  );
  finish({ ...current, plan: { ...current.plan, mode: "live" } });
  expect(await screen.findByText("Refresh paused while editing")).toBeVisible();
  expect(screen.getByText("Staging")).toBeVisible();
  expect(screen.getByLabelText("Open Ratatoskr at login")).toBeChecked();
  await fireEvent.click(screen.getByRole("button", { name: "Discard" }));
  await waitFor(() => expect(reads).toHaveBeenCalledTimes(3));
});
test("initial read failure never claims ready-to-sync and can be retried", async () => {
  reads.mockRejectedValue(new Error("Configuration unreadable"));
  render(App);
  expect(await screen.findByRole("alert")).toHaveTextContent(
    "No good snapshot yet",
  );
  expect(screen.queryByText("Ready to sync")).toBeNull();
  reads.mockImplementation(() => current);
  await fireEvent.click(screen.getByRole("button", { name: "Retry refresh" }));
  expect(await screen.findByRole("heading", { name: /Skills/ })).toBeVisible();
});

test("deferred catch-up waits until the hidden window is shown", async () => {
  await start();
  await fireEvent.click(screen.getByRole("button", { name: /Settings/ }));
  await fireEvent.click(
    await screen.findByLabelText("Open Ratatoskr at login"),
  );
  await screen.findByRole("button", { name: "Save" });
  await focus();
  expect(await screen.findByText("Refresh paused while editing")).toBeVisible();
  native.visible = false;
  native.shown.forEach((callback) => callback({ payload: false }));
  await fireEvent.click(screen.getByRole("button", { name: "Discard" }));
  await vi.advanceTimersByTimeAsync(120_000);
  expect(reads).toHaveBeenCalledTimes(1);
  native.visible = true;
  native.shown.forEach((callback) => callback({ payload: true }));
  await waitFor(() => expect(reads).toHaveBeenCalledTimes(2));
});

test.each([
  "Cannot reopen workspace: Cannot show main window",
  "Desktop update diagnostics failed: configuration unreadable",
])(
  "shows native app errors through the shared handler: %s",
  async (message) => {
    await start();
    await emit("app-error", message);
    expect(await screen.findByRole("alert")).toHaveTextContent(message);
    // Successful updater diagnostics must not dismiss an unrelated app failure.
    await emit("app-error", null);
    expect(screen.getByRole("alert")).toHaveTextContent(message);
  },
);

test.each(["macos", "windows"])(
  "%s close preserves unsaved edits for the native hide path",
  async (platform) => {
    HTMLDialogElement.prototype.showModal = function () {
      this.open = true;
    };
    HTMLDialogElement.prototype.close = function () {
      this.open = false;
    };
    native.platform = platform;
    await start();
    await fireEvent.click(screen.getByRole("button", { name: /Settings/ }));
    await fireEvent.click(
      await screen.findByLabelText("Open Ratatoskr at login"),
    );
    await screen.findByRole("button", { name: "Save" });
    const preventDefault = vi.fn();
    native.close.forEach((callback) => callback({ preventDefault }));
    flushSync();
    expect(preventDefault).not.toHaveBeenCalled();
    expect(
      screen.queryByRole("dialog", { name: "You have unsaved changes" }),
    ).toBeNull();
    expect(screen.getByRole("button", { name: "Save" })).toBeEnabled();
  },
);
