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

test("only macOS can destroy the main window; Windows retains close-to-tray", () => {
  assert.equal(destroyGrants("windows").length, 0);
  const macos = destroyGrants("macOS");
  assert.equal(macos.length, 1);
  assert.deepEqual(macos[0].platforms, ["macOS"]);
  assert.deepEqual(macos[0].windows, ["main"]);
});
