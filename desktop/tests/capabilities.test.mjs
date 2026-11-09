import test from "node:test";
import assert from "node:assert/strict";
import { readdirSync, readFileSync } from "node:fs";

const directory = new URL("../src-tauri/capabilities/", import.meta.url);
const capabilities = readdirSync(directory)
  .filter((file) => file.endsWith(".json"))
  .map((file) => JSON.parse(readFileSync(new URL(file, directory), "utf8")));

function destroyGrants(platform) {
  return capabilities.filter(
    (capability) =>
      (!capability.platforms || capability.platforms.includes(platform)) &&
      capability.permissions.some(
        (permission) =>
          (typeof permission === "string"
            ? permission
            : permission.identifier) === "core:window:allow-destroy",
      ),
  );
}

test("macOS and Windows can destroy only the guarded main window", () => {
  for (const platform of ["macOS", "windows"]) {
    const grants = destroyGrants(platform);
    assert.equal(grants.length, 1);
    assert.deepEqual(grants[0].platforms, ["macOS", "windows"]);
    assert.deepEqual(grants[0].windows, ["main"]);
  }
  assert.equal(destroyGrants("linux").length, 0);
});
