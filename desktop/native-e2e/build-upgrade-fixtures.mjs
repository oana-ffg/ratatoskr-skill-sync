// CI only: two real NSIS payloads and a lower-version predecessor from this code.
import { execFileSync } from "node:child_process";
import {
  mkdirSync,
  readFileSync,
  readdirSync,
  copyFileSync,
  writeFileSync,
} from "node:fs";
import { fileURLToPath } from "node:url";
import { join } from "node:path";

if (process.platform !== "win32" || process.env.CI !== "true")
  throw new Error("Disposable Windows CI only");
const root = fileURLToPath(new URL("../../", import.meta.url));
const desktop = join(root, "desktop");
const cli = join(desktop, "node_modules/@tauri-apps/cli/tauri.js");
const fixtures = join(root, "target/native-update-fixtures");
mkdirSync(fixtures, { recursive: true });
const version = JSON.parse(readFileSync(join(desktop, "package.json"))).version;
const [major, minor, patch] = version.split("-")[0].split(".").map(Number);
const failedVersion = `${major}.${minor}.${patch + 1}`;
const bundle = join(root, "target/debug/bundle/nsis");
function build(version, failing, bundles) {
  const configuration = {
    version,
    build: {
      beforeBuildCommand: "npm run prepare:helper -- --debug && npm run build",
    },
    ...(failing
      ? {
          bundle: {
            windows: {
              nsis: {
                installerHooks: join(desktop, "native-e2e/abort-install.nsh"),
              },
            },
          },
        }
      : {}),
  };
  execFileSync(
    process.execPath,
    [
      cli,
      "build",
      "--debug",
      "--features",
      "e2e-webdriver",
      ...(bundles ? ["--bundles", "nsis"] : ["--no-bundle"]),
      "--config",
      JSON.stringify(configuration),
    ],
    {
      cwd: desktop,
      stdio: "inherit",
    },
  );
  if (bundles) {
    const installer = readdirSync(bundle).find(
      (name) => name.includes(`_${version}_`) && name.endsWith("-setup.exe"),
    );
    if (!installer) throw new Error(`Missing NSIS candidate for ${version}`);
    copyFileSync(
      join(bundle, installer),
      join(fixtures, failing ? "abort.exe" : "upgrade.exe"),
    );
  }
}
build(version, false, true);
build(failedVersion, true, true);
build("0.0.0", false, false);
copyFileSync(
  join(root, "target/debug/ratatoskr-desktop.exe"),
  join(fixtures, "predecessor.exe"),
);
writeFileSync(
  join(fixtures, "versions.json"),
  JSON.stringify({ version, failedVersion }),
);
execFileSync(
  "cargo",
  [
    "build",
    "--locked",
    "-p",
    "ratatoskr-core",
    "--example",
    "update_tls_fixture",
  ],
  { cwd: root, stdio: "inherit" },
);
execFileSync("cargo", ["build", "--locked", "-p", "ratatoskr-skill-sync"], {
  cwd: root,
  stdio: "inherit",
});
