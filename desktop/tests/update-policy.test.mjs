import test from "node:test";
import assert from "node:assert/strict";
import { runNightlyUpdate, runStartupUpdate, updateStatusText, hasUnsavedEdits } from "../src/update-policy.ts";

test("override status is visible and official defaults are labelled", () => {
  const status = { endpoint: "https://example.com/latest.json", automatic: true, time: "00:00", override_active: false };
  assert.match(updateStatusText(status), /Official signed updates/);
  assert.match(updateStatusText({ ...status, override_active: true }), /Custom update override active/);
  assert.match(updateStatusText({ ...status, automatic: false }), /Automatic updates off/);
});
test("no install or restart without an available update", async () => {
  let installs = 0;
  await runNightlyUpdate(async () => false, () => true, async () => { installs++; });
  assert.equal(installs, 0);
});
function cleanEditors() {
  return {
    configuration: { text: "config", saved: "config" },
    source: { text: "source", saved: "source" },
    skill: { markdown: "skill", savedMarkdown: "skill", metadata: "metadata", savedMetadata: "metadata" },
    newSkill: { name: "", markdown: "" },
    schedule: { automatic: true, savedAutomatic: true, time: "00:00", savedTime: "00:00" },
  };
}
test("each unsaved editor defers installation, saving permits the idle retry", async () => {
  assert.equal(hasUnsavedEdits(cleanEditors()), false);
  for (const edit of [
    s => { s.configuration.text += "edited"; },
    s => { s.source.text += "edited"; },
    s => { s.skill.markdown += "edited"; },
    s => { s.skill.metadata += "edited"; },
    s => { s.newSkill.name = "new-skill"; },
    s => { s.newSkill.markdown = "draft"; },
    s => { s.schedule.automatic = false; },
    s => { s.schedule.time = "01:00"; },
  ]) {
    let state = cleanEditors();
    edit(state);
    let installs = 0;
    const idle = () => !hasUnsavedEdits(state);
    const install = async () => { installs++; };
    await runNightlyUpdate(async () => true, idle, install);
    assert.equal(installs, 0);
    state = cleanEditors();
    await runNightlyUpdate(async () => true, idle, install);
    assert.equal(installs, 1);
  }
});
test("busy operations defer installation, then retry when idle", async () => {
  let busy = true;
  let installs = 0;
  await runNightlyUpdate(async () => true, () => !busy, async () => { installs++; });
  assert.equal(installs, 0);
  busy = false;
  await runNightlyUpdate(async () => true, () => !busy, async () => { installs++; });
  assert.equal(installs, 1);
});
test("edits started during a network check prevent installation", async () => {
  let idle = true;
  let installs = 0;
  await runNightlyUpdate(async () => { idle = false; return true; }, () => idle, async () => { installs++; });
  assert.equal(installs, 0);
});
test("check and install failures propagate to the alert/report caller", async () => {
  await assert.rejects(runNightlyUpdate(async () => { throw new Error("offline"); }, () => true, async () => {}), /offline/);
  await assert.rejects(runNightlyUpdate(async () => true, () => true, async () => { throw new Error("bad signature"); }), /bad signature/);
});

test("startup waits for the update decision and installation before enabling the workspace", async () => {
  const events = [];
  let finish;
  const installed = new Promise(resolve => { finish = resolve; });
  const startup = runStartupUpdate(async () => {
    events.push("check");
    await installed;
    events.push("installed");
  }, async () => { events.push("interactive"); });
  assert.deepEqual(events, ["check"]);
  finish();
  await startup;
  assert.deepEqual(events, ["check", "installed", "interactive"]);
});
test("startup reports check or signature errors and restores access to the workspace", async () => {
  for (const failure of ["offline", "bad signature"]) {
    let ready = false;
    await assert.rejects(runStartupUpdate(async () => { throw new Error(failure); }, async () => { ready = true; }), new RegExp(failure));
    assert.equal(ready, true);
  }
});
