import { render, screen, fireEvent, waitFor } from "@testing-library/svelte";
import { test, expect, vi } from "vitest";
import { mockIPC } from "@tauri-apps/api/mocks";
import Agents from "./Agents.svelte";
import { fixture, fixtureIPC } from "../../tests/fixture";

test("machine paths use the native picker, cancellation preserves values, and errors surface", async () => {
  let result: string | null | Error = null;
  mockIPC((cmd, args) => {
    if (cmd === "choose_folder") {
      if (result instanceof Error) throw result;
      return result;
    }
    return fixtureIPC(cmd, args as Record<string, unknown>);
  });
  const onerror = vi.fn();
  render(Agents, {
    workspace: fixture.workspace,
    onupdate: vi.fn(),
    ondirty: vi.fn(),
    onerror,
  });
  const button = (
    await screen.findAllByRole("button", { name: "Choose Home folder" })
  )[0];
  const field = screen.getAllByRole("textbox", {
    name: "Home",
  })[0];
  expect(field).toHaveValue("/fixture-home");
  await fireEvent.click(button);
  expect(field).toHaveValue("/fixture-home");
  result = "/chosen/home";
  await fireEvent.click(button);
  await waitFor(() => expect(field).toHaveValue("/chosen/home"));
  result = new Error("Folder dialog failed");
  await fireEvent.click(button);
  await waitFor(() => expect(onerror).toHaveBeenCalledWith(result));
});

test("capabilities can be removed and added as chips", async () => {
  mockIPC((cmd, args) => fixtureIPC(cmd, args as Record<string, unknown>));
  render(Agents, {
    workspace: fixture.workspace,
    onupdate: vi.fn(),
    ondirty: vi.fn(),
    onerror: vi.fn(),
  });
  const remove = (
    await screen.findAllByRole("button", {
      name: "Remove Transcription capability",
    })
  )[0];
  await fireEvent.click(remove);
  const names = screen.getAllByRole("textbox", { name: "New capability" });
  await fireEvent.input(names[0], { target: { value: "audio" } });
  await fireEvent.click(
    screen.getAllByRole("button", { name: "Add capability" })[0],
  );
  expect(
    await screen.findByRole("button", { name: "Remove Audio capability" }),
  ).toBeVisible();
});
