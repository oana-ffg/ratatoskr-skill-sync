import { spawnSync } from "node:child_process";
const result = spawnSync(
  "cargo",
  ["build", "--locked", "-p", "ratatoskr-core", "--example", "desktop_fixture"],
  { cwd: new URL("../..", import.meta.url), stdio: "inherit" },
);
if (result.error) throw result.error;
process.exit(result.status ?? 1);
