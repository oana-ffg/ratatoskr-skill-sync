import { test, expect, type Page } from "@playwright/test";
import { editorFixture, fixture, instructionsFixture } from "../tests/fixture";
import { installFixture } from "./helpers";

const OPEN = "<!-- only machine=brunnr -->";
const CLOSE = "<!-- /only -->";
const BOX = `${OPEN}\nInside one.\nInside two.\n${CLOSE}`;
const FILLER = Array.from(
  { length: 150 },
  (_, line) => `Filler line ${line + 1}.`,
).join("\n");
const DOCUMENT = `# Rules\n\nBefore box.\n${BOX}\nAfter box.\n\n## Quota\n\n${FILLER}\n`;

/** Opens the instructions on `text`. `saved()` clicks Save and returns the exact draft the app wrote. */
async function open(page: Page, text = DOCUMENT) {
  const backend = instructionsFixture();
  let written: string | undefined;
  await installFixture(page, (cmd, args) => {
    if (cmd === "read_global_instructions")
      return { ...backend.document, text: written ?? text };
    if (cmd === "save_global_instructions") {
      written = String(args.markdown);
      return backend.invoke("read_workspace", {});
    }
    return backend.invoke(cmd, args);
  });
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: /Global instructions/ })
    .click();
  await expect(page.locator(".status-pill")).toHaveText("✓ Valid");
  const editor = page.getByRole("textbox", {
    name: "Global instructions editor",
  });
  return {
    editor,
    async saved() {
      await page.getByRole("button", { name: "Save", exact: true }).click();
      await expect(
        page.getByText("Unsaved changes", { exact: true }),
      ).toHaveCount(0);
      return written;
    },
    /** The app's unsaved flag: true when the draft differs from what was opened. */
    unsaved: page.getByText("Unsaved changes", { exact: true }),
  };
}

/** Clicks into the visible line with this text and moves to its start or end. */
async function caret(page: Page, text: string, side: "Home" | "End") {
  await page.locator(".cm-line", { hasText: text }).first().click();
  await page.keyboard.press(side);
}

/** Lets the debounced analysis of the current draft land before asserting on decorations. */
async function settled(page: Page) {
  await expect(page.locator(".status-pill")).toHaveText("✓ Valid");
}

const bandTexts = async (page: Page) =>
  (await page.locator(".cm-line.only-band").allInnerTexts()).map((text) =>
    text.replace(/\n$/, ""),
  );

test.describe("the toolbar stays reachable", () => {
  test("Only for… is on screen and clickable after scrolling a long file", async ({
    page,
  }) => {
    await open(page);
    const scroller = page.locator(".main-content");
    await scroller.evaluate((element) => (element.scrollTop = 1800));
    const before = await scroller.evaluate((element) => element.scrollTop);
    expect(before).toBeGreaterThan(1000);
    const button = page.getByRole("button", { name: "＋ Only for…" });
    const box = (await button.boundingBox())!;
    const shell = (await page.locator(".topbar").boundingBox())!;
    expect(box.y).toBeGreaterThanOrEqual(shell.y + shell.height);
    expect(box.y + box.height).toBeLessThan(page.viewportSize()!.height);
    await page.mouse.click(box.x + box.width / 2, box.y + box.height / 2);
    await expect(
      page.getByRole("dialog", { name: /Only include this section/ }),
    ).toBeVisible();
    expect(await scroller.evaluate((element) => element.scrollTop)).toBe(
      before,
    );
  });

  test("moving the caret up never hides it under the toolbar", async ({
    page,
  }) => {
    await open(page);
    await page.locator(".main-content").evaluate((e) => (e.scrollTop = 1800));
    await page.locator(".cm-line", { hasText: "Filler" }).nth(20).click();
    for (let press = 0; press < 60; press++)
      await page.keyboard.press("ArrowUp");
    await page.waitForTimeout(100);
    const toolbar = (await page.locator(".format-toolbar").boundingBox())!;
    const cursor = (await page.locator(".cm-cursor").first().boundingBox())!;
    expect(cursor.y).toBeGreaterThanOrEqual(toolbar.y + toolbar.height);
  });

  test("the search panel sits below the toolbar instead of under it", async ({
    page,
  }) => {
    await open(page);
    await page.locator(".main-content").evaluate((e) => (e.scrollTop = 1800));
    await page.getByRole("button", { name: "Find" }).click();
    const toolbar = (await page.locator(".format-toolbar").boundingBox())!;
    const panel = (await page.locator(".cm-panels-top").boundingBox())!;
    expect(panel.y).toBeGreaterThanOrEqual(toolbar.y + toolbar.height - 1);
  });
});

test("the Conflict editor's toolbar is not pinned, so its Find panel cannot cover it", async ({
  page,
}) => {
  const backend = instructionsFixture();
  const conflict = {
    ...fixture.workspace.plan.conflict_details[0],
    source: "/source/global/AGENTS.md",
  };
  const workspace = structuredClone(fixture.workspace);
  workspace.plan.conflict_details = [conflict];
  await installFixture(page, (cmd, args) => {
    if (cmd === "read_workspace") return workspace;
    if (cmd === "read_conflict_source") return { text: DOCUMENT };
    return backend.invoke(cmd, args);
  });
  await page.goto("/");
  await page
    .getByRole("navigation")
    .getByRole("button", { name: /Review changes/ })
    .click();
  await page.getByRole("button", { name: "Edit source…" }).first().click();
  const dialog = page.getByRole("dialog");
  const editor = dialog.getByRole("textbox", {
    name: "Source Markdown editor",
  });
  await editor.click();
  await expect(dialog.locator(".format-toolbar")).not.toHaveCSS(
    "position",
    "sticky",
  );
  await page.keyboard.press("ControlOrMeta+f");
  const toolbar = (await dialog.locator(".format-toolbar").boundingBox())!;
  const panel = (await dialog.locator(".cm-panels-top").boundingBox())!;
  expect(panel.y).toBeGreaterThanOrEqual(toolbar.y + toolbar.height);
});

test.describe("undo and redo", () => {
  test("undoing a burst of typing restores the draft and the box exactly", async ({
    page,
  }) => {
    const { saved, unsaved } = await open(page);
    await caret(page, "Inside two.", "End");
    await page.keyboard.type(" typed fast", { delay: 0 });
    await settled(page);
    await page.keyboard.press("ControlOrMeta+z");
    await settled(page);
    await expect(unsaved).toHaveCount(0);
    expect(await bandTexts(page)).toEqual(["Inside one.", "Inside two."]);
    await expect(page.getByText(/only -->/)).toHaveCount(0);
    await page.keyboard.press("ControlOrMeta+Shift+z");
    await settled(page);
    expect(await bandTexts(page)).toEqual([
      "Inside one.",
      "Inside two. typed fast",
    ]);
    expect(await saved()).toBe(
      DOCUMENT.replace("Inside two.", "Inside two. typed fast"),
    );
  });

  test("typing after an undo edits what is on screen, not a stale draft", async ({
    page,
  }) => {
    const { saved } = await open(page);
    await caret(page, "Inside two.", "End");
    await page.keyboard.type("xyz", { delay: 0 });
    await page.keyboard.press("ControlOrMeta+z");
    await page.keyboard.type("!");
    expect(await saved()).toBe(DOCUMENT.replace("Inside two.", "Inside two.!"));
  });

  test("adding a condition undoes and redoes as one step", async ({ page }) => {
    const { saved, unsaved } = await open(page);
    await page.locator(".cm-line", { hasText: "After box." }).click();
    await page.keyboard.press("Home");
    await page.keyboard.press("Shift+End");
    await page.getByRole("button", { name: "＋ Only for…" }).click();
    const dialog = page.getByRole("dialog");
    await dialog.getByRole("button", { name: "Skirnir", exact: true }).click();
    await dialog.getByRole("button", { name: "Apply condition" }).click();
    await expect(page.locator(".only-header")).toHaveCount(2);
    await settled(page);
    await page.locator(".cm-line", { hasText: "Before box." }).click();
    await page.keyboard.press("ControlOrMeta+z");
    await expect(page.locator(".only-header")).toHaveCount(1);
    await expect(unsaved).toHaveCount(0);
    await page.keyboard.press("ControlOrMeta+Shift+z");
    await expect(page.locator(".only-header")).toHaveCount(2);
    expect(await saved()).toBe(
      DOCUMENT.replace(
        "After box.",
        `<!-- only machine=skirnir -->\nAfter box.\n${CLOSE}`,
      ),
    );
  });

  test("Discard ends the undo history instead of replaying it on the reloaded file", async ({
    page,
  }) => {
    const short = `Before box.\n${BOX}\nAfter box.\n`;
    const { saved, unsaved } = await open(page, short);
    await caret(page, "Before box.", "End");
    await page.keyboard.type(" edited");
    await page.keyboard.press("ControlOrMeta+End");
    await page.keyboard.press("Backspace");
    await expect(unsaved).toBeVisible();
    await page.getByRole("button", { name: "Discard", exact: true }).click();
    await expect(unsaved).toHaveCount(0);
    await caret(page, "After box.", "End");
    await page.keyboard.press("ControlOrMeta+z");
    await page.keyboard.press("ControlOrMeta+z");
    await settled(page);
    await expect(unsaved).toHaveCount(0);
    await page.keyboard.type("!");
    expect(await saved()).toBe(short.replace("After box.", "After box.!"));
  });

  test("the marker fragment and next paragraph never leak into the box after undo", async ({
    page,
  }) => {
    await open(page);
    await caret(page, "Inside two.", "End");
    await page.keyboard.type("0123456789", { delay: 0 });
    await page.keyboard.press("ControlOrMeta+z");
    await settled(page);
    await expect(page.locator(".cm-content")).not.toContainText("/only");
    await expect(
      page.locator(".cm-line", { hasText: "After box." }),
    ).not.toHaveClass(/only-band/);
  });

  test("applying a condition deep in a long file keeps your place", async ({
    page,
  }) => {
    const long =
      `${"A long wrapped line of instructions. ".repeat(12)}\n\n`.repeat(30);
    await open(page, `${long}Pick me.\n\n${long}`);
    const scroller = page.locator(".main-content");
    const line = page.locator(".cm-line", { hasText: "Pick me." });
    await expect
      .poll(
        async () => {
          await scroller.evaluate((element) => (element.scrollTop += 400));
          return line.count();
        },
        { intervals: [50] },
      )
      .toBe(1);
    await line.scrollIntoViewIfNeeded();
    await line.click();
    await page.keyboard.press("Home");
    await page.keyboard.press("Shift+End");
    const before = (await line.boundingBox())!.y;
    await page.getByRole("button", { name: "＋ Only for…" }).click();
    const dialog = page.getByRole("dialog");
    await dialog.getByRole("button", { name: "Skirnir", exact: true }).click();
    await dialog.getByRole("button", { name: "Apply condition" }).click();
    await expect(page.locator(".only-header")).toHaveCount(1);
    await settled(page);
    await expect(line).toHaveClass(/only-band/);
    const after = (await line.boundingBox())!.y;
    expect(Math.abs(after - before)).toBeLessThan(80);
  });

  test("selecting a whole line by triple click leaves no blank line in the new box", async ({
    page,
  }) => {
    const { saved } = await open(page, "First.\nSecond.\n");
    await page
      .locator(".cm-line", { hasText: "First." })
      .click({ clickCount: 3 });
    await page.getByRole("button", { name: "＋ Only for…" }).click();
    const dialog = page.getByRole("dialog");
    await dialog.getByRole("button", { name: "Skirnir", exact: true }).click();
    await dialog.getByRole("button", { name: "Apply condition" }).click();
    await expect(page.locator(".only-header")).toHaveCount(1);
    expect(await saved()).toBe(
      `<!-- only machine=skirnir -->\nFirst.\n${CLOSE}\nSecond.\n`,
    );
  });
});

test.describe("the box edge", () => {
  test("the closing marker takes up no line of its own", async ({ page }) => {
    await open(page);
    expect(await bandTexts(page)).toEqual(["Inside one.", "Inside two."]);
  });

  test("typing and Enter at the last line stay inside the box", async ({
    page,
  }) => {
    const { saved } = await open(page);
    await caret(page, "Inside two.", "End");
    await page.keyboard.type(" more");
    await page.keyboard.press("Enter");
    await page.keyboard.type("Third.");
    expect(await saved()).toBe(
      DOCUMENT.replace("Inside two.", "Inside two. more\nThird."),
    );
  });

  test("arrow keys step straight between the box and the lines around it", async ({
    page,
  }) => {
    const short = `Before box.\n${BOX}\nAfter box.\n`;
    const { saved } = await open(page, short);
    // Each step types a marker where the caret landed, so the result shows every landing spot.
    await caret(page, "Inside two.", "End");
    await page.keyboard.press("ArrowRight");
    await page.keyboard.type("1");
    await page.keyboard.press("Home");
    await page.keyboard.press("ArrowLeft");
    await page.keyboard.type("2");
    await caret(page, "Inside one.", "Home");
    await page.keyboard.press("ArrowLeft");
    await page.keyboard.type("3");
    await page.keyboard.press("ArrowRight");
    await page.keyboard.type("4");
    await caret(page, "Inside two.", "End");
    await page.keyboard.press("ArrowDown");
    await page.keyboard.type("5");
    await page.keyboard.press("ArrowUp");
    await page.keyboard.type("6");
    expect(await saved()).toBe(
      `Before box.3\n${OPEN}\n4Inside one.\nInside two.26\n${CLOSE}\n1After box.5\n`,
    );
  });

  test("Delete on the last line and Backspace on the first do nothing", async ({
    page,
  }) => {
    const { unsaved } = await open(page);
    await caret(page, "Inside two.", "End");
    await page.keyboard.press("Delete");
    await page.keyboard.press("ControlOrMeta+Delete");
    await caret(page, "Inside one.", "Home");
    await page.keyboard.press("Backspace");
    await page.keyboard.press("ControlOrMeta+Backspace");
    await page.waitForTimeout(50);
    await expect(unsaved).toHaveCount(0);
    expect(await bandTexts(page)).toEqual(["Inside one.", "Inside two."]);
  });

  test("deleting inside the box still works up to its edge", async ({
    page,
  }) => {
    const { saved } = await open(page);
    await caret(page, "Inside one.", "End");
    await page.keyboard.press("Delete");
    expect(await saved()).toBe(
      DOCUMENT.replace("Inside one.\nInside two.", "Inside one.Inside two."),
    );
  });

  test("Backspace at the line after the box pulls that line in", async ({
    page,
  }) => {
    const { saved } = await open(page);
    await caret(page, "After box.", "Home");
    await page.keyboard.press("Backspace");
    await page.keyboard.type("|");
    expect(await saved()).toBe(
      `# Rules\n\nBefore box.\n${OPEN}\nInside one.\nInside two.|After box.\n${CLOSE}\n\n## Quota\n\n${FILLER}\n`,
    );
  });

  test("Delete at the line before the box pulls its first line out", async ({
    page,
  }) => {
    const { saved } = await open(page);
    await caret(page, "Before box.", "End");
    await page.keyboard.press("Delete");
    await page.keyboard.type("|");
    expect(await saved()).toBe(
      `# Rules\n\nBefore box.|Inside one.\n${OPEN}\nInside two.\n${CLOSE}\nAfter box.\n\n## Quota\n\n${FILLER}\n`,
    );
  });

  test("clicking the lines around the box puts the caret outside it", async ({
    page,
  }) => {
    const { saved } = await open(page);
    await caret(page, "After box.", "Home");
    await page.keyboard.type("Out ");
    expect(await saved()).toBe(
      DOCUMENT.replace("After box.", "Out After box."),
    );
  });

  test("Select all then Delete removes everything, markers included", async ({
    page,
  }) => {
    const { saved } = await open(page);
    await caret(page, "Inside one.", "Home");
    await page.keyboard.press("ControlOrMeta+a");
    await page.keyboard.press("Delete");
    expect(await saved()).toBe("");
  });

  test("a selection across the edge deletes the text and keeps the box", async ({
    page,
  }) => {
    const { saved } = await open(page);
    await caret(page, "Inside two.", "End");
    for (let press = 0; press < 4; press++)
      await page.keyboard.press("ArrowLeft");
    for (let press = 0; press < 4; press++)
      await page.keyboard.press("Shift+ArrowRight");
    for (let press = 0; press < 3; press++)
      await page.keyboard.press("Shift+ArrowRight");
    await page.keyboard.press("Delete");
    await settled(page);
    expect(await bandTexts(page)).toEqual(["Inside one.", "Inside "]);
    expect(await saved()).toBe(
      DOCUMENT.replace(
        `Inside two.\n${CLOSE}\nAfter box.`,
        `Inside \n${CLOSE}\nter box.`,
      ),
    );
  });

  test("a selection across the edge can be typed over without breaking the box", async ({
    page,
  }) => {
    const { saved } = await open(page);
    await caret(page, "Before box.", "End");
    await page.keyboard.press("ArrowLeft");
    for (let press = 0; press < 7; press++)
      await page.keyboard.press("Shift+ArrowRight");
    await page.keyboard.type("X");
    await settled(page);
    expect(await bandTexts(page)).toEqual(["e one.", "Inside two."]);
    expect(await saved()).toBe(
      DOCUMENT.replace(
        `Before box.\n${OPEN}\nInside one.`,
        `Before boxX\n${OPEN}\ne one.`,
      ),
    );
  });

  test("Enter over a selection across the edge keeps the box", async ({
    page,
  }) => {
    const { saved } = await open(page);
    await caret(page, "Inside two.", "End");
    await page.keyboard.press("ArrowLeft");
    for (let press = 0; press < 3; press++)
      await page.keyboard.press("Shift+ArrowRight");
    await page.keyboard.press("Enter");
    await settled(page);
    expect(await page.locator(".only-header").count()).toBe(1);
    expect(await saved()).toBe(
      DOCUMENT.replace(
        `Inside two.\n${CLOSE}\nAfter box.`,
        `Inside two\n\n${CLOSE}\nfter box.`,
      ),
    );
  });

  test("Bold over a selection across the edge still wraps the source text", async ({
    page,
  }) => {
    const { saved } = await open(page);
    await caret(page, "Before box.", "End");
    await page.keyboard.press("Shift+ArrowDown");
    await page.getByRole("button", { name: "Bold" }).click();
    const wrapped = (await saved())!;
    expect(wrapped.match(/<!-- only/g)).toHaveLength(1);
    expect(wrapped).toContain(`**\n${OPEN}\nInside one.**`);
  });

  test("a selection covering the whole box deletes it with its markers", async ({
    page,
  }) => {
    const { saved } = await open(page);
    await caret(page, "Before box.", "End");
    await page.keyboard.press("Shift+ArrowDown");
    await page.keyboard.press("Shift+ArrowDown");
    await page.keyboard.press("Shift+ArrowDown");
    await page.keyboard.press("Delete");
    await settled(page);
    expect(await bandTexts(page)).toEqual([]);
    const text = (await saved())!;
    expect(text).not.toContain("only");
    expect(text).toContain("Before box.");
  });

  test("a selection shows on box lines and on the caret's own line", async ({
    page,
  }) => {
    await open(page);
    // The bare end of each line: only a drawn selection can change these pixels.
    const lineEnd = async (text: string) => {
      const line = (await page
        .locator(".cm-line", { hasText: text })
        .boundingBox())!;
      return page.screenshot({
        clip: {
          x: line.x + line.width - 60,
          y: line.y + 2,
          width: 40,
          height: line.height - 4,
        },
      });
    };
    await caret(page, "After box.", "Home");
    const [inBox, beforeBox] = [
      await lineEnd("Inside one."),
      await lineEnd("Before box."),
    ];
    await page.keyboard.press("ControlOrMeta+a");
    await expect
      .poll(async () => (await lineEnd("Inside one.")).equals(inBox))
      .toBe(false);
    expect((await lineEnd("Before box.")).equals(beforeBox)).toBe(false);
    // A selection on the caret's own line, seen in the middle of its text.
    await caret(page, "Before box.", "End");
    const line = (await page
      .locator(".cm-line", { hasText: "Before box." })
      .boundingBox())!;
    const text = {
      x: line.x + 30,
      y: line.y + 4,
      width: 40,
      height: line.height - 8,
    };
    const unselected = await page.screenshot({ clip: text });
    await page.keyboard.press("Shift+Home");
    await expect
      .poll(async () =>
        (await page.screenshot({ clip: text })).equals(unselected),
      )
      .toBe(false);
  });

  test("copying a selection across the box keeps the markers", async ({
    page,
    context,
    browserName,
  }) => {
    test.skip(browserName !== "chromium", "clipboard grant is Chromium only");
    await context.grantPermissions(["clipboard-read", "clipboard-write"]);
    await open(page);
    await caret(page, "Before box.", "Home");
    for (let press = 0; press < 3; press++)
      await page.keyboard.press("Shift+ArrowDown");
    await page.keyboard.press("ControlOrMeta+c");
    const text = await page.evaluate(() => navigator.clipboard.readText());
    expect(text).toContain(OPEN);
    expect(text).toContain("Inside two.");
  });

  test("a box at the very end of the file can still be left with ArrowDown", async ({
    page,
  }) => {
    const { saved } = await open(page, `Before.\n${BOX}`);
    await caret(page, "Inside two.", "End");
    await page.keyboard.press("ControlOrMeta+End");
    await page.keyboard.type("!");
    await page.keyboard.press("ArrowDown");
    await page.keyboard.type("Below.");
    expect(await saved()).toBe(
      `Before.\n${OPEN}\nInside one.\nInside two.!\n${CLOSE}\nBelow.`,
    );
  });

  test("Insert value uses the corrected caret when the file ends in a box", async ({
    page,
  }) => {
    const { saved } = await open(page, `Before.\n${BOX}`);
    await caret(page, "Inside two.", "Home");
    await page.keyboard.press("ControlOrMeta+End");
    await page.getByRole("button", { name: "＋ Insert value…" }).click();
    await page
      .getByRole("dialog")
      .getByRole("button", { name: /Recordings archive/i })
      .click();
    await expect(page.locator(".value-pill")).toHaveCount(1);
    const text = (await saved())!;
    expect(text.indexOf("{{af:")).toBeGreaterThan(text.indexOf(OPEN));
    expect(text.indexOf("{{af:")).toBeLessThan(text.indexOf(CLOSE));
    expect(text.endsWith(CLOSE)).toBe(true);
  });

  test("a box at the very start of the file can still be left with ArrowUp", async ({
    page,
  }) => {
    const { saved } = await open(page, `${BOX}\nAfter.\n`);
    await caret(page, "Inside one.", "Home");
    await page.keyboard.press("ControlOrMeta+Home");
    await page.keyboard.type("!");
    await page.keyboard.press("Home");
    await page.keyboard.press("ArrowUp");
    await page.keyboard.type("Above.");
    expect(await saved()).toBe(
      `Above.\n${OPEN}\n!Inside one.\nInside two.\n${CLOSE}\nAfter.\n`,
    );
  });

  test("an empty box has one line to type on, and Delete from above removes it", async ({
    page,
  }) => {
    const empty = `Before box.\n${OPEN}\n\n${CLOSE}\nAfter box.\n`;
    const { saved } = await open(page, empty);
    expect(await bandTexts(page)).toEqual([""]);
    await page.locator(".cm-line.only-band").click();
    await page.keyboard.press("Backspace");
    await page.keyboard.press("Delete");
    await page.keyboard.type("Typed.");
    await settled(page);
    expect(await bandTexts(page)).toEqual(["Typed."]);
    for (let press = 0; press < 6; press++)
      await page.keyboard.press("Backspace");
    await settled(page);
    await caret(page, "Before box.", "End");
    await page.keyboard.press("Delete");
    await page.keyboard.type("|");
    expect(await saved()).toBe("Before box.|\nAfter box.\n");
  });

  test("pulling out a box's only line leaves it empty and ready to type in", async ({
    page,
  }) => {
    const { saved } = await open(
      page,
      `Before box.\n${OPEN}\nOnly line.\n${CLOSE}\nAfter box.\n`,
    );
    await caret(page, "Before box.", "End");
    await page.keyboard.press("Delete");
    await settled(page);
    expect(await bandTexts(page)).toEqual([""]);
    expect(await saved()).toBe(
      `Before box.Only line.\n${OPEN}\n\n${CLOSE}\nAfter box.\n`,
    );
  });

  test("deleting a whole line beside an edge removes just that line", async ({
    page,
  }) => {
    const { saved } = await open(
      page,
      `Before box.\n${BOX}\nAfter box.\nLast.\n`,
    );
    await caret(page, "Inside one.", "Home");
    await page.keyboard.press("ControlOrMeta+Shift+k");
    await caret(page, "After box.", "Home");
    await page.keyboard.press("ControlOrMeta+Shift+k");
    expect(await saved()).toBe(
      `Before box.\n${OPEN}\nInside two.\n${CLOSE}\nLast.\n`,
    );
  });

  test("cutting a whole line beside an edge keeps the marker on its own line", async ({
    page,
  }) => {
    const { saved } = await open(page, `Before box.\n${BOX}\nAfter box.\n`);
    await caret(page, "Inside two.", "Home");
    await page.keyboard.press("ControlOrMeta+x");
    // Before the next analysis lands: the caret must already be off the hidden line.
    await page.keyboard.press("Backspace");
    await page.keyboard.type("|");
    expect(await saved()).toBe(
      `Before box.\n${OPEN}\nInside one|\n${CLOSE}\nAfter box.\n`,
    );
  });

  test("cutting a box's only line leaves it empty", async ({ page }) => {
    const { saved } = await open(
      page,
      `Before box.\n${OPEN}\nOnly line.\n${CLOSE}\nAfter box.\n`,
    );
    await caret(page, "Only line.", "Home");
    await page.keyboard.press("ControlOrMeta+x");
    await page.keyboard.press("Backspace");
    await page.keyboard.type("|");
    expect(await saved()).toBe(
      `Before box.\n${OPEN}\n|\n${CLOSE}\nAfter box.\n`,
    );
  });

  test("cutting the line just above a box keeps the marker on its own line", async ({
    page,
  }) => {
    const { saved } = await open(
      page,
      `First.\nBefore box.\n${BOX}\nAfter box.\n`,
    );
    await caret(page, "Before box.", "Home");
    await page.keyboard.press("ControlOrMeta+x");
    await page.keyboard.press("Delete");
    await page.keyboard.type("|");
    expect(await saved()).toBe(
      `First.|Inside one.\n${OPEN}\nInside two.\n${CLOSE}\nAfter box.\n`,
    );
  });

  test("two boxes in a row stay separate", async ({ page }) => {
    const second = `<!-- only harness=codex -->\nSecond box.\n${CLOSE}`;
    const { saved } = await open(page, `${BOX}\n${second}\nAfter.\n`);
    await caret(page, "Inside two.", "End");
    await page.keyboard.press("Delete");
    await page.keyboard.press("ArrowRight");
    await page.keyboard.type("!");
    await page.keyboard.press("ArrowLeft");
    await page.keyboard.press("Backspace");
    expect(await saved()).toBe(
      `${BOX}\n<!-- only harness=codex -->\n!Second box.\n${CLOSE}\nAfter.\n`,
    );
  });

  test("a box straight after the frontmatter can still be left with ArrowUp", async ({
    page,
  }) => {
    const backend = editorFixture();
    const frontmatter = `---\nname: recordings\ndescription: Short.\n---\n`;
    backend.document.markdown = `${frontmatter}${BOX}\nAfter.\n`;
    let draft = "";
    await installFixture(page, (cmd, args) => {
      if (cmd === "analyze_skill") draft = String(args.markdown);
      return backend.invoke(cmd, args);
    });
    await page.goto("/");
    await page.getByRole("button", { name: /recordings Enabled/ }).click();
    await settled(page);
    expect(await bandTexts(page)).toEqual(["Inside one.", "Inside two."]);
    await caret(page, "Inside one.", "Home");
    await page.keyboard.press("Backspace");
    await page.keyboard.press("ArrowUp");
    await page.keyboard.type("Above.");
    await expect
      .poll(() => draft)
      .toBe(`${frontmatter}Above.\n${BOX}\nAfter.\n`);
    expect(await bandTexts(page)).toEqual(["Inside one.", "Inside two."]);
  });

  test("Markdown source mode keeps plain text editing with visible markers", async ({
    page,
  }) => {
    const { saved } = await open(page);
    await page.getByRole("button", { name: "Markdown source" }).click();
    await caret(page, "Inside two.", "End");
    await page.keyboard.press("Delete");
    expect(await saved()).toBe(
      DOCUMENT.replace(`two.\n${CLOSE}`, `two.${CLOSE}`),
    );
  });

  test("Windows line endings survive edits at every edge", async ({ page }) => {
    const crlf = `Before box.\r\n${OPEN}\r\nInside.\r\n${CLOSE}\r\nAfter box.\r\n`;
    const { saved } = await open(page, crlf);
    await caret(page, "Inside.", "End");
    await page.keyboard.press("Delete");
    await caret(page, "After box.", "Home");
    await page.keyboard.press("Backspace");
    expect(await saved()).toBe(
      `Before box.\r\n${OPEN}\r\nInside.After box.\r\n${CLOSE}\r\n`,
    );
  });
});
