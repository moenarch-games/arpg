import { readFile } from "node:fs/promises";
import { expect, test } from "./fixtures.js";

// Workbench scenarios run the real WASM authority; these specs drive them through the
// normal controls and read authoritative evidence from the training panel.
async function openScenario(page, appUrl, fixture) {
  await page.goto(`${appUrl}?scenario=training&seed=42&fixture=${fixture}`);
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  await page.evaluate(() => document.activeElement?.blur());
  const step = page.getByRole("button", { name: "Step", exact: true });
  const timeline = page.getByRole("region", { name: "Event timeline" });
  const diagnostics = page.locator(".training-diagnostics");
  const steps = async (count) => {
    for (let tick = 0; tick < count; tick += 1) await step.click();
  };
  const stepUntil = async (locator, text, limit = 60) => {
    for (let tick = 0; tick < limit; tick += 1) {
      if ((await locator.textContent()).includes(text)) return;
      await step.click();
    }
    throw new Error(`never saw ${text}`);
  };
  const press = async (key) => {
    await page.evaluate(() => document.activeElement?.blur());
    await page.keyboard.press(key);
  };
  return { timeline, diagnostics, steps, stepUntil, press };
}

test("an unknown scenario falls back to the generated dungeon", async ({ page, appUrl }) => {
  await page.goto(`${appUrl}?scenario=training&seed=42&fixture=lava`);
  await expect(
    page.getByText('Unknown scenario "lava"; using the generated dungeon'),
  ).toBeVisible();
  await expect(page.getByLabel("Scenario")).toHaveValue("dungeon");
});

test("melee scenarios show a forward hit, an aimed-away whiff, a blocked strike and an outrun one", async ({
  page,
  appUrl,
}) => {
  const dummy = await openScenario(page, appUrl, "dummy");
  await dummy.press("r");
  await dummy.stepUntil(dummy.timeline, "sword.lightSwing · player 1 → monster 1 · hit 25");

  const whiff = await openScenario(page, appUrl, "dummy");
  await page.keyboard.down("a");
  await whiff.steps(1);
  await page.keyboard.up("a");
  await whiff.press("r");
  await whiff.stepUntil(whiff.diagnostics, "primaryAttack · recovery");
  await expect(whiff.timeline).toContainText("No events yet");

  const blocked = await openScenario(page, appUrl, "obstructed");
  await blocked.press("r");
  await blocked.stepUntil(blocked.timeline, "obstructed");
  await expect(blocked.timeline).not.toContainText("hit");

  // The archer starts inside the swing but backs out of reach during the wind-up.
  const retreat = await openScenario(page, appUrl, "retreating");
  await retreat.press("r");
  await retreat.stepUntil(retreat.diagnostics, "primaryAttack · recovery");
  await expect(retreat.diagnostics).toContainText("Retreating1");
  await expect(retreat.timeline).toContainText("No events yet");
});

test("the traversal scenario climbs platform and bridge and stops a jump at the ceiling", async ({
  page,
  appUrl,
}) => {
  const traversal = await openScenario(page, appUrl, "traversal");
  await expect(traversal.diagnostics).toContainText("region 1");
  await page.keyboard.down("d");
  await traversal.steps(7);
  await traversal.press("Space");
  await traversal.stepUntil(traversal.diagnostics, "region 2", 60);
  await traversal.stepUntil(traversal.diagnostics, "feet 60", 5);
  // Pressed against the bridge's face, a jump climbs onto it through a transit (#210).
  await traversal.steps(12);
  await traversal.press("Space");
  await traversal.stepUntil(traversal.diagnostics, "transit 3", 30);
  await traversal.stepUntil(traversal.diagnostics, "region 3", 40);
  await page.keyboard.up("d");

  // Under the low ceiling a jump stops with the head at its underside, 140 units up.
  const ceiling = await openScenario(page, appUrl, "traversal");
  await page.keyboard.down("w");
  await ceiling.steps(36);
  await page.keyboard.up("w");
  await ceiling.steps(10);
  await ceiling.press("Space");
  await ceiling.stepUntil(ceiling.diagnostics, "feet 40", 20);
  await ceiling.stepUntil(ceiling.diagnostics, "feet 0", 40);
});

test("a block opens a counter window that expires and a counter honours reach", async ({
  page,
  appUrl,
}) => {
  // Three scenario loads, each with the real character bundle (#55), and ~200 steps.
  test.slow();
  const arena = await openScenario(page, appUrl, "enemy");
  await page.keyboard.down("f");
  await arena.stepUntil(arena.timeline, "monster.claw · monster 1 → player 1 · blocked");
  await page.keyboard.up("f");
  await arena.press("r");
  await arena.steps(1);
  await expect(arena.diagnostics).toContainText("counter · windup");
  // The counter continues into the declared combo follow-up (recovery tick 2 of 10).
  await arena.stepUntil(arena.diagnostics, "counter · recovery · 8t");
  await arena.press("r");
  await arena.steps(1);
  await expect(arena.diagnostics).toContainText("lightFollowUp · windup");

  const late = await openScenario(page, appUrl, "enemy");
  await page.keyboard.down("f");
  await late.stepUntil(late.timeline, "blocked");
  await page.keyboard.up("f");
  await late.steps(31);
  await late.press("r");
  await late.steps(1);
  await expect(late.diagnostics).toContainText("primaryAttack · windup");

  // The archer backs off before it shoots: a counter after blocking its bolt starts, but
  // honours reach and whiffs instead of homing in on the retreated attacker.
  const retreated = await openScenario(page, appUrl, "retreating");
  await page.keyboard.down("f");
  await retreated.stepUntil(retreated.timeline, "→ player 1 · blocked", 150);
  await expect(retreated.timeline).toContainText("monster.bolt");
  await page.keyboard.up("f");
  await retreated.press("r");
  await retreated.steps(1);
  await expect(retreated.diagnostics).toContainText("counter · windup");
  await retreated.stepUntil(retreated.diagnostics, "counter · recovery");
  await expect(retreated.timeline).not.toContainText("sword.counterSlash");
});

test("archery scenarios show an arrow hit and an arrow stopped by a pillar", async ({
  page,
  appUrl,
}) => {
  for (const [fixture, hits] of [
    ["archery", true],
    ["archeryObstructed", false],
  ]) {
    const range = await openScenario(page, appUrl, fixture);
    await range.press("x");
    await range.steps(1);
    await page.keyboard.down("r");
    await range.steps(32);
    await page.keyboard.up("r");
    await range.stepUntil(range.diagnostics, "Arrows1");
    await range.stepUntil(range.diagnostics, "Arrows0");
    if (hits) await expect(range.timeline).toContainText("bow.arrow · player 1 → monster 1 · hit");
    else await expect(range.timeline).toContainText("No events yet");
  }
});

test("role scenarios show a ranged shot in flight and a heavy telegraphed slam", async ({
  page,
  appUrl,
}) => {
  const ranged = await openScenario(page, appUrl, "ranged");
  await ranged.stepUntil(ranged.diagnostics, "Arrows1", 80);
  await ranged.stepUntil(ranged.timeline, "monster.bolt · monster 2 → player 1 · hit 8", 80);

  const heavy = await openScenario(page, appUrl, "heavy");
  // The slam lands only after its 45-tick wind-up.
  await heavy.stepUntil(heavy.timeline, "monster.slam · monster 4 → player 1 · hit 28", 80);
  await expect(heavy.timeline).toContainText(/tick (4[5-9]|[5-9]\d) · monster\.slam/);
});

test("a defeated dummy drops gold that the authority prompts for and pays once", async ({
  page,
  appUrl,
}) => {
  const arena = await openScenario(page, appUrl, "dummy");
  for (let swing = 0; swing < 4; swing += 1) {
    await arena.press("r");
    await arena.steps(1);
    await arena.stepUntil(arena.diagnostics, "Player actionidle");
  }
  await expect(arena.timeline).toContainText("defeated");
  // Walk towards the drop until the authority offers it.
  const hud = page.getByRole("region", { name: "Player status" });
  await page.keyboard.down("d");
  await arena.stepUntil(hud, "E · Pick up gold");
  await page.keyboard.up("d");
  await arena.steps(2);
  await arena.press("e");
  await arena.stepUntil(arena.timeline, "picked up loot");
  await expect(hud).toContainText("Gold 10");
  await arena.stepUntil(arena.diagnostics, "Player actionidle");
  await arena.press("e");
  await arena.stepUntil(arena.timeline, "interact refused · nothingInRange");
});

test("workbench spawn, tuning and reset reach the reproduction", async ({ page, appUrl }) => {
  // The ranged scenario's room is not the first room, so outsiders must not be offered.
  await page.goto(`${appUrl}?scenario=training&seed=42&fixture=ranged`);
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  await page.getByText("Arrangement and tuning", { exact: true }).click();
  const workbench = page.getByRole("region", { name: "Workbench arrangement and tuning" });
  const step = page.getByRole("button", { name: "Step", exact: true });
  await step.click();

  const living = workbench.getByLabel("Room monster");
  const before = await living.locator("option").count();
  await workbench.getByLabel("Monster", { exact: true }).selectOption("monster.skirmisher");
  await workbench.getByLabel("Offset x").fill("-250");
  await workbench.getByLabel("Offset z").fill("180");
  await workbench.getByRole("button", { name: "Spawn", exact: true }).click();
  await expect(living.locator("option")).toHaveCount(before + 1);
  await expect(page.getByText("Workbench · spawned monster.skirmisher at -250, 180")).toBeVisible();

  await workbench.getByLabel("Offset x").fill("9000");
  await workbench.getByRole("button", { name: "Spawn", exact: true }).click();
  await expect(workbench.getByRole("alert")).toHaveText(
    "spawn offset lies outside the workbench room",
  );

  await workbench.getByLabel("Tuning").selectOption("guard.maxPoints");
  await workbench.getByLabel("Value").fill("0");
  await workbench.getByRole("button", { name: "Set", exact: true }).click();
  await expect(workbench.getByRole("alert")).toContainText(
    "guard.maxPoints: must be within 1..=1000",
  );
  await workbench.getByLabel("Value").fill("40");
  await workbench.getByRole("button", { name: "Set", exact: true }).click();
  await expect(workbench.getByRole("alert")).toHaveCount(0);
  await expect(page.getByText("Guard 40/40")).toBeVisible();

  await workbench.getByRole("button", { name: "Reset arrangement", exact: true }).click();
  await expect(living.locator("option")).toHaveCount(before);
  await step.click();
  // The selector offers only the workbench room's monsters, so its default removes.
  await workbench.getByRole("button", { name: "Remove", exact: true }).click();
  await expect(workbench.getByRole("alert")).toHaveCount(0);
  await expect(living.locator("option")).toHaveCount(before - 1);
  await workbench.getByRole("button", { name: "Reset arrangement", exact: true }).click();
  await step.click();

  const download = page.waitForEvent("download");
  await page.getByRole("button", { name: "Export reproduction", exact: true }).click();
  const reproduction = JSON.parse(await readFile(await (await download).path(), "utf8"));
  expect(reproduction.operations.map((entry) => entry.operation)).toEqual([
    { type: "spawnMonster", definition: "monster.skirmisher", offset: [-250, 180] },
    { type: "setTuning", parameter: "guard.maxPoints", value: 40 },
    { type: "resetArrangement" },
    { type: "removeMonster", monsterId: expect.any(Number) },
    { type: "resetArrangement" },
  ]);
});
