import { readFile } from "node:fs/promises";

import { expect, test } from "./fixtures.js";

const point = (id, x, y) => ({ id, x: Math.round(x), y: Math.round(y) });
const center = (id, rect) => point(id, rect.x + rect.width / 2, rect.y + rect.height / 2);

async function touch(cdp, type, ...touchPoints) {
  // Chromium's CDP exercises non-primary contacts, unlike Playwright's tap().
  await cdp.send("Input.dispatchTouchEvent", { type, touchPoints });
}

async function exportedCommands(page) {
  const download = page.waitForEvent("download");
  await page.getByRole("button", { name: "Export reproduction", exact: true }).click();
  const file = await download;
  const replay = JSON.parse(await readFile(await file.path(), "utf8"));
  return replay.commands.map((entry) => entry.command);
}

async function phone(browser, appUrl) {
  const context = await browser.newContext({
    viewport: { width: 390, height: 844 },
    hasTouch: true,
    isMobile: true,
  });
  const page = await context.newPage();
  await page.goto(`${appUrl}?scenario=training&seed=42`);
  await page.getByRole("button", { name: "Pause", exact: true }).tap();
  return { context, page, cdp: await context.newCDPSession(page) };
}

test("two fingers move, attack once, and continue moving independently", async ({
  browser,
  appUrl,
}) => {
  const { context, page, cdp } = await phone(browser, appUrl);
  try {
    const stickRect = await page.getByRole("group", { name: "Movement joystick" }).boundingBox();
    const attack = page.getByRole("button", { name: "Primary attack", exact: true });
    const attackRect = await attack.boundingBox();
    const origin = center(1, stickRect);
    const right = point(1, origin.x + stickRect.width * 0.4, origin.y);
    const forward = point(1, origin.x, origin.y - stickRect.height * 0.4);
    const strike = center(2, attackRect);
    const targets = await page.evaluate(
      (points) =>
        points.map(({ x, y }) => {
          const hit = document.elementFromPoint(x, y);
          const target = hit?.closest("button, [aria-label='Movement joystick']");
          return target?.getAttribute("aria-label");
        }),
      [origin, strike],
    );
    expect(targets).toEqual(["Movement joystick", "Primary attack"]);

    await touch(cdp, "touchStart", origin);
    await touch(cdp, "touchMove", right);
    await touch(cdp, "touchStart", right, strike);
    await touch(cdp, "touchEnd", strike);
    await touch(cdp, "touchMove", forward);
    await touch(cdp, "touchEnd", forward);

    await page.getByRole("button", { name: "Step", exact: true }).click();

    const commands = (await exportedCommands(page))
      .filter((command) => command.type === "setMovement" || command.type === "primaryAttack")
      .map((command) =>
        command.type === "setMovement" ? `${command.x},${command.z}` : "primaryAttack",
      );
    expect(commands).toEqual(["1,0", "primaryAttack", "0,-1", "0,0"]);
    await expect(attack).toHaveAttribute("data-phase", "windup");
  } finally {
    await context.close();
  }
});

test("a single touch press does not double-dispatch its synthesized click", async ({
  browser,
  appUrl,
}) => {
  const { context, page } = await phone(browser, appUrl);
  try {
    await page.getByRole("button", { name: "Primary attack", exact: true }).tap();
    await page.getByRole("button", { name: "Step", exact: true }).click();
    const commands = await exportedCommands(page);
    expect(commands.filter((command) => command.type === "primaryAttack")).toHaveLength(1);
  } finally {
    await context.close();
  }
});

test("a mouse click after a touch still triggers a separate action", async ({
  browser,
  appUrl,
}) => {
  const { context, page } = await phone(browser, appUrl);
  try {
    const attack = page.getByRole("button", { name: "Primary attack", exact: true });
    await attack.tap();
    await attack.click();
    await page.getByRole("button", { name: "Step", exact: true }).click();
    const commands = await exportedCommands(page);
    expect(commands.filter((command) => command.type === "primaryAttack")).toHaveLength(2);
  } finally {
    await context.close();
  }
});

test("holding guard while moving releases only when the guard finger lifts", async ({
  browser,
  appUrl,
}) => {
  const { context, page, cdp } = await phone(browser, appUrl);
  try {
    const guardRect = await page.getByRole("button", { name: "Hold shield" }).boundingBox();
    const stickRect = await page.getByRole("group", { name: "Movement joystick" }).boundingBox();
    const guard = center(1, guardRect);
    const origin = center(2, stickRect);
    const moving = point(2, origin.x + stickRect.width * 0.4, origin.y);

    await touch(cdp, "touchStart", guard);
    await touch(cdp, "touchStart", guard, origin);
    await touch(cdp, "touchMove", guard, moving);
    await touch(cdp, "touchEnd", moving);
    await page.getByRole("button", { name: "Step", exact: true }).click();
    // The second finger's release must not lower the first finger's shield.
    const held = await exportedCommands(page);
    expect(held.filter((command) => command.type === "setGuard")).toEqual([
      { type: "setGuard", raised: true },
    ]);
    await touch(cdp, "touchEnd", guard);
    await page.getByRole("button", { name: "Step", exact: true }).click();

    const commands = await exportedCommands(page);
    expect(commands.filter((command) => command.type === "setGuard")).toEqual([
      { type: "setGuard", raised: true },
      { type: "setGuard", raised: false },
    ]);
    expect(commands.filter((command) => command.type === "setMovement")).toEqual([
      { type: "setMovement", x: 1, z: 0 },
      { type: "setMovement", x: 0, z: 0 },
    ]);
  } finally {
    await context.close();
  }
});

async function swipeAction(cdp, id, start, dx, dy) {
  const end = point(id, start.x + dx, start.y + dy);
  await touch(cdp, "touchStart", point(id, start.x, start.y));
  await touch(cdp, "touchMove", end);
  await touch(cdp, "touchEnd", end);
}

test("a sword heavy swipe and movement stay independent and ordered", async ({
  browser,
  appUrl,
}) => {
  const { context, page, cdp } = await phone(browser, appUrl);
  try {
    const stick = await page.getByRole("group", { name: "Movement joystick" }).boundingBox();
    const attack = await page.getByRole("button", { name: "Primary attack" }).boundingBox();
    const origin = center(1, stick);
    const right = point(1, origin.x + stick.width * 0.4, origin.y);
    const forward = point(1, origin.x, origin.y - stick.height * 0.4);
    const start = center(2, attack);
    const raised = point(2, start.x, start.y - 48);

    await touch(cdp, "touchStart", origin);
    await touch(cdp, "touchMove", right);
    await touch(cdp, "touchStart", right, start);
    await touch(cdp, "touchMove", right, raised);
    await touch(cdp, "touchEnd", raised);
    await touch(cdp, "touchMove", forward);
    await touch(cdp, "touchEnd", forward);

    await page.getByRole("button", { name: "Step", exact: true }).click();
    const commands = (await exportedCommands(page))
      .filter((command) =>
        ["setMovement", "primaryAttack", "secondaryAttack"].includes(command.type),
      )
      .map((command) =>
        command.type === "setMovement" ? `${command.x},${command.z}` : command.type,
      );
    expect(commands).toEqual(["1,0", "secondaryAttack", "0,-1", "0,0"]);
    await expect(page.getByRole("button", { name: "Heavy attack" })).toBeHidden();
  } finally {
    await context.close();
  }
});

test("horizontal sword swipes reuse target commands without accidentally attacking", async ({
  browser,
  appUrl,
}) => {
  const { context, page, cdp } = await phone(browser, appUrl);
  try {
    const attack = page.getByRole("button", { name: "Primary attack" });
    const start = center(1, await attack.boundingBox());
    await swipeAction(cdp, 1, start, 50, 0);
    await swipeAction(cdp, 1, start, -50, 0);
    await attack.tap();
    await page.getByRole("button", { name: "Step", exact: true }).click();

    const actions = (await exportedCommands(page))
      .map((command) => command.type)
      .filter((type) =>
        ["cycleTarget", "clearTarget", "primaryAttack", "secondaryAttack"].includes(type),
      );
    expect(actions).toEqual(["cycleTarget", "clearTarget", "primaryAttack"]);
  } finally {
    await context.close();
  }
});

test("a cancelled or ambiguous swipe never becomes a light attack", async ({ browser, appUrl }) => {
  const { context, page, cdp } = await phone(browser, appUrl);
  try {
    const attack = page.getByRole("button", { name: "Primary attack" });
    const start = center(1, await attack.boundingBox());
    await touch(cdp, "touchStart", start);
    await touch(cdp, "touchMove", point(1, start.x, start.y - 45));
    await touch(cdp, "touchCancel");

    await swipeAction(cdp, 1, start, 40, 40);
    await attack.tap();
    await page.getByRole("button", { name: "Step", exact: true }).click();

    const actions = (await exportedCommands(page))
      .map((command) => command.type)
      .filter((type) => ["primaryAttack", "secondaryAttack"].includes(type));
    expect(actions).toEqual(["primaryAttack"]);
  } finally {
    await context.close();
  }
});

test("an action gesture cannot finish across the settings context transition", async ({
  browser,
  appUrl,
}) => {
  const { context, page, cdp } = await phone(browser, appUrl);
  try {
    const attack = page.getByRole("button", { name: "Primary attack" });
    const start = center(1, await attack.boundingBox());
    const raised = point(1, start.x, start.y - 48);
    await touch(cdp, "touchStart", start);
    await touch(cdp, "touchMove", raised);
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    await touch(cdp, "touchEnd", raised);

    await page
      .getByRole("dialog", { name: "Settings", exact: true })
      .getByRole("button", { name: "Close", exact: true })
      .click();
    await page.getByRole("button", { name: "Primary attack" }).tap();
    await page.getByRole("button", { name: "Step", exact: true }).click();
    const actions = (await exportedCommands(page))
      .map((command) => command.type)
      .filter((type) => ["primaryAttack", "secondaryAttack"].includes(type));
    expect(actions).toEqual(["primaryAttack"]);
  } finally {
    await context.close();
  }
});

test("guard hold survives another finger's targeting swipe and releases in order", async ({
  browser,
  appUrl,
}) => {
  const { context, page, cdp } = await phone(browser, appUrl);
  try {
    const guard = center(1, await page.getByRole("button", { name: "Hold shield" }).boundingBox());
    const attack = center(
      2,
      await page.getByRole("button", { name: "Primary attack" }).boundingBox(),
    );
    const target = point(2, attack.x + 48, attack.y);
    await touch(cdp, "touchStart", guard);
    await touch(cdp, "touchStart", guard, attack);
    await touch(cdp, "touchMove", guard, target);
    await touch(cdp, "touchEnd", target);
    await touch(cdp, "touchEnd", guard);
    await page.getByRole("button", { name: "Step", exact: true }).click();
    const commands = (await exportedCommands(page))
      .filter((command) => ["setGuard", "cycleTarget", "primaryAttack"].includes(command.type))
      .map((command) => (command.type === "setGuard" ? `guard:${command.raised}` : command.type));
    expect(commands).toEqual(["guard:true", "cycleTarget", "guard:false"]);
  } finally {
    await context.close();
  }
});

test("weapon switch invalidates a held sword swipe before the next snapshot", async ({
  browser,
  appUrl,
}) => {
  const { context, page, cdp } = await phone(browser, appUrl);
  try {
    const attack = center(
      1,
      await page.getByRole("button", { name: "Primary attack" }).boundingBox(),
    );
    const swap = center(2, await page.getByRole("button", { name: "Switch to bow" }).boundingBox());
    const target = point(1, attack.x + 45, attack.y);
    await touch(cdp, "touchStart", attack);
    await touch(cdp, "touchStart", attack, swap);
    // Swapping is a press action; the training authority remains paused.
    await touch(cdp, "touchEnd", swap);
    await touch(cdp, "touchMove", target);
    await touch(cdp, "touchEnd", target);
    await page.getByRole("button", { name: "Step", exact: true }).click();

    const commands = await exportedCommands(page);
    expect(commands.filter((command) => command.type === "equipWeapon")).toHaveLength(1);
    expect(
      commands.filter((command) =>
        ["primaryAttack", "secondaryAttack", "cycleTarget", "clearTarget"].includes(command.type),
      ),
    ).toEqual([]);
  } finally {
    await context.close();
  }
});

test("releasing the movement finger first still commits the independent Attack gesture", async ({
  browser,
  appUrl,
}) => {
  const { context, page, cdp } = await phone(browser, appUrl);
  try {
    const stick = await page.getByRole("group", { name: "Movement joystick" }).boundingBox();
    const attack = await page.getByRole("button", { name: "Primary attack" }).boundingBox();
    const origin = center(1, stick);
    const right = point(1, origin.x + stick.width * 0.4, origin.y);
    const strike = center(2, attack);

    await touch(cdp, "touchStart", origin);
    await touch(cdp, "touchMove", right);
    await touch(cdp, "touchStart", right, strike);
    await touch(cdp, "touchEnd", right);
    await touch(cdp, "touchEnd", strike);

    await page.getByRole("button", { name: "Step", exact: true }).click();
    const commands = (await exportedCommands(page))
      .filter((command) => ["setMovement", "primaryAttack"].includes(command.type))
      .map((command) =>
        command.type === "setMovement" ? `${command.x},${command.z}` : command.type,
      );
    expect(commands).toEqual(["1,0", "0,0", "primaryAttack"]);
  } finally {
    await context.close();
  }
});

test("the touch Jump control jumps once per tap while the other thumb moves (#59)", async ({
  browser,
  appUrl,
}) => {
  const { context, page, cdp } = await phone(browser, appUrl);
  try {
    const stickRect = await page.getByRole("group", { name: "Movement joystick" }).boundingBox();
    const jump = page.getByRole("button", { name: "Jump", exact: true });
    const jumpRect = await jump.boundingBox();
    // The control sits on screen next to the others, clear of the joystick.
    const viewport = page.viewportSize();
    expect(jumpRect.x + jumpRect.width).toBeLessThanOrEqual(viewport.width);
    expect(jumpRect.y + jumpRect.height).toBeLessThanOrEqual(viewport.height);
    const origin = center(1, stickRect);
    const right = point(1, origin.x + stickRect.width * 0.4, origin.y);
    const press = center(2, jumpRect);
    expect(
      await page.evaluate(
        ({ x, y }) =>
          document.elementFromPoint(x, y)?.closest("button")?.getAttribute("aria-label"),
        press,
      ),
    ).toBe("Jump");

    await touch(cdp, "touchStart", origin);
    await touch(cdp, "touchMove", right);
    await touch(cdp, "touchStart", right, press);
    await touch(cdp, "touchEnd", press);
    await page.getByRole("button", { name: "Step", exact: true }).click();
    await expect(jump).toHaveAttribute("data-motion", "airborne");
    await touch(cdp, "touchEnd", right);

    const commands = (await exportedCommands(page)).map((command) => command.type);
    expect(commands.filter((type) => type === "jump")).toHaveLength(1);
    expect(commands).not.toContain("primaryAttack");
  } finally {
    await context.close();
  }
});
