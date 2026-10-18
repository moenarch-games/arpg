import { readFile } from "node:fs/promises";

import { expect, test } from "./fixtures.js";

// Aim and target lock are intent only: these specs drive the real WASM authority through
// the normal controls and read its evidence from the training panel.
async function openDummy(page, appUrl) {
  await page.goto(`${appUrl}?scenario=training&seed=42&fixture=dummy`);
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  await page.evaluate(() => document.activeElement?.blur());
  const step = page.getByRole("button", { name: "Step", exact: true });
  await step.click();
  return {
    step,
    timeline: page.getByRole("region", { name: "Event timeline" }),
    diagnostics: page.locator(".training-diagnostics"),
  };
}

test("a target lock turns the swing towards the dummy after facing away", async ({
  page,
  appUrl,
}) => {
  const arena = await openDummy(page, appUrl);
  await page.keyboard.down("a");
  await arena.step.click();
  await page.keyboard.up("a");
  await page.evaluate(() => document.activeElement?.blur());
  await page.keyboard.press("t");
  // The paused authority publishes the lock with its next snapshot.
  await arena.step.click();
  await expect(arena.diagnostics).toContainText("locked monster 1");
  await expect(page.getByLabel("Character progression")).toContainText("Target locked");
  await page.keyboard.press("r");
  for (let tick = 0; tick < 30; tick += 1) {
    if ((await arena.timeline.textContent()).includes("hit 25")) break;
    await arena.step.click();
  }
  await expect(arena.timeline).toContainText("sword.lightSwing · player 1 → monster 1 · hit 25");

  await page.evaluate(() => document.activeElement?.blur());
  await page.keyboard.press("g");
  await arena.step.click();
  await expect(arena.diagnostics).toContainText("Targetnone");
});

test("mouse facing is the default but remains an explicit accessibility preference", async ({ page, appUrl }) => {
  const arena = await openDummy(page, appUrl);
  const canvas = page
    .getByRole("img", { name: "ARPG game world" })
    .or(page.locator("canvas.game-canvas"));
  const box = await canvas.boundingBox();
  await page.mouse.move(box.x + box.width * 0.3, box.y + box.height * 0.6);
  await page.mouse.move(box.x + box.width * 0.32, box.y + box.height * 0.62);
  await arena.step.click();
  await expect(arena.diagnostics).toContainText("Aimaim ");

  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByLabel("Aim melee and bow with the mouse").uncheck();
  await page.keyboard.press("Escape");
  await arena.step.click();
  await expect(arena.diagnostics).toContainText("Aimfacing");

  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByLabel("Aim melee and bow with the mouse").check();
  await page.keyboard.press("Escape");
  await page.mouse.move(box.x + box.width * 0.3, box.y + box.height * 0.6);
  await page.mouse.move(box.x + box.width * 0.32, box.y + box.height * 0.62);
  await arena.step.click();
  await expect(arena.diagnostics).toContainText("Aimaim ");
});

 
async function recordedCommands(page) {
  const download = page.waitForEvent("download");
  await page.getByRole("button", { name: "Export reproduction", exact: true }).click();
  const replay = JSON.parse(await readFile(await (await download).path(), "utf8"));
  return replay.commands.map((entry) => entry.command);
}

test("holding right mouse moves toward the cursor and releasing stops", async ({ page, appUrl }) => {
  await openDummy(page, appUrl);
  const box = await page.locator("canvas.game-canvas").boundingBox();
  const point = (x, y) => [box.x + box.width * x, box.y + box.height * y];
  await page.mouse.move(...point(0.8, 0.45));
  await page.mouse.down({ button: "right" });
  await page.mouse.move(...point(0.2, 0.55));
  await page.mouse.up({ button: "right" });

  const commands = await recordedCommands(page);
  const moves = commands.filter((command) => command.type === "setMovement");
  expect(moves.length).toBeGreaterThanOrEqual(3);
  expect([moves[0].x, moves[0].z]).not.toEqual([0, 0]);
  expect([moves[1].x, moves[1].z]).not.toEqual([moves[0].x, moves[0].z]);
  expect([moves.at(-1).x, moves.at(-1).z]).toEqual([0, 0]);
  expect(moves.filter((command) => command.x === 0 && command.z === 0)).toHaveLength(1);
  expect(commands.some((command) => command.type === "setAim" && command.direction)).toBe(true);
  expect(commands.some((command) => command.type === "primaryAttack")).toBe(false);
});

test("losing focus cancels a right-mouse hold without resuming it", async ({ page, appUrl }) => {
  await openDummy(page, appUrl);
  const box = await page.locator("canvas.game-canvas").boundingBox();
  await page.mouse.move(box.x + box.width * 0.8, box.y + box.height * 0.5);
  await page.mouse.down({ button: "right" });
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
  await page.mouse.move(box.x + box.width * 0.75, box.y + box.height * 0.55);
  await page.mouse.up({ button: "right" });
  const moves = (await recordedCommands(page)).filter((command) => command.type === "setMovement");
  expect(moves).toHaveLength(2);
  expect([moves[0].x, moves[0].z]).not.toEqual([0, 0]);
  expect([moves[1].x, moves[1].z]).toEqual([0, 0]);
});

test("cancelled right-button capture stops moving exactly once", async ({ page, appUrl }) => {
  await openDummy(page, appUrl);
  const canvas = page.locator("canvas.game-canvas");
  const box = await canvas.boundingBox();
  await page.evaluate(() => {
    const canvas = document.querySelector("canvas.game-canvas");
    canvas.addEventListener(
      "pointerdown",
      (event) => {
        window.__mousePointerId = event.pointerId;
      },
      { once: true },
    );
  });
  await page.mouse.move(box.x + box.width * 0.8, box.y + box.height * 0.45);
  await page.mouse.down({ button: "right" });
  await page.evaluate(() => {
    document.querySelector("canvas.game-canvas").dispatchEvent(
      new PointerEvent("pointercancel", {
        bubbles: true,
        pointerId: window.__mousePointerId,
        pointerType: "mouse",
      }),
    );
  });
  await page.mouse.move(box.x + box.width * 0.2, box.y + box.height * 0.55);
  await page.mouse.up({ button: "right" });
  const moves = (await recordedCommands(page)).filter((command) => command.type === "setMovement");
  expect(moves).toHaveLength(2);
  expect([moves[0].x, moves[0].z]).not.toEqual([0, 0]);
  expect([moves[1].x, moves[1].z]).toEqual([0, 0]);
});
