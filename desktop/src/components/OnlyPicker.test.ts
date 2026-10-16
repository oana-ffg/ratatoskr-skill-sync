import { render, screen, fireEvent } from "@testing-library/svelte";
import { vi, test, expect, beforeAll } from "vitest";
import OnlyPicker from "./OnlyPicker.svelte";
beforeAll(() => {
  HTMLDialogElement.prototype.showModal = function () {
    this.open = true;
  };
  HTMLDialogElement.prototype.close = function () {
    this.open = false;
  };
});
test("picks structured conditions with a plain-language summary and keyboard focusable buttons", async () => {
  const apply = vi.fn();
  render(OnlyPicker, {
    agents: ["codex", "claude"],
    machines: ["brunnr"],
    onapply: apply,
    onclose: vi.fn(),
  });
  expect(
    screen.getByRole("button", { name: "Apply condition" }),
  ).toBeDisabled();
  await fireEvent.click(screen.getByRole("button", { name: "Codex" }));
  expect(
    screen.getByText(/Shows for: Codex on every system, every machine/),
  ).toBeVisible();
  await fireEvent.click(screen.getByRole("button", { name: "Windows" }));
  await fireEvent.click(
    screen.getByRole("button", { name: "Apply condition" }),
  );
  expect(apply).toHaveBeenCalledWith({ harness: ["codex"], os: ["windows"] });
});
