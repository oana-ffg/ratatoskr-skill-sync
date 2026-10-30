// Minimal W3C WebDriver client for the app's embedded test-only server. The
// suite needs a handful of commands, so it avoids WebdriverIO's browser-driver
// downloaders and install scripts.
import { writeFile } from "node:fs/promises";

const ELEMENT = "element-6066-11e4-a52e-4f735466cecf";
// Every request has a deadline, so a stalled server cannot outlive a wait.
const REQUEST_TIMEOUT = 15000;

/** Literal for an XPath expression; the test text never contains both quote kinds. */
export function xpathString(text) {
  return text.includes("'") ? `"${text}"` : `'${text}'`;
}

/** Polls `check` until it returns a truthy value, reporting the last failure on timeout. */
export async function until(what, check, timeout = 20000) {
  const deadline = Date.now() + timeout;
  let failure;
  for (;;) {
    try {
      const value = await check();
      if (value) return value;
      failure = undefined;
    } catch (error) {
      failure = error;
    }
    if (Date.now() > deadline) {
      throw new Error(
        `Timed out after ${timeout} ms waiting for ${what}` +
          (failure ? `: ${failure.message}` : ""),
        { cause: failure },
      );
    }
    await new Promise((resolve) => setTimeout(resolve, 250));
  }
}

export class Session {
  constructor(base, id) {
    this.base = base;
    this.id = id;
  }

  /** Waits for the server inside a freshly launched app, then opens a session. */
  static async start(port, isRunning) {
    const base = `http://127.0.0.1:${port}`;
    await until(
      "the embedded WebDriver server",
      async () => {
        if (!isRunning()) throw new Error("The app exited during startup");
        const response = await fetch(`${base}/status`, {
          signal: AbortSignal.timeout(REQUEST_TIMEOUT),
        });
        return response.ok && (await response.json()).value?.ready;
      },
      60000,
    );
    const { sessionId } = await request(base, "POST", "/session", {
      capabilities: {},
    });
    return new Session(base, sessionId);
  }

  command(method, path, body) {
    return request(this.base, method, `/session/${this.id}${path}`, body);
  }

  async find(xpath) {
    const found = await this.command("POST", "/elements", {
      using: "xpath",
      value: xpath,
    });
    for (const element of found) {
      const id = element[ELEMENT];
      if (await this.command("GET", `/element/${id}/displayed`)) return id;
    }
    return null;
  }

  /** The first displayed element matching `xpath`, waiting for it to appear. */
  element(xpath, timeout) {
    return until(xpath, () => this.find(xpath), timeout);
  }

  async enabled(xpath) {
    const id = await this.element(xpath);
    return (await this.command("GET", `/element/${id}/enabled`)) ? id : null;
  }

  async click(xpath) {
    const id = await until(`${xpath} to be enabled`, () => this.enabled(xpath));
    await this.command("POST", `/element/${id}/click`, {});
  }

  async type(xpath, text) {
    const id = await this.element(xpath);
    await this.command("POST", `/element/${id}/value`, { text });
  }

  /** Text of every element matching `xpath`, without waiting. */
  async texts(xpath) {
    const found = await this.command("POST", "/elements", {
      using: "xpath",
      value: xpath,
    });
    return Promise.all(
      found.map((element) =>
        this.command("GET", `/element/${element[ELEMENT]}/text`),
      ),
    );
  }

  async screenshot(file) {
    await writeFile(
      file,
      Buffer.from(await this.command("GET", "/screenshot"), "base64"),
    );
  }

  end() {
    return this.command("DELETE", "");
  }
}

async function request(base, method, path, body) {
  const response = await fetch(`${base}${path}`, {
    method,
    headers: { "content-type": "application/json" },
    body: body === undefined ? undefined : JSON.stringify(body),
    signal: AbortSignal.timeout(REQUEST_TIMEOUT),
  });
  const payload = await response.json();
  if (!response.ok || payload.value?.error) {
    throw new Error(
      `${method} ${path}: ${payload.value?.error ?? response.status} ${payload.value?.message ?? ""}`,
    );
  }
  return payload.value;
}
