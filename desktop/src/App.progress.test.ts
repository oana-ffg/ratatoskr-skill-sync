import {
  render,
  screen,
  fireEvent,
  waitFor,
  act,
} from "@testing-library/svelte";
import { tick } from "svelte";
import { test, expect, vi, afterEach } from "vitest";
import { Channel } from "@tauri-apps/api/core";
import { mockIPC, clearMocks } from "@tauri-apps/api/mocks";
import App from "./App.svelte";
import { reviewSummary } from "./api";
import { fixture, fixtureIPC } from "../tests/fixture";

function gate() {
  let resolve!: (value: unknown) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise((yes, no) => {
    resolve = yes;
    reject = no;
  });
  return { promise, resolve, reject };
}
async function settingsSave() {
  await screen.findByRole("heading", { name: /Skills/ });
  await fireEvent.click(screen.getByRole("button", { name: /Settings/ }));
  await fireEvent.click(
    await screen.findByLabelText("Open Ratatoskr at login"),
  );
  await fireEvent.click(screen.getByRole("button", { name: "Save" }));
}
afterEach(() => {
  vi.useRealTimers();
  clearMocks();
});

test.each(["success", "failure"])(
  "save progress updates, times each stage and clears on %s",
  async (outcome) => {
    const pending = gate();
    let channel!: Channel<string>;
    mockIPC((cmd, args) => {
      if (cmd === "save_local_settings") {
        channel = (args as { onProgress: Channel<string> }).onProgress;
        return pending.promise;
      }
      return fixtureIPC(cmd, args as Record<string, unknown>);
    });
    render(App);
    await settingsSave();
    const status = screen.getByRole("status", { name: "Save progress" });
    expect(status.closest(".top-actions")).toContainElement(
      screen.getByRole("button", { name: "Saving…" }),
    );
    expect(status).toHaveAttribute("aria-live", "polite");
    vi.useFakeTimers();
    await act(() => channel.onmessage("Getting the latest content…"));
    expect(status).toHaveTextContent(/^Getting the latest content…$/);
    await act(() => vi.advanceTimersByTimeAsync(2999));
    expect(status).not.toHaveTextContent(/\ds$/);
    await act(() => vi.advanceTimersByTimeAsync(1));
    expect(status).toHaveTextContent("Getting the latest content… 3s");
    await act(() => channel.onmessage("Pushing to origin/main…"));
    expect(status).toHaveTextContent(/^Pushing to origin\/main…$/);
    await act(() => vi.advanceTimersByTimeAsync(12000));
    expect(status).toHaveTextContent("Pushing to origin/main… 12s");
    await act(() => channel.onmessage("Refreshing…"));
    expect(status).toHaveTextContent(/^Refreshing…$/);
    vi.useRealTimers();
    await act(async () => {
      if (outcome === "success") pending.resolve(fixture.workspace);
      else
        pending.reject(new Error("saved and committed locally but NOT pushed"));
      await tick();
    });
    expect(status).toHaveTextContent(/^$/);
    await waitFor(() =>
      expect(screen.queryByRole("button", { name: "Saving…" })).toBeNull(),
    );
    if (outcome === "failure")
      expect(screen.getByRole("alert")).toHaveTextContent("NOT pushed");
    await act(() => channel.onmessage("Late old progress…"));
    expect(status).toHaveTextContent(/^$/);
  },
);

test("Push now uses the shared status and ignores a previous command's late stages", async () => {
  const channels: Channel<string>[] = [];
  const pending = [gate(), gate()];
  const workspace = structuredClone(fixture.workspace);
  workspace.plan.content_status.outgoing_commits = 1;
  mockIPC((cmd, args) => {
    if (cmd === "read_workspace") return workspace;
    if (cmd === "push_content") {
      const request = pending[channels.length];
      channels.push((args as { onProgress: Channel<string> }).onProgress);
      return request.promise;
    }
    return fixtureIPC(cmd, args as Record<string, unknown>);
  });
  render(App);
  await fireEvent.click(
    await screen.findByRole("button", { name: "Push now" }),
  );
  const status = screen.getByRole("status", { name: "Save progress" });
  await act(() => channels[0].onmessage("Pushing to origin/main…"));
  expect(status).toBeVisible();
  expect(status).toHaveTextContent("Pushing to origin/main…");
  await act(() => pending[0].resolve(workspace));
  expect(status).toHaveTextContent(/^$/);
  await fireEvent.click(screen.getByRole("button", { name: "Push now" }));
  await act(() => channels[1].onmessage("Pushing to origin/main…"));
  await act(() => channels[0].onmessage("Old command…"));
  expect(status).toHaveTextContent(/^Pushing to origin\/main…$/);
  await act(() => pending[1].reject(new Error("Push failed")));
  expect(status).toHaveTextContent(/^$/);
  expect(await screen.findByText("Push failed")).toBeVisible();
});

test("Agents saves several files with separate channels through the same progress line", async () => {
  const requests: {
    channel: Channel<string>;
    pending: ReturnType<typeof gate>;
  }[] = [];
  mockIPC((cmd, args) => {
    if (cmd === "save_structured_source") {
      const pending = gate();
      requests.push({
        channel: (args as { onProgress: Channel<string> }).onProgress,
        pending,
      });
      return pending.promise;
    }
    return fixtureIPC(cmd, args as Record<string, unknown>);
  });
  render(App);
  await screen.findByRole("heading", { name: /Skills/ });
  await fireEvent.click(
    screen.getByRole("button", { name: /Agents & machines/ }),
  );
  const homes = await screen.findAllByRole("textbox", { name: "Home" });
  await fireEvent.input(homes[0], { target: { value: "/first-home" } });
  await fireEvent.input(homes[1], { target: { value: "/second-home" } });
  await fireEvent.click(screen.getByRole("button", { name: "Save" }));
  const status = screen.getByRole("status", { name: "Save progress" });
  await act(() => requests[0].channel.onmessage("Committing…"));
  expect(status).toHaveTextContent("Committing…");
  await act(() => requests[0].pending.resolve(fixture.workspace));
  await waitFor(() => expect(requests).toHaveLength(2));
  expect(screen.getByRole("button", { name: "Saving…" })).toBeDisabled();
  await act(() => requests[1].channel.onmessage("Getting the latest content…"));
  await act(() => requests[0].channel.onmessage("Late first file…"));
  expect(status).toHaveTextContent(/^Getting the latest content…$/);
  await act(() => requests[1].pending.resolve(fixture.workspace));
  expect(status).toHaveTextContent(/^$/);
});

test("Apply reports what it is doing through the shared status", async () => {
  const pending = gate();
  let channel!: Channel<string>;
  const workspace = structuredClone(fixture.workspace);
  workspace.plan.conflicts = [];
  workspace.plan.stale_targets = [];
  workspace.legacy = [];
  mockIPC((cmd, args) => {
    if (cmd === "read_workspace") return workspace;
    if (cmd === "apply_workspace") {
      channel = (args as { onProgress: Channel<string> }).onProgress;
      return pending.promise;
    }
    return fixtureIPC(cmd, args as Record<string, unknown>);
  });
  render(App);
  await screen.findByRole("heading", { name: /Skills/ });
  await fireEvent.click(screen.getByRole("button", { name: /Review changes/ }));
  await fireEvent.click(
    await screen.findByRole("button", {
      name: reviewSummary(workspace).applyAction,
    }),
  );
  const status = screen.getByRole("status", { name: "Save progress" });
  await act(() => channel.onmessage("Applying changes…"));
  expect(status).toHaveTextContent(/^Applying changes…$/);
  await act(() => pending.resolve(workspace));
  expect(status).toHaveTextContent(/^$/);
});
