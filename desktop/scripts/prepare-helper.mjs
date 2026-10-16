import { spawnSync } from "node:child_process";
import { mkdirSync, copyFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "../..");
function run(command, args) {
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: "utf8",
    stdio: ["ignore", "pipe", "inherit"],
  });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
  return result.stdout.trim();
}
const triple =
  process.env.TAURI_ENV_TARGET_TRIPLE ||
  run("rustc", ["-vV"])
    .split("\n")
    .find((line) => line.startsWith("host: "))
    .slice(6);
const release = !process.argv.includes("--debug");
run("cargo", [
  "build",
  "--locked",
  "-p",
  "ratatoskr-skill-sync",
  "--target",
  triple,
  ...(release ? ["--release"] : []),
]);
const extension = triple.includes("windows") ? ".exe" : "";
const source = resolve(
  process.env.CARGO_TARGET_DIR || resolve(root, "target"),
  triple,
  release ? "release" : "debug",
  `ratatoskr${extension}`,
);
const destination = resolve(
  root,
  "desktop/src-tauri/binaries",
  `ratatoskr-${triple}${extension}`,
);
mkdirSync(dirname(destination), { recursive: true });
copyFileSync(source, destination);
console.log(`Prepared helper for ${triple}`);
