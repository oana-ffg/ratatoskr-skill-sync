// A real NSIS upgrade, on disposable Windows CI only. This deliberately shares
// the installation/instance state that makes running native journeys locally unsafe.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn, execFileSync } from "node:child_process";
import {
  copyFileSync,
  mkdirSync,
  readFileSync,
  readdirSync,
  writeFileSync,
} from "node:fs";
import { createHash } from "node:crypto";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { once } from "node:events";
import { createWorkspace } from "./workspace.mjs";
import { until } from "./webdriver.mjs";

test(
  "a real 0.2.18 NSIS upgrade restarts the hidden tray and resumes sync",
  {
    skip:
      process.env.RATATOSKR_NATIVE_E2E !== "1" || process.platform !== "win32",
    timeout: 240000,
  },
  async () => {
    const workspace = createWorkspace();
    const installation = join(workspace.root, "installed app");
    const transaction = join(workspace.root, "transaction");
    mkdirSync(join(transaction, "backup"), { recursive: true });
    const request = await fetch(
      "https://api.github.com/repos/oana-ffg/ratatoskr-skill-sync/releases/tags/v0.2.18",
      { signal: AbortSignal.timeout(30000) },
    );
    assert.ok(request.ok, `Read baseline release: ${request.status}`);
    const release = await request.json();
    const asset = release.assets.find((asset) =>
      asset.name.endsWith("windows-x86_64-setup.exe"),
    );
    assert.ok(
      asset?.digest?.startsWith("sha256:"),
      "Baseline installer needs a published digest",
    );
    const download = await fetch(asset.browser_download_url, {
      signal: AbortSignal.timeout(30000),
    });
    assert.ok(download.ok, `Download baseline installer: ${download.status}`);
    const baseline = Buffer.from(await download.arrayBuffer());
    assert.equal(
      `sha256:${createHash("sha256").update(baseline).digest("hex")}`,
      asset.digest,
    );
    const oldInstaller = join(workspace.root, "baseline.exe");
    writeFileSync(oldInstaller, baseline);
    execFileSync(oldInstaller, ["/S", `/D=${installation}`], {
      timeout: 60000,
      windowsHide: true,
    });
    const desktop = join(installation, "ratatoskr-desktop.exe");
    const helper = join(installation, "ratatoskr.exe");
    assert.match(
      execFileSync(helper, ["--version"], { encoding: "utf8" }),
      /0\.2\.18/,
    );
    const environment = { ...process.env, RATATOSKR_CONFIG: workspace.config };
    const tray = spawn(desktop, ["--background"], {
      env: environment,
      windowsHide: true,
    });
    const trayExit = once(tray, "exit");
    const since = Math.floor(Date.now() / 1000);
    const data = join(
      process.env.APPDATA,
      "io.github.oana-ffg.ratatoskr-skill-sync",
    );
    const source = fileURLToPath(
      new URL(
        "../../crates/ratatoskr-core/src/windows_update.ps1",
        import.meta.url,
      ),
    );
    const bundles = fileURLToPath(
      new URL("../../target/debug/bundle/nsis/", import.meta.url),
    );
    const candidate = readdirSync(bundles).filter((name) =>
      name.endsWith("-setup.exe"),
    );
    assert.equal(
      candidate.length,
      1,
      "Expected one freshly built NSIS candidate",
    );
    const bytes = readFileSync(join(bundles, candidate[0]));
    const version = JSON.parse(
      readFileSync(new URL("../package.json", import.meta.url)),
    ).version;
    let supervisor;
    try {
      await until(
        "baseline tray sync",
        () => {
          const run = JSON.parse(workspace.ratatoskr("job-status")).last_run;
          return run?.finished_at >= since && run.error === null;
        },
        60000,
      );
      for (const name of ["ratatoskr-desktop.exe", "ratatoskr.exe"]) {
        copyFileSync(
          join(installation, name),
          join(transaction, "backup", name),
        );
      }
      copyFileSync(source, join(transaction, "supervise.ps1"));
      writeFileSync(join(transaction, "installer.exe"), bytes);
      writeFileSync(
        join(transaction, "plan.json"),
        JSON.stringify({
          parent: tray.pid,
          app_id: JSON.parse(
            readFileSync(
              new URL("../src-tauri/tauri.conf.json", import.meta.url),
            ),
          ).identifier,
          desktop,
          data,
          version,
          sha256: createHash("sha256").update(bytes).digest("hex"),
        }),
      );
      supervisor = spawn(
        "powershell.exe",
        [
          "-NoProfile",
          "-NonInteractive",
          "-ExecutionPolicy",
          "Bypass",
          "-File",
          join(transaction, "supervise.ps1"),
          join(transaction, "plan.json"),
        ],
        { env: environment, windowsHide: true },
      );
      let errors = "";
      supervisor.stderr.on("data", (chunk) => {
        errors += chunk;
      });
      const supervisedExit = once(supervisor, "exit");
      await until(
        "supervisor readiness",
        () => {
          try {
            return readFileSync(join(transaction, "ready"), "utf8") === "ready";
          } catch (error) {
            if (error.code === "ENOENT") return false;
            throw error;
          }
        },
        20000,
      );
      // The production handoff exits the predecessor after this same handshake.
      tray.kill();
      await trayExit;
      const [code] = await supervisedExit;
      assert.equal(code, 0, errors);
      assert.equal(
        JSON.parse(readFileSync(join(data, "windows-install-result.json")))
          .error,
        null,
      );
      assert.equal(
        execFileSync(helper, ["--version"], { encoding: "utf8" }).trim(),
        `ratatoskr ${version}`,
      );
      const afterUpgrade = Math.floor(Date.now() / 1000);
      await until(
        "upgraded tray sync",
        () => {
          const run = JSON.parse(workspace.ratatoskr("job-status")).last_run;
          return run?.finished_at >= afterUpgrade && run.error === null;
        },
        120000,
      );
    } finally {
      if (supervisor?.exitCode === null) supervisor.kill();
      if (tray.exitCode === null) tray.kill();
      // Only this disposable journey's installed executable, never another app.
      execFileSync(
        "powershell.exe",
        [
          "-NoProfile",
          "-NonInteractive",
          "-Command",
          "$ErrorActionPreference='Stop'; Get-Process | Where-Object { $_.ProcessName -eq 'ratatoskr-desktop' -and $_.Path -eq $env:RATATOSKR_TEST_DESKTOP } | Stop-Process",
        ],
        {
          env: { ...process.env, RATATOSKR_TEST_DESKTOP: desktop },
          windowsHide: true,
        },
      );
    }
  },
);
