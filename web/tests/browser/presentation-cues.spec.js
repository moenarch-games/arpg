import { expect, test } from "./fixtures.js";

// Presentation cues (#67, 67a) over the real WASM authority: the feedback feed, damage
// numbers and combo indicator describe what the authority resolved, step by step.
async function openScenario(page, appUrl, fixture) {
  await page.goto(`${appUrl}?scenario=training&seed=42&fixture=${fixture}`);
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  await page.evaluate(() => document.activeElement?.blur());
  const step = page.getByRole("button", { name: "Step", exact: true });
  const feed = page.getByRole("list", { name: "Combat feedback" });
  const diagnostics = page.locator(".training-diagnostics");
  const hud = page.getByRole("region", { name: "Player status" });
  const steps = async (count) => {
    for (let tick = 0; tick < count; tick += 1) await step.click();
  };
  const stepUntil = async (locator, text, limit = 60) => {
    for (let tick = 0; tick < limit; tick += 1) {
      const content = await locator.textContent();
      if (typeof text === "string" ? content.includes(text) : text.test(content)) return;
      await step.click();
    }
    throw new Error(`never saw ${text}`);
  };
  const press = async (key) => {
    await page.evaluate(() => document.activeElement?.blur());
    await page.keyboard.press(key);
  };
  return { feed, diagnostics, hud, steps, stepUntil, press };
}

test("a hit shows its authoritative damage number and a missed swing says so without one", async ({
  page,
  appUrl,
}) => {
  const dummy = await openScenario(page, appUrl, "dummy");
  await dummy.press("r");
  await dummy.stepUntil(dummy.feed, /Hit /);
  await expect(page.locator(".damage-number")).toHaveText(["25"]);

  const whiff = await openScenario(page, appUrl, "dummy");
  await page.keyboard.down("a");
  await whiff.steps(1);
  await page.keyboard.up("a");
  await whiff.press("r");
  await whiff.stepUntil(whiff.diagnostics, "primaryAttack · recovery");
  await expect(whiff.feed).toContainText("Missed");
  await expect(page.locator(".damage-number")).toHaveCount(0);
});

test("block, counter available, counter strike and a passed window read distinctly", async ({
  page,
  appUrl,
}) => {
  test.slow();
  const arena = await openScenario(page, appUrl, "enemy");
  await page.keyboard.down("f");
  await arena.stepUntil(arena.feed, "Blocked", 150);
  await page.keyboard.up("f");
  await expect(arena.hud).toContainText("Counter!");
  await arena.press("r");
  await arena.steps(1);
  await expect(arena.feed).toContainText("Counter strike");
  await expect(arena.feed).not.toContainText("Counter window passed");

  const late = await openScenario(page, appUrl, "enemy");
  await page.keyboard.down("f");
  await late.stepUntil(late.feed, "Blocked", 150);
  await page.keyboard.up("f");
  await late.stepUntil(late.feed, "Counter window passed", 40);
  await expect(late.hud).not.toContainText("Counter!");
});

test("a short combo shows its step and a bow release and world impact are announced", async ({
  page,
  appUrl,
}) => {
  const combo = await openScenario(page, appUrl, "dummy");
  await combo.press("r");
  await combo.steps(1);
  await combo.stepUntil(combo.diagnostics, "primaryAttack · recovery · 6t");
  await combo.press("r");
  await combo.steps(1);
  await expect(combo.hud).toContainText("Combo 2/3");

  const range = await openScenario(page, appUrl, "archeryObstructed");
  await range.press("x");
  await range.steps(1);
  await page.keyboard.down("r");
  await range.steps(32);
  await page.keyboard.up("r");
  await range.stepUntil(range.feed, "Arrow loosed");
  await range.stepUntil(range.feed, "Arrow hit the world");
  await expect(page.locator(".damage-number")).toHaveCount(0);
});
