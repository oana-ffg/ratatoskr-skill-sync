import test from "node:test";
import assert from "node:assert/strict";
import { runStartupUpdate } from "../src/update-policy.ts";

test("startup waits for the update decision and installation before enabling the workspace", async () => {
  const events = [];
  let finish;
  const installed = new Promise((resolve) => {
    finish = resolve;
  });
  const startup = runStartupUpdate(
    async () => {
      events.push("check");
      await installed;
      events.push("installed");
    },
    async () => {
      events.push("interactive");
    },
  );
  assert.deepEqual(events, ["check"]);
  finish();
  await startup;
  assert.deepEqual(events, ["check", "installed", "interactive"]);
});
test("startup reports check or signature errors and restores access to the workspace", async () => {
  for (const failure of ["offline", "bad signature"]) {
    let ready = false;
    await assert.rejects(
      runStartupUpdate(
        async () => {
          throw new Error(failure);
        },
        async () => {
          ready = true;
        },
      ),
      new RegExp(failure),
    );
    assert.equal(ready, true);
  }
});
