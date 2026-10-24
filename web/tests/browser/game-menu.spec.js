import { expect, test } from "./fixtures.js";

// The in-world game menu (#60): Escape opens Resume / Settings / Leave, steps out of the
// innermost layer, pauses only local play and confirms leaving a run.

const menuOf = (page) => page.getByRole("dialog", { name: "Menu", exact: true });
const confirmOf = (page) => page.getByRole("dialog", { name: "Leave game?", exact: true });
const settingsOf = (page) => page.getByRole("dialog", { name: "Settings", exact: true });
const characterScreen = (page) => page.getByRole("main", { name: "Character selection" });

test("Escape opens the menu, nests settings and confirms leaving a run", async ({
  page,
  appUrl,
}) => {
  await page.goto(appUrl);
  await page.getByRole("button", { name: "Enter World", exact: true }).click();
  const guard = page.getByRole("button", { name: "Hold shield", exact: true });
  await expect(guard).toBeEnabled();
  await page.evaluate(() => document.activeElement?.blur());

  await page.keyboard.press("Escape");
  const menu = menuOf(page);
  await expect(menu).toBeVisible();
  await expect(menu.getByRole("status")).toHaveText("Game paused");
  await expect(menu.getByRole("button", { name: "Resume", exact: true })).toBeFocused();
  // Gameplay keys do not reach the world while the menu is open.
  await page.keyboard.down("f");
  await page.waitForTimeout(200);
  await page.keyboard.up("f");
  await page.keyboard.press("Escape");
  await expect(menu).not.toBeVisible();
  await expect(guard).not.toHaveAttribute("data-guard", /.+/);

  // Settings nest inside the menu; Escape returns to the menu with Settings focused.
  await page.keyboard.press("Escape");
  await menu.getByRole("button", { name: "Settings", exact: true }).click();
  await expect(settingsOf(page)).toBeVisible();
  await expect(menu).not.toBeVisible();
  await page.keyboard.press("Escape");
  await expect(settingsOf(page)).not.toBeVisible();
  await expect(menu.getByRole("button", { name: "Settings", exact: true })).toBeFocused();

  // Leaving a run asks first; Escape and Cancel keep playing from the menu.
  await menu.getByRole("button", { name: "Leave", exact: true }).click();
  const confirm = confirmOf(page);
  await expect(confirm).toContainText("Progress since your last save is lost.");
  await expect(confirm.getByRole("button", { name: "Cancel", exact: true })).toBeFocused();
  await page.keyboard.press("Escape");
  await expect(confirm).not.toBeVisible();
  await expect(menu.getByRole("button", { name: "Leave", exact: true })).toBeFocused();
  await menu.getByRole("button", { name: "Leave", exact: true }).click();
  await confirm.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(menu).toBeVisible();
  await menu.getByRole("button", { name: "Resume", exact: true }).click();
  await expect(guard).toBeEnabled();

  await page.getByRole("button", { name: "Menu", exact: true }).click();
  await menu.getByRole("button", { name: "Leave", exact: true }).click();
  await confirm.getByRole("button", { name: "Leave game", exact: true }).click();
  await expect(characterScreen(page)).toBeVisible();
  await expect(page.locator("canvas")).toHaveCount(1);
});

test("the menu pauses local training, which leaves without confirmation", async ({
  page,
  appUrl,
}) => {
  await page.goto(`${appUrl}?scenario=training&seed=42`);
  const panel = page.getByRole("complementary", { name: "Training arena controls" });
  const tickOf = async () => Number((await panel.getByText(/^Tick \d+$/).textContent()).slice(5));
  await expect.poll(tickOf).toBeGreaterThan(5);

  await page.getByRole("button", { name: "Menu", exact: true }).click();
  await expect(menuOf(page)).toBeVisible();
  const paused = await tickOf();
  await page.waitForTimeout(300);
  expect(await tickOf()).toBe(paused);
  await menuOf(page).getByRole("button", { name: "Resume", exact: true }).click();
  await expect.poll(tickOf).toBeGreaterThan(paused);

  await page.getByRole("button", { name: "Menu", exact: true }).click();
  await menuOf(page).getByRole("button", { name: "Leave", exact: true }).click();
  await expect(confirmOf(page)).toHaveCount(0);
  await expect(characterScreen(page)).toBeVisible();
  await expect(page).not.toHaveURL(/scenario=training/);
});

test("browser Back asks to leave and reload never resumes a session", async ({ page, appUrl }) => {
  await page.goto(appUrl);
  await page.getByRole("button", { name: "Enter World", exact: true }).click();
  await expect(page.getByRole("button", { name: "Hold shield", exact: true })).toBeEnabled();

  await page.goBack();
  const confirm = confirmOf(page);
  await expect(confirm).toBeVisible();
  await confirm.getByRole("button", { name: "Cancel", exact: true }).click();
  await expect(menuOf(page)).toBeVisible();
  await menuOf(page).getByRole("button", { name: "Resume", exact: true }).click();

  await page.goBack();
  await confirm.getByRole("button", { name: "Leave game", exact: true }).click();
  await expect(characterScreen(page)).toBeVisible();

  await page.getByRole("button", { name: "Enter World", exact: true }).click();
  await expect(page.getByRole("button", { name: "Hold shield", exact: true })).toBeEnabled();
  await page.reload();
  await expect(characterScreen(page)).toBeVisible();
  await page.getByRole("button", { name: "Enter World", exact: true }).click();
  await page.goBack();
  await expect(confirm).toBeVisible();
});
