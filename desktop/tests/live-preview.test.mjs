import assert from "node:assert/strict";
import test from "node:test";
import { setTimeout as wait } from "node:timers/promises";
import { createLivePreview } from "../src/live-preview.ts";

test("rapid edits debounce to the latest draft", async () => {
  const loaded = [];
  const shown = [];
  const preview = createLivePreview(
    async (draft) => {
      loaded.push(draft);
      return draft;
    },
    (value) => shown.push(value),
    assert.fail,
    5,
  );
  preview.schedule("old");
  preview.schedule("latest");
  await wait(20);
  assert.deepEqual(loaded, ["latest"]);
  assert.deepEqual(shown, ["latest"]);
});

test("superseded responses and errors cannot replace the current preview", async () => {
  const requests = [];
  const shown = [];
  const errors = [];
  const preview = createLivePreview(
    (draft) =>
      new Promise((resolve, reject) =>
        requests.push({ draft, resolve, reject }),
      ),
    (value) => shown.push(value),
    (error) => errors.push(error),
    0,
  );
  preview.schedule("first");
  await wait(5);
  preview.schedule("second");
  await wait(5);
  requests[1].resolve("current");
  requests[0].reject("stale error");
  await wait(5);
  assert.deepEqual(shown, ["current"]);
  assert.deepEqual(errors, []);
  preview.schedule("third");
  await wait(5);
  preview.cancel();
  requests[2].resolve("cancelled");
  await wait(5);
  assert.deepEqual(shown, ["current"]);
});

test("current validation failures reach the editor", async () => {
  const errors = [];
  const preview = createLivePreview(
    async () => {
      throw new Error("Unclosed only block");
    },
    assert.fail,
    (failure) => errors.push(failure.message),
    0,
  );
  preview.schedule("invalid");
  await wait(10);
  assert.deepEqual(errors, ["Unclosed only block"]);
});
