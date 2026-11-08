// Disposable CI only: a real app archive and a lower-version updater predecessor.
import { execFileSync } from "node:child_process";
import { cpSync, copyFileSync, mkdirSync, readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";
if (process.platform !== "darwin" || process.env.CI !== "true")
  throw new Error("Disposable macOS CI only");
const root = fileURLToPath(new URL("../../", import.meta.url));
const desktop = join(root, "desktop");
const cli = join(desktop, "node_modules/@tauri-apps/cli/tauri.js");
const fixtures = join(root, "target/native-macos-update-fixtures");
const bundle = join(root, "target/debug/bundle/macos/Ratatoskr Skill Sync.app");
mkdirSync(fixtures, { recursive: true });
const version = JSON.parse(readFileSync(join(desktop, "package.json"))).version;
function build(version, bundles) {
  execFileSync(
    process.execPath,
    [
      cli,
      "build",
      "--debug",
      "--features",
      "e2e-webdriver",
      ...(bundles ? ["--bundles", "app"] : ["--no-bundle"]),
      "--config",
      JSON.stringify({
        version,
        build: {
          beforeBuildCommand:
            "npm run prepare:helper -- --debug && npm run build",
        },
      }),
    ],
    { cwd: desktop, stdio: "inherit" },
  );
}
build(version, true);
cpSync(bundle, join(fixtures, "successor.app"), { recursive: true });
execFileSync("tar", [
  "-czf",
  join(fixtures, "upgrade.app.tar.gz"),
  "-C",
  join(root, "target/debug/bundle/macos"),
  "Ratatoskr Skill Sync.app",
]);
build("0.0.0", false);
copyFileSync(
  join(root, "target/debug/ratatoskr-desktop"),
  join(fixtures, "predecessor"),
);
// Restore the normal native journeys' executable after constructing the predecessor.
copyFileSync(
  join(fixtures, "successor.app/Contents/MacOS/ratatoskr-desktop"),
  join(root, "target/debug/ratatoskr-desktop"),
);
execFileSync(
  "cargo",
  ["build", "--locked", "-p", "ratatoskr-core", "--examples"],
  { cwd: root, stdio: "inherit" },
);
