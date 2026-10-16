import test from "node:test";
import assert from "node:assert/strict";
import { runNightlyUpdate, runStartupUpdate } from "../src/update-policy.ts";

test("no install or restart without an available update", async () => {
  let installs = 0;
  await runNightlyUpdate(
    async () => false,
    () => true,
    async () => {
      installs++;
    },
  );
  assert.equal(installs, 0);
});
test("busy operations defer installation, then retry when idle", async () => {
  let busy = true;
  let installs = 0;
  await runNightlyUpdate(
    async () => true,
    () => !busy,
    async () => {
      installs++;
    },
  );
  assert.equal(installs, 0);
  busy = false;
  await runNightlyUpdate(
    async () => true,
    () => !busy,
    async () => {
      installs++;
    },
  );
  assert.equal(installs, 1);
});
test("edits started during a network check prevent installation", async () => {
  let idle = true;
  let installs = 0;
  await runNightlyUpdate(
    async () => {
      idle = false;
      return true;
    },
    () => idle,
    async () => {
      installs++;
    },
  );
  assert.equal(installs, 0);
});
test("check and install failures propagate to the alert/report caller", async () => {
  await assert.rejects(
    runNightlyUpdate(
      async () => {
        throw new Error("offline");
      },
      () => true,
      async () => {},
    ),
    /offline/,
  );
  await assert.rejects(
    runNightlyUpdate(
      async () => true,
      () => true,
      async () => {
        throw new Error("bad signature");
      },
    ),
    /bad signature/,
  );
});

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
