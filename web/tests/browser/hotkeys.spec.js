import { expect, test } from "./fixtures.js";

async function openSettings(page) {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Settings", exact: true });
  await expect(settings).toBeVisible();
  return settings;
}

function actionRow(settings, actionName) {
  return settings
    .getByRole("table", { name: "Keybindings" })
    .getByRole("row")
    .filter({ has: settings.page().getByText(actionName, { exact: true }) });
}

async function replaceBinding(page, settings, actionName, key) {
  await actionRow(settings, actionName).getByRole("button", { name: "Edit", exact: true }).click();
  const recorder = settings.locator(".ib-recorder");
  await expect(recorder).toBeVisible();
  // The recorder starts from the existing shortcut; clear it so the key replaces it.
  await recorder.getByRole("button", { name: "Clear", exact: true }).first().click();
  await recorder.getByRole("button", { name: "Focus recorder", exact: true }).click();
  await page.keyboard.press(key);
  const save = recorder.getByRole("button", { name: "Save", exact: true }).first();
  await expect(save).toBeEnabled();
  await save.click();
}

async function closeSettings(page, settings) {
  await settings.getByRole("button", { name: "Close", exact: true }).click();
  await expect(settings).not.toBeVisible();
  // Focus returns to the Settings button; release it so gameplay keys reach the runtime.
  await page.evaluate(() => document.activeElement?.blur());
}

async function savedProfile(page) {
  return page.evaluate(() => localStorage.getItem("arpg-input-profile-v2"));
}

test("a replaced movement hotkey persists across reloads", async ({ page, appUrl }) => {
  await page.goto(`${appUrl}?scenario=training&seed=42`);
  let settings = await openSettings(page);
  await replaceBinding(page, settings, "Move forward", "ArrowUp");
  expect(await savedProfile(page)).toContain("ArrowUp");
  await closeSettings(page, settings);

  await page.reload();
  settings = await openSettings(page);
  const row = actionRow(settings, "Move forward");
  await expect(row).toContainText("ArrowUp");
  await expect(row).not.toContainText("KeyW");
});

test("a replaced primary-attack hotkey drives combat and disables the old key", async ({
  page,
  appUrl,
}) => {
  await page.goto(`${appUrl}?scenario=training&seed=42`);
  let settings = await openSettings(page);
  // The standard key is R (#59); move it to J.
  await replaceBinding(page, settings, "Primary attack", "j");
  expect(await savedProfile(page)).toContain("KeyJ");
  await closeSettings(page, settings);

  await page.reload();
  settings = await openSettings(page);
  await expect(actionRow(settings, "Primary attack")).toContainText("KeyJ");
  await expect(actionRow(settings, "Primary attack")).not.toContainText("KeyR");
  await closeSettings(page, settings);

  await page.getByRole("button", { name: "Pause", exact: true }).click();
  await page.evaluate(() => document.activeElement?.blur());
  const attack = page.getByRole("button", { name: "Primary attack", exact: true });
  const step = page.getByRole("button", { name: "Step", exact: true });

  await page.keyboard.press("r");
  await step.click();
  await page.evaluate(() => document.activeElement?.blur());
  await expect(attack).not.toHaveAttribute("data-phase", "windup");

  await page.keyboard.press("j");
  await step.click();
  await expect(attack).toHaveAttribute("data-phase", "windup");
});
