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

test("mouse aim is opt-in and sends direction intent from the canvas", async ({ page, appUrl }) => {
  const arena = await openDummy(page, appUrl);
  const canvas = page
    .getByRole("img", { name: "ARPG game world" })
    .or(page.locator("canvas.game-canvas"));
  const box = await canvas.boundingBox();
  await page.mouse.move(box.x + box.width * 0.3, box.y + box.height * 0.6);
  await page.mouse.move(box.x + box.width * 0.32, box.y + box.height * 0.62);
  await arena.step.click();
  await expect(arena.diagnostics).toContainText("Aimfacing");

  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByLabel("Aim melee and bow with the mouse").check();
  await page.keyboard.press("Escape");
  await page.mouse.move(box.x + box.width * 0.3, box.y + box.height * 0.6);
  await page.mouse.move(box.x + box.width * 0.32, box.y + box.height * 0.62);
  await arena.step.click();
  await expect(arena.diagnostics).toContainText("Aimaim ");

  await page.getByRole("button", { name: "Settings", exact: true }).click();
  await page.getByLabel("Aim melee and bow with the mouse").uncheck();
  await page.keyboard.press("Escape");
  await arena.step.click();
  await expect(arena.diagnostics).toContainText("Aimfacing");
});
