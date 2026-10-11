import { readFile } from "node:fs/promises";

import { expect, test } from "./fixtures.js";

test("combat buttons activate through the keyboard", async ({ page, appUrl }) => {
  await page.goto(`${appUrl}?scenario=training&seed=42`);
  await expect(page.getByRole("button", { name: "Pause", exact: true })).toBeVisible();
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  const attack = page.getByRole("button", { name: "Primary attack", exact: true });
  await attack.focus();
  await page.keyboard.press("Enter");
  await page.getByRole("button", { name: "Step", exact: true }).click();
  await expect(attack).toHaveAttribute("data-phase", "windup");
});

test("settings own focus and restore the opener on dismissal", async ({ page, appUrl }) => {
  await page.goto(`${appUrl}?scenario=training&seed=42`);
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Settings", exact: true });
  await expect(settings).toBeVisible();
  await expect(settings.getByRole("button", { name: "Close", exact: true })).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await expect(settings.getByRole("button", { name: "Close", exact: true })).not.toBeFocused();
  await page.keyboard.press("Tab");
  await expect(settings.getByRole("button", { name: "Close", exact: true })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(settings).not.toBeVisible();
  await expect(page.getByRole("button", { name: "Settings", exact: true })).toBeFocused();
});

test("blocked storage does not break primary workflows", async ({ page, appUrl }) => {
  await page.addInitScript(() => {
    Object.defineProperty(window, "localStorage", {
      get() {
        throw new DOMException("Blocked", "SecurityError");
      },
    });
  });
  await page.goto(appUrl);
  await page.getByRole("button", { name: "Enter World", exact: true }).click();
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByLabel("Enable shadows").uncheck();
  await expect(page.getByLabel("Enable shadows")).not.toBeChecked();
  await expect(page.getByRole("dialog", { name: "Settings", exact: true })).toBeVisible();
});

test("tick subscriptions update their view without rerendering the shell", async ({
  page,
  appUrl,
}) => {
  await page.goto(appUrl);
  const result = await page.evaluate(async () => {
    const { verifySnapshotUpdateDomain } = await import("/arpg/tests/browser/render-harness.jsx");
    const container = document.createElement("div");
    document.body.append(container);
    try {
      return verifySnapshotUpdateDomain(container);
    } finally {
      container.remove();
    }
  });
  expect(result).toEqual({ shellRenders: 1, text: "Tick 120" });
});

test("the browser projection accepts real WASM snapshots and rejects corrupt presentation data", async ({
  page,
  appUrl,
}) => {
  await page.goto(appUrl);
  const result = await page.evaluate(async () => {
    const wasm = await import("/arpg/src/wasm/arpg_web_wasm.js");
    const { decodeSnapshot } = await import("/arpg/src/wire-protocol.js");
    await wasm.default();
    const game = new wasm.WasmGame(42);
    try {
      game.addPlayer(1);
      const encoded = game.snapshotJson();
      const projected = decodeSnapshot(encoded);
      const corrupted = JSON.parse(encoded);
      corrupted.payload.players[0].position = [0, 0];
      let rejected = false;
      try {
        decodeSnapshot(JSON.stringify(corrupted));
      } catch {
        rejected = true;
      }
      return {
        matches: JSON.stringify(projected) === JSON.stringify(JSON.parse(encoded).payload),
        rejected,
      };
    } finally {
      game.free();
    }
  });
  expect(result).toEqual({ matches: true, rejected: true });
});

test("touch combat and settings remain usable on a phone viewport", async ({ browser, appUrl }) => {
  const context = await browser.newContext({
    viewport: { width: 390, height: 844 },
    hasTouch: true,
    isMobile: true,
  });
  const page = await context.newPage();
  try {
    await page.goto(`${appUrl}?scenario=training&seed=42`);
    await page.getByRole("button", { name: "Pause", exact: true }).tap();
    const attack = page.getByRole("button", { name: "Primary attack", exact: true });
    await attack.tap();
    await page.getByRole("button", { name: "Step", exact: true }).tap();
    await expect(attack).toHaveAttribute("data-phase", "windup");
    await page.getByRole("button", { name: "Settings", exact: true }).tap();
    await expect(page.getByRole("dialog", { name: "Settings", exact: true })).toBeVisible();
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );
  } finally {
    await context.close();
  }
});

test("leaving and re-entering the world restarts one paused-aware simulation", async ({
  page,
  appUrl,
}) => {
  await page.goto(`${appUrl}?scenario=training&seed=42`);
  const panel = page.getByRole("complementary", { name: "Training arena controls" });
  const tickOf = async () => Number((await panel.getByText(/^Tick \d+$/).textContent()).slice(5));

  for (let round = 0; round < 3; round += 1) {
    await expect.poll(tickOf).toBeGreaterThan(5);
    await page.getByRole("button", { name: "Characters", exact: true }).click();
    await expect(page.getByRole("main", { name: "Character selection" })).toBeVisible();
    await page.getByRole("button", { name: "Training Arena", exact: true }).click();
  }

  await page.getByRole("button", { name: "Pause", exact: true }).click();
  const paused = await tickOf();
  await page.waitForTimeout(250);
  expect(await tickOf()).toBe(paused);
  await page.getByRole("button", { name: "Step", exact: true }).click();
  await expect.poll(tickOf).toBe(paused + 1);
});

// Normal play (Enter World, not the training route) through the runtime owners of #88: the
// presenter sizes and keeps one canvas renderer, keyboard input reaches the authority in
// gameplay context but not while Settings is open, a graphics change keeps play running,
// and leaving and re-entering repeats all of it.
test("normal play renders, takes keyboard input and survives settings and re-entry", async ({
  page,
  appUrl,
}) => {
  await page.goto(appUrl);
  const canvas = page.getByLabel("ARPG game world", { exact: true });
  const guard = page.getByRole("button", { name: "Hold shield", exact: true });

  for (let round = 0; round < 3; round += 1) {
    await page.getByRole("button", { name: "Enter World", exact: true }).click();
    await expect(page.locator("canvas")).toHaveCount(1);
    await expect
      .poll(() => canvas.evaluate((element) => element.width * element.height))
      .toBeGreaterThan(0);
    await expect(guard).toBeEnabled();

    await page.evaluate(() => document.activeElement?.blur());
    await page.keyboard.down("f");
    await expect(guard).toHaveAttribute("data-guard", "raised");
    await page.keyboard.up("f");
    await expect(guard).not.toHaveAttribute("data-guard", /.+/);

    // Menu context: gameplay keys do not reach the authority while Settings is open.
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    const settings = page.getByRole("dialog", { name: "Settings", exact: true });
    await expect(settings).toBeVisible();
    await page.keyboard.down("f");
    await page.waitForTimeout(250);
    // Combat buttons are hidden behind Settings; the HUD guard status stays rendered.
    await expect(page.locator(".guard-status")).toHaveCount(0);
    await page.keyboard.up("f");
    if (round === 1) await settings.getByLabel("Enable shadows").click();
    await settings.getByRole("button", { name: "Close", exact: true }).click();
    await expect(settings).not.toBeVisible();

    await page.evaluate(() => document.activeElement?.blur());
    await page.keyboard.down("f");
    await expect(guard).toHaveAttribute("data-guard", "raised");
    await page.keyboard.up("f");
    await expect(page.locator("canvas")).toHaveCount(1);

    await page.getByRole("button", { name: "Characters", exact: true }).click();
    await expect(page.getByRole("main", { name: "Character selection" })).toBeVisible();
  }
});

test("the training arena exports its session as a replayable reproduction", async ({
  page,
  appUrl,
}) => {
  await page.goto(`${appUrl}?scenario=training&seed=42`);
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  const step = page.getByRole("button", { name: "Step", exact: true });
  await page.evaluate(() => document.activeElement?.blur());
  await page.keyboard.press("Space");
  for (let tick = 0; tick < 3; tick += 1) await step.click();

  const download = page.waitForEvent("download");
  await page.getByRole("button", { name: "Export reproduction", exact: true }).click();
  const file = await download;
  expect(file.suggestedFilename()).toMatch(/^arpg-reproduction-.+-seed-42-tick-\d+\.json$/);
  const reproduction = JSON.parse(await readFile(await file.path(), "utf8"));
  expect(reproduction.seed).toBe(42);
  expect(reproduction.players).toEqual([1]);
  expect(reproduction.ticks).toBeGreaterThanOrEqual(3);
  expect(reproduction.commands.map((entry) => entry.command)).toContainEqual(
    expect.objectContaining({ type: "primaryAttack" }),
  );
  await expect(page.getByText(/^Reproduction exported · /)).toBeVisible();
});

test("holding guard raises the authoritative shield and releasing lowers it", async ({
  page,
  appUrl,
}) => {
  await page.goto(`${appUrl}?scenario=training&seed=42`);
  const guard = page.getByRole("button", { name: "Hold shield", exact: true });
  await expect(guard).toBeEnabled();
  await page.evaluate(() => document.activeElement?.blur());

  await page.keyboard.down("f");
  await expect(guard).toHaveAttribute("data-guard", "raised");
  await expect(page.getByText("Shield raised", { exact: true })).toBeVisible();
  await page.keyboard.up("f");
  await expect(guard).not.toHaveAttribute("data-guard", /.+/);

  await page.keyboard.down("f");
  await expect(guard).toHaveAttribute("data-guard", "raised");
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await expect(guard).not.toHaveAttribute("data-guard", /.+/);
  await page.keyboard.up("f");
});

test("timed attacks chain into the light combo and a missed window starts over", async ({
  page,
  appUrl,
}) => {
  await page.goto(`${appUrl}?scenario=training&seed=42`);
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  const step = page.getByRole("button", { name: "Step", exact: true });
  const diagnostics = page.locator(".training-diagnostics");
  // Commands apply immediately; the paused view updates on the next simulation step.
  const pressAndStep = async () => {
    await page.evaluate(() => document.activeElement?.blur());
    await page.keyboard.press("Space");
    await step.click();
  };
  const stepUntil = async (text) => {
    for (let tick = 0; tick < 40; tick += 1) {
      if ((await diagnostics.textContent()).includes(text)) return;
      await step.click();
    }
    throw new Error(`never reached ${text}`);
  };

  await pressAndStep();
  await expect(diagnostics).toContainText("primaryAttack · windup");
  // Recovery tick 2 of the opener (8 recovery ticks) opens the follow-up interval.
  await stepUntil("primaryAttack · recovery · 6t");
  await pressAndStep();
  await expect(diagnostics).toContainText("lightFollowUp · windup");
  await stepUntil("lightFollowUp · recovery · 6t");
  await pressAndStep();
  await expect(diagnostics).toContainText("lightFinisher · windup");

  // Missing the interval: once the opener has recovered, a press starts over.
  await stepUntil("idle");
  await pressAndStep();
  await stepUntil("idle");
  await pressAndStep();
  await expect(diagnostics).toContainText("primaryAttack · windup");
});

test("the bow draws while held, shoots one arrow on release and cancels on blur", async ({
  page,
  appUrl,
}) => {
  await page.goto(`${appUrl}?scenario=training&seed=42`);
  const draw = page.getByRole("button", { name: "Draw bow", exact: true });
  const arrows = page.locator(".training-diagnostics div").filter({ hasText: "Arrows" });
  await page.getByRole("button", { name: "Switch to bow", exact: true }).waitFor();
  await page.evaluate(() => document.activeElement?.blur());
  await page.keyboard.press("x");
  await expect(draw).toBeVisible();

  await page.keyboard.down("Space");
  await expect(draw).toHaveAttribute("data-draw", "drawing");
  await page.waitForTimeout(600);
  await page.keyboard.up("Space");
  await expect(arrows).toContainText("Arrows1");
  await expect(arrows).toContainText("Arrows0", { timeout: 5_000 });

  await page.keyboard.down("Space");
  await expect(draw).toHaveAttribute("data-draw", "drawing");
  await page.waitForTimeout(600);
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await expect(draw).not.toHaveAttribute("data-draw", /.+/);
  await page.keyboard.up("Space");
  await page.waitForTimeout(300);
  await expect(arrows).toContainText("Arrows0");
});

test("all combat controls stay reachable on the narrowest phones", async ({ browser, appUrl }) => {
  for (const width of [320, 360, 390]) {
    const context = await browser.newContext({
      viewport: { width, height: 640 },
      hasTouch: true,
      isMobile: true,
    });
    const page = await context.newPage();
    try {
      await page.goto(`${appUrl}?scenario=training&seed=42`);
      const controls = page.getByRole("region", { name: "Combat actions" }).getByRole("button");
      // Heavy is encoded by an upward Attack swipe on touch layouts.
      await expect(controls).toHaveCount(4);
      await expect(page.getByRole("button", { name: "Heavy attack" })).toBeHidden();
      const stick = await page.getByRole("group", { name: "Movement joystick" }).boundingBox();
      const boxes = [];
      for (const control of await controls.all()) boxes.push(await control.boundingBox());
      for (const box of boxes) {
        expect(box.x).toBeGreaterThanOrEqual(0);
        expect(box.x + box.width).toBeLessThanOrEqual(width);
        expect(box.x).toBeGreaterThanOrEqual(stick.x + stick.width);
      }
      for (const [index, box] of boxes.entries())
        for (const other of boxes.slice(index + 1))
          expect(
            box.x + box.width <= other.x ||
              other.x + other.width <= box.x ||
              box.y + box.height <= other.y ||
              other.y + other.height <= box.y,
          ).toBe(true);
    } finally {
      await context.close();
    }
  }
});

test("narrow fine-pointer layouts retain a visible Heavy button", async ({ page, appUrl }) => {
  await page.setViewportSize({ width: 320, height: 640 });
  await page.goto(`${appUrl}?scenario=training&seed=42`);
  await expect(page.getByRole("button", { name: "Heavy attack" })).toBeVisible();
});
