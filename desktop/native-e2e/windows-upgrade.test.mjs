// Disposable Windows CI only: real signed downloads, app handoff and NSIS failure.
import { test } from "node:test";
import assert from "node:assert/strict";
import { spawn, spawnSync, execFileSync } from "node:child_process";
import {
  copyFileSync,
  createWriteStream,
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
  readdirSync,
} from "node:fs";
import { createServer } from "node:https";
import { createPrivateKey, X509Certificate } from "node:crypto";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createWorkspace } from "./workspace.mjs";
import { until } from "./webdriver.mjs";

test(
  "signed Windows updates resume sync after both install success and NSIS abort",
  {
    skip:
      process.env.RATATOSKR_NATIVE_E2E !== "1" || process.platform !== "win32",
    timeout: 300000,
  },
  async (t) => {
    const workspace = createWorkspace();
    const root = fileURLToPath(new URL("../../", import.meta.url));
    const fixtures = join(root, "target/native-update-fixtures");
    const { version, failedVersion } = JSON.parse(
      readFileSync(join(fixtures, "versions.json")),
    );
    const installation = join(workspace.root, "installed app");
    const tls = join(workspace.root, "tls");
    const results = join(root, "desktop/test-results/native");
    mkdirSync(results, { recursive: true });
    execFileSync(
      join(root, "target/debug/examples/update_tls_fixture.exe"),
      [tls],
      { timeout: 10000 },
    );
    const tauri = join(root, "desktop/node_modules/@tauri-apps/cli/tauri.js");
    const key = join(workspace.root, "update-signing.key");
    const cli = (args) =>
      execFileSync(process.execPath, [tauri, ...args], {
        encoding: "utf8",
        timeout: 30000,
      });
    cli(["signer", "generate", "--ci", "--password", "", "--write-keys", key]);
    const publicKey = readFileSync(`${key}.pub`, "utf8").trim();
    function payload(name, version) {
      const path = join(workspace.root, name);
      copyFileSync(join(fixtures, name), path);
      cli([
        "signer",
        "sign",
        "--private-key-path",
        key,
        "--password",
        "",
        "--app-version",
        version,
        path,
      ]);
      return {
        path,
        version,
        bytes: readFileSync(path),
        signature: readFileSync(`${path}.sig`, "utf8").trim(),
      };
    }
    const upgrade = payload("upgrade.exe", version);
    const abort = payload("abort.exe", failedVersion);
    let selected = upgrade;
    let downloads = 0;
    let endpoint;
    const server = createServer(
      {
        key: createPrivateKey({
          key: readFileSync(join(tls, "private.der")),
          format: "der",
          type: "pkcs8",
        }).export({ format: "pem", type: "pkcs8" }),
        cert: new X509Certificate(
          readFileSync(join(tls, "certificate.der")),
        ).toString(),
      },
      (request, response) => {
        if (request.url === "/latest.json") {
          response.setHeader("Content-Type", "application/json");
          const artifact = {
            url: endpoint.replace("latest.json", "installer.exe"),
            signature: selected.signature,
          };
          response.end(
            JSON.stringify({
              version: selected.version,
              pub_date: new Date().toISOString(),
              platforms: {
                "windows-x86_64": artifact,
                "windows-x86_64-nsis": artifact,
              },
            }),
          );
        } else if (request.url === "/installer.exe") {
          downloads++;
          response.end(selected.bytes);
        } else {
          response.writeHead(404);
          response.end();
        }
      },
    );
    await new Promise((resolve, reject) => {
      server.once("error", reject);
      server.listen(0, "127.0.0.1", resolve);
    });
    t.after(async () => {
      server.closeAllConnections();
      await new Promise((resolve, reject) =>
        server.close((error) => (error ? reject(error) : resolve())),
      );
    });
    endpoint = `https://127.0.0.1:${server.address().port}/latest.json`;
    const config = readFileSync(workspace.config, "utf8");
    assert.ok(config.includes("[updates]"));
    writeFileSync(
      workspace.config,
      config.replace(
        "[updates]",
        `[updates]\nendpoint = ${JSON.stringify(endpoint)}\npublic_key = ${JSON.stringify(publicKey)}`,
      ),
    );
    // NSIS only accepts /D unquoted and last, so Node must not quote the space.
    execFileSync(upgrade.path, ["/S", `/D=${installation}`], {
      timeout: 60000,
      windowsHide: true,
      windowsVerbatimArguments: true,
    });
    const desktop = join(installation, "ratatoskr-desktop.exe");
    const helper = join(installation, "ratatoskr.exe");
    copyFileSync(join(fixtures, "predecessor.exe"), desktop);
    const environment = {
      ...process.env,
      RATATOSKR_CONFIG: workspace.config,
      RATATOSKR_E2E_UPDATE_CA: join(tls, "certificate.der"),
      NO_PROXY: "127.0.0.1,localhost",
    };
    const log = createWriteStream(join(results, "signed-update.log"));
    const predecessor = spawn(desktop, ["--background"], {
      env: environment,
      windowsHide: true,
      stdio: ["ignore", "pipe", "pipe"],
    });
    predecessor.stdout.pipe(log);
    predecessor.stderr.pipe(log);
    function helperCommand(...args) {
      const result = spawnSync(
        helper,
        ["--config", workspace.config, "--json", ...args],
        {
          env: environment,
          encoding: "utf8",
          timeout: 10000,
          windowsHide: true,
        },
      );
      assert.ifError(result.error);
      assert.ok(result.status === 0 || result.status === 1, result.stderr);
      return result;
    }
    const job = () => JSON.parse(workspace.ratatoskr("job-status"));
    const data = join(
      process.env.APPDATA,
      "io.github.oana-ffg.ratatoskr-skill-sync",
    );
    function readOptional(name) {
      try {
        return JSON.parse(readFileSync(join(data, name), "utf8"));
      } catch (error) {
        if (error.code === "ENOENT") return null;
        throw error;
      }
    }
    // Poll the request files, not the installed CLI: running that executable
    // during NSIS replacement would introduce an unrelated file-lock race.
    const status = () => {
      const request = readOptional("desktop-update-request.json");
      const response = readOptional("desktop-update-response.json");
      return {
        request,
        response: response?.id === request?.id ? response : null,
      };
    };
    let failure;
    try {
      const initial = await until(
        "predecessor sync",
        () => {
          const run = job().last_run;
          return run?.finished_at && run.error === null && run;
        },
        60000,
      );
      assert.equal(helperCommand("update", "--desktop").status, 0);
      await until(
        "the signed upgrade to settle",
        () => status().response?.outcome.result === "installed",
        120000,
      );
      assert.equal(
        predecessor.exitCode,
        0,
        "The app must exit through the real updater handoff",
      );
      assert.equal(
        execFileSync(helper, ["--version"], {
          encoding: "utf8",
          timeout: 10000,
        }).trim(),
        `ratatoskr ${version}`,
      );
      const upgraded = await until(
        "the upgraded tray sync",
        () => {
          const state = job();
          return (
            state.last_run?.finished_at > initial.finished_at &&
            state.last_run.error === null &&
            state
          );
        },
        90000,
      );
      assert.equal(upgraded.desktop_update_error, null);
      selected = abort;
      assert.equal(helperCommand("update", "--desktop").status, 0);
      await until(
        "the aborted install to settle after tray recovery",
        () => status().response?.outcome.result === "failed",
        120000,
      );
      const outcome = JSON.parse(
        readFileSync(join(data, "windows-install-result.json")),
      );
      assert.equal(outcome.version, failedVersion);
      assert.ok(outcome.error, "A real NSIS abort must be reported");
      assert.equal(
        execFileSync(helper, ["--version"], {
          encoding: "utf8",
          timeout: 10000,
        }).trim(),
        `ratatoskr ${version}`,
      );
      workspace.publish("Verify sync after installer abort", {
        "skills/after-abort/SKILL.md":
          "---\nname: after-abort\ndescription: Published after an installer abort.\n---\nRecovered sync.\n",
        "skills/after-abort/skill.toml": "",
      });
      const staged = workspace.staged("codex/skills/after-abort/SKILL.md");
      await until(
        "published content to sync after the failed installer",
        () => existsSync(staged),
        120000,
      );
      assert.match(readFileSync(staged, "utf8"), /Recovered sync\./);
      const recovered = job();
      assert.equal(recovered.last_run.error, null);
      assert.ok(
        recovered.desktop_update_error,
        "Sync success must preserve the installer failure",
      );
      assert.equal(
        downloads,
        2,
        "Both updates must download through Tauri, not a test-side installer handoff",
      );
    } catch (error) {
      failure = error;
      try {
        const diagnostics = [job(), status()];
        for (const folder of readdirSync(data).filter((name) =>
          name.startsWith("windows-install-"),
        )) {
          const path = join(data, folder, "supervisor.log");
          if (existsSync(path))
            diagnostics.push({ folder, log: readFileSync(path, "utf8") });
        }
        writeFileSync(
          join(results, "signed-update-diagnostics.json"),
          JSON.stringify(diagnostics, null, 2),
        );
      } catch (diagnosticError) {
        error.message += `\nRecording diagnostics failed: ${diagnosticError.message}`;
      }
      throw error;
    } finally {
      try {
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
            timeout: 10000,
          },
        );
      } catch (cleanupError) {
        if (failure)
          failure.message += `\nStopping the fixture desktop failed: ${cleanupError.message}`;
        else throw cleanupError;
      } finally {
        log.end();
      }
    }
  },
);
