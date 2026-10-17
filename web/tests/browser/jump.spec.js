// #59: Space = Jump on the standard keyboard profile, R = Primary Attack, and the one-time
// migration of stored profiles. The training arena's "Export reproduction" file lists every
// command the Rust/Wasm authority accepted, so each test reads what a key press actually
// did to the game.
import { readFile } from "node:fs/promises";
import { expect, test } from "./fixtures.js";

const LEGACY_KEY = "arpg-input-profile-v1";
const PROFILE_KEY = "arpg-input-profile-v2";

const physical = (id, action, code) => ({
  id,
  action,
  sequence: [{ key: { kind: "physical", value: code }, modifiers: {} }],
  when: { op: "context", id: "gameplay" },
  priority: 0,
});

// A customized version 1 profile: Heavy moved to H, Primary Attack deliberately left on its
// old default Space.
const CUSTOMIZED_V1 = {
  id: "arpg-player",
  patches: [
    {
      op: "replace",
      bindingId: "game.secondaryAttack.default",
      binding: physical("game.secondaryAttack.default", "game.secondaryAttack", "KeyH"),
    },
  ],
};

// Stores `profile` under the version 1 key before the first page load only.
async function storeLegacyProfile(page, profile) {
  await page.addInitScript(
    ([key, value]) => {
      if (sessionStorage.getItem("arpg-test-seeded")) return;
      localStorage.setItem(key, value);
      sessionStorage.setItem("arpg-test-seeded", "1");
    },
    [LEGACY_KEY, JSON.stringify(profile)],
  );
}

async function openArena(page, appUrl) {
  await page.goto(`${appUrl}?scenario=training&seed=42`);
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  await page.evaluate(() => document.activeElement?.blur());
  await page.getByRole("button", { name: "Step", exact: true }).click();
}

async function pressAndStep(page, key) {
  await page.evaluate(() => document.activeElement?.blur());
  await page.keyboard.press(key);
  const step = page.getByRole("button", { name: "Step", exact: true });
  for (let tick = 0; tick < 3; tick += 1) await step.click();
}

// The types of every command the authority accepted so far.
async function acceptedTypes(page) {
  const download = page.waitForEvent("download");
  await page.getByRole("button", { name: "Export reproduction", exact: true }).click();
  const replay = JSON.parse(await readFile(await (await download).path(), "utf8"));
  return replay.commands.map((entry) => entry.command.type);
}

const storedProfile = (page) =>
  page.evaluate((key) => JSON.parse(localStorage.getItem(key) ?? "null"), PROFILE_KEY);

async function keybindingRow(page, actionName) {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Settings", exact: true });
  return settings
    .getByRole("table", { name: "Keybindings" })
    .getByRole("row")
    .filter({ has: page.getByText(actionName, { exact: true }) });
}

test("on a fresh profile Space jumps without attacking and R attacks without jumping", async ({
  page,
  appUrl,
}) => {
  await openArena(page, appUrl);
  const jump = page.getByRole("button", { name: "Jump", exact: true });
  const attack = page.getByRole("button", { name: "Primary attack", exact: true });

  await pressAndStep(page, "Space");
  await expect(jump).toHaveAttribute("data-motion", "airborne");
  await expect(attack).not.toHaveAttribute("data-phase", /.+/);
  expect(await acceptedTypes(page)).toEqual(["jump"]);

  // R attacks, here in the air: the body keeps flying while the swing winds up.
  await page.evaluate(() => document.activeElement?.blur());
  await page.keyboard.press("r");
  await page.getByRole("button", { name: "Step", exact: true }).click();
  await expect(attack).toHaveAttribute("data-phase", "windup");
  await expect(jump).toHaveAttribute("data-motion", "airborne");
  expect(await acceptedTypes(page)).toEqual(["jump", "primaryAttack"]);

  // The hints name the effective keys.
  const hint = page.locator(".desktop-controls-hint");
  await expect(hint).toContainText("Space jump");
  await expect(hint).toContainText("R primary");
  // Nothing was migrated or written for a fresh browser.
  expect(await storedProfile(page)).toBeNull();

  // The body lands again.
  const step = page.getByRole("button", { name: "Step", exact: true });
  for (let tick = 0; tick < 45; tick += 1) await step.click();
  await expect(jump).toHaveAttribute("data-motion", "grounded");
});

test("a customized old profile keeps Space as its attack and shows Jump unbound", async ({
  page,
  appUrl,
}) => {
  await storeLegacyProfile(page, CUSTOMIZED_V1);
  await openArena(page, appUrl);
  const attack = page.getByRole("button", { name: "Primary attack", exact: true });

  await pressAndStep(page, "Space");
  await expect(attack).toHaveAttribute("data-phase", /.+/);
  await expect(page.getByRole("button", { name: "Jump", exact: true })).toHaveAttribute(
    "data-motion",
    "grounded",
  );
  expect(await acceptedTypes(page)).toEqual(["primaryAttack"]);
  await expect(page.locator(".desktop-controls-hint")).toContainText("unbound jump");
  await expect(page.locator(".desktop-controls-hint")).toContainText("Space primary");

  // The shared editor shows Primary Attack on Space, Heavy on H and Jump without a key.
  const jumpRow = await keybindingRow(page, "Jump");
  await expect(jumpRow).not.toContainText("Space");
  const dialog = page.getByRole("dialog", { name: "Settings", exact: true });
  const row = (name) =>
    dialog
      .getByRole("table", { name: "Keybindings" })
      .getByRole("row")
      .filter({ has: page.getByText(name, { exact: true }) });
  await expect(row("Primary attack")).toContainText("Space");
  await expect(row("Heavy attack")).toContainText("KeyH");
});

test("an unmodified old profile migrates once to Space = Jump", async ({ page, appUrl }) => {
  await storeLegacyProfile(page, { id: "arpg-player", patches: [] });
  await openArena(page, appUrl);
  expect(await storedProfile(page)).toEqual({ id: "arpg-player", patches: [] });
  await pressAndStep(page, "Space");
  expect(await acceptedTypes(page)).toEqual(["jump"]);

  // Changing the old document afterwards changes nothing: the migration does not run again.
  await page.evaluate(
    ([key, value]) => localStorage.setItem(key, value),
    [LEGACY_KEY, JSON.stringify(CUSTOMIZED_V1)],
  );
  await openArena(page, appUrl);
  expect(await storedProfile(page)).toEqual({ id: "arpg-player", patches: [] });
  await pressAndStep(page, "Space");
  expect(await acceptedTypes(page)).toEqual(["jump"]);
});
