// Real signed updater and launchd lifetime acceptance. NEVER run on a personal Mac.
import { test } from "node:test";
import assert from "node:assert/strict";
import { execFileSync, spawn } from "node:child_process";
import {
  cpSync,
  copyFileSync,
  existsSync,
  mkdirSync,
  readFileSync,
  writeFileSync,
} from "node:fs";
import { createPrivateKey, X509Certificate } from "node:crypto";
import { createServer } from "node:https";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { createWorkspace } from "./workspace.mjs";
import { until } from "./webdriver.mjs";

const disabled =
  process.env.RATATOSKR_NATIVE_E2E !== "1" || process.platform !== "darwin";
const root = fileURLToPath(new URL("../../", import.meta.url));
const data = join(
  process.env.HOME || "",
  "Library/Application Support/io.github.oana-ffg.ratatoskr-skill-sync",
);
function running(executable) {
  return execFileSync("ps", ["-axo", "command="], { encoding: "utf8" })
    .split("\n")
    .some((line) => line === executable || line.startsWith(`${executable} `));
}
function writeRequest(executable, id) {
  mkdirSync(data, { recursive: true });
  writeFileSync(
    join(data, "desktop-update-request.json"),
    JSON.stringify({
      id,
      requested_at: new Date().toISOString(),
      app: executable,
    }),
  );
}
function response(id) {
  const path = join(data, "desktop-update-response.json");
  if (!existsSync(path)) return null;
  const answer = JSON.parse(readFileSync(path));
  return answer.id === id ? answer.outcome : null;
}

test(
  "macOS background creates no webview and exits, including an old Installing response",
  { skip: disabled, timeout: 60000 },
  async () => {
    const workspace = createWorkspace();
    const executable = join(root, "target/debug/ratatoskr-desktop");
    const version = JSON.parse(
      readFileSync(join(root, "desktop/package.json")),
    ).version;
    for (const oldResponse of [false, true]) {
      const id = `old-protocol-${Date.now()}`;
      if (oldResponse) {
        writeRequest(executable, id);
        writeFileSync(
          join(data, "desktop-update-response.json"),
          JSON.stringify({
            id,
            finished_at: new Date().toISOString(),
            outcome: { result: "installing", version },
          }),
        );
      }
      const child = spawn(executable, ["--background"], {
        env: { ...process.env, RATATOSKR_CONFIG: workspace.config },
        stdio: ["ignore", "pipe", "pipe"],
      });
      let output = "";
      child.stdout.on("data", (chunk) => (output += chunk));
      child.stderr.on("data", (chunk) => (output += chunk));
      try {
        // WebKit child processes carry this executable's application name. The
        // signed-install journey below also asks the embedded server for windows.
        await until("headless exit", () => child.exitCode !== null, 20000);
        assert.equal(child.exitCode, 0, output);
        assert.match(output, /Ratatoskr headless webviews: 0/);
        assert.ok(!running(executable));
        if (oldResponse) assert.equal(response(id)?.result, "installed");
      } finally {
        if (child.exitCode === null) child.kill("SIGKILL");
      }
    }
  },
);

test(
  "signed macOS background install survives its launchd parent and leaves no process",
  { skip: disabled, timeout: 180000 },
  async (t) => {
    const workspace = createWorkspace();
    const fixtures = join(root, "target/native-macos-update-fixtures");
    const installed = join(workspace.root, "installed.app");
    cpSync(join(fixtures, "successor.app"), installed, { recursive: true });
    const executable = join(installed, "Contents/MacOS/ratatoskr-desktop");
    copyFileSync(join(fixtures, "predecessor"), executable);
    const tls = join(workspace.root, "tls");
    execFileSync(join(root, "target/debug/examples/update_tls_fixture"), [tls]);
    const key = join(workspace.root, "signing.key");
    const cli = join(root, "desktop/node_modules/@tauri-apps/cli/tauri.js");
    execFileSync(process.execPath, [
      cli,
      "signer",
      "generate",
      "--ci",
      "--password",
      "",
      "--write-keys",
      key,
    ]);
    const version = JSON.parse(
      readFileSync(join(root, "desktop/package.json")),
    ).version;
    const archive = join(fixtures, "upgrade.app.tar.gz");
    execFileSync(process.execPath, [
      cli,
      "signer",
      "sign",
      "--private-key-path",
      key,
      "--password",
      "",
      "--app-version",
      version,
      archive,
    ]);
    let endpoint;
    let downloads = 0;
    let permitCheck;
    const permitted = new Promise((resolve) => (permitCheck = resolve));
    let checkStarted = false;
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
      async (request, res) => {
        if (request.url === "/latest.json") {
          checkStarted = true;
          await permitted;
          res.setHeader("Content-Type", "application/json");
          res.end(
            JSON.stringify({
              version,
              pub_date: new Date().toISOString(),
              platforms: {
                [`darwin-${process.arch === "arm64" ? "aarch64" : "x86_64"}`]: {
                  url: endpoint.replace("latest.json", "upgrade.app.tar.gz"),
                  signature: readFileSync(`${archive}.sig`, "utf8").trim(),
                },
              },
            }),
          );
        } else if (request.url === "/upgrade.app.tar.gz") {
          downloads++;
          res.end(readFileSync(archive));
        } else {
          res.writeHead(404);
          res.end();
        }
      },
    );
    await new Promise((resolve, reject) => {
      server.once("error", reject);
      server.listen(0, "127.0.0.1", resolve);
    });
    t.after(async () => {
      permitCheck();
      server.closeAllConnections();
      await new Promise((resolve) => server.close(resolve));
    });
    endpoint = `https://127.0.0.1:${server.address().port}/latest.json`;
    writeFileSync(
      workspace.config,
      readFileSync(workspace.config, "utf8").replace(
        "[updates]",
        `[updates]\nendpoint = ${JSON.stringify(endpoint)}\npublic_key = ${JSON.stringify(readFileSync(`${key}.pub`, "utf8").trim())}`,
      ),
    );
    const id = `signed-macos-${Date.now()}`;
    writeRequest(executable, id);
    const marker = join(workspace.root, "parent-finished");
    const label = `io.ratatoskr.native-parent-${process.pid}`;
    const plist = join(workspace.root, "parent.plist");
    const escape = (value) =>
      value.replaceAll("&", "&amp;").replaceAll("<", "&lt;");
    const environment = {
      RATATOSKR_CONFIG: workspace.config,
      RATATOSKR_E2E_UPDATE_CA: join(tls, "certificate.der"),
      TAURI_WEBDRIVER_PORT: "4459",
      NO_PROXY: "127.0.0.1,localhost",
    };
    writeFileSync(
      plist,
      `<?xml version="1.0"?><plist version="1.0"><dict><key>Label</key><string>${label}</string><key>ProgramArguments</key><array>${[join(root, "target/debug/examples/desktop_launch_fixture"), executable, marker].map((arg) => `<string>${escape(arg)}</string>`).join("")}</array><key>RunAtLoad</key><true/><key>EnvironmentVariables</key><dict>${Object.entries(
        environment,
      )
        .map(
          ([key, value]) =>
            `<key>${key}</key><string>${escape(value)}</string>`,
        )
        .join("")}</dict></dict></plist>`,
    );
    const domain = `gui/${process.getuid()}`;
    execFileSync("launchctl", ["bootstrap", domain, plist]);
    try {
      await until("short-lived parent to finish", () => existsSync(marker));
      await until("parent job to exit", () =>
        /state = not running/.test(
          execFileSync("launchctl", ["print", `${domain}/${label}`], {
            encoding: "utf8",
          }),
        ),
      );
      await until(
        "detached app to check after parent exit",
        () => checkStarted,
        30000,
      );
      assert.ok(
        running(executable),
        "launchd must not kill the updater with its parent group",
      );
      // The test-only server enumerates native WebViews; no session creation (which
      // would itself open a window) is allowed in this headless acceptance check.
      const status = await fetch("http://127.0.0.1:4459/status", {
        signal: AbortSignal.timeout(15000),
      });
      assert.equal(
        (await status.json()).value.ready,
        false,
        "Headless updater must have zero native WebViews",
      );
      permitCheck();
      await until(
        "installed response from headless successor",
        () => response(id)?.result === "installed",
        90000,
      );
      await until(
        "all updater processes to exit",
        () => !running(executable),
        20000,
      );
      assert.equal(downloads, 1);
      assert.equal(
        execFileSync(
          join(installed, "Contents/MacOS/ratatoskr"),
          ["--version"],
          { encoding: "utf8" },
        ).trim(),
        `ratatoskr ${version}`,
      );
    } finally {
      permitCheck();
      execFileSync("launchctl", ["bootout", `${domain}/${label}`]);
      if (running(executable))
        execFileSync("pkill", [
          "-f",
          `^${executable.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")}( |$)`,
        ]);
    }
  },
);
