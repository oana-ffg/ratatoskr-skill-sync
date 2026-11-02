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
  requests[0].reject("stale error");
  await wait(5);
  requests[1].resolve("current");
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

test("drafts typed during a slow load never start a second load at the same time", async () => {
  const started = [];
  let finish = [];
  const shown = [];
  const preview = createLivePreview(
    (draft) =>
      new Promise((resolve) => {
        started.push(draft);
        finish.push(() => resolve(draft));
      }),
    (value) => shown.push(value),
    assert.fail,
    0,
  );
  preview.schedule("first");
  await wait(5);
  for (const draft of ["a", "b", "c", "latest"]) {
    preview.schedule(draft);
    await wait(5);
  }
  assert.deepEqual(started, ["first"], "typing must not stack up loads");
  finish[0]();
  await wait(5);
  assert.deepEqual(started, ["first", "latest"], "only the newest draft runs");
  finish[1]();
  await wait(5);
  assert.deepEqual(shown, ["latest"]);
});

test("a superseded load stops before making its remaining backend calls", async () => {
  const calls = [];
  const gates = [];
  const preview = createLivePreview(
    async (draft, proceed) => {
      calls.push(`${draft}:first`);
      await new Promise((resolve) => gates.push(resolve));
      proceed();
      calls.push(`${draft}:second`);
      return draft;
    },
    () => {},
    assert.fail,
    0,
  );
  preview.schedule("old");
  await wait(5);
  preview.schedule("new");
  await wait(5);
  gates[0]();
  await wait(5);
  assert.deepEqual(calls, ["old:first", "new:first"]);
  gates[1]();
  await wait(5);
  assert.deepEqual(calls, ["old:first", "new:first", "new:second"]);
});

test("cancelling while a load runs starts nothing afterwards", async () => {
  const started = [];
  let finish;
  const preview = createLivePreview(
    (draft) =>
      new Promise((resolve) => {
        started.push(draft);
        finish = resolve;
      }),
    assert.fail,
    assert.fail,
    0,
  );
  preview.schedule("first");
  await wait(5);
  preview.schedule("queued");
  await wait(5);
  preview.cancel();
  finish("first");
  await wait(5);
  assert.deepEqual(started, ["first"]);
});
