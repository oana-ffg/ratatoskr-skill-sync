import {
  render,
  screen,
  fireEvent,
  waitFor,
  within,
} from "@testing-library/svelte";
import { test, expect, vi } from "vitest";
import { mockIPC } from "@tauri-apps/api/mocks";
import type { StructuredSource } from "../api";
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

test("saving unrelated profile edits preserves an explicit project-agent subset", async () => {
  const saved: StructuredSource[] = [];
  mockIPC((cmd, args) => {
    if (cmd === "save_structured_source") {
      saved.push(JSON.parse(JSON.stringify(args)) as StructuredSource);
      return fixture.workspace;
    }
    const result = fixtureIPC(cmd, args as Record<string, unknown>);
    if (
      cmd === "read_structured_source" &&
      String((args as { file: string }).file).startsWith("machines/")
    ) {
      const source = structuredClone(result) as StructuredSource;
      source.values.project_harnesses = ["codex"];
      return source;
    }
    return result;
  });
  const { component } = render(Agents, {
    workspace: fixture.workspace,
    onupdate: vi.fn(),
    ondirty: vi.fn(),
    onerror: vi.fn(),
  });
  const homes = await screen.findAllByRole("textbox", { name: "Home" });
  await fireEvent.input(homes[0], { target: { value: "/changed-home" } });
  await component.save();
  expect(saved).toHaveLength(1);
  expect(saved[0].values.project_harnesses).toEqual(["codex"]);
  expect(saved[0].values.home).toBe("/changed-home");
});

test("project-agent controls support disabling all project output", async () => {
  let saved: StructuredSource | undefined;
  mockIPC((cmd, args) => {
    if (cmd === "save_structured_source") {
      saved = JSON.parse(JSON.stringify(args)) as StructuredSource;
      return fixture.workspace;
    }
    return fixtureIPC(cmd, args as Record<string, unknown>);
  });
  const { component } = render(Agents, {
    workspace: fixture.workspace,
    onupdate: vi.fn(),
    ondirty: vi.fn(),
    onerror: vi.fn(),
  });
  const all = (
    await screen.findAllByRole("checkbox", {
      name: "Use all enabled agents for project skills",
    })
  )[0];
  expect(all).toBeChecked();
  await fireEvent.click(all);
  const projectAgents = within(
    screen.getByRole("group", { name: "Project agents" }),
  );
  await fireEvent.click(projectAgents.getByRole("button", { name: "Codex" }));
  await fireEvent.click(
    projectAgents.getByRole("button", { name: "Claude Code" }),
  );
  await component.save();
  expect(saved?.values.project_harnesses).toEqual([]);
});
