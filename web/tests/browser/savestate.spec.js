import { readFileSync } from "node:fs";
import { expect, test } from "./fixtures.js";

const SAVE_KEY = "arpg-save-document-v1";

// The core's checked-in save fixtures (#68, crates/arpg-core/tests/save_fixtures.rs).
const coreFixture = (name) =>
  readFileSync(
    new URL(`../../../crates/arpg-core/tests/fixtures/saves/${name}`, import.meta.url),
    "utf8",
  );

const saveFile = (coreStateText) =>
  `{"format":"arpg-save","fileVersion":1,"presentation":{"characterId":"aldren"},` +
  `"client":{"controlledPlayerId":1},"coreState":${coreStateText}}`;

const hudStatus = (page) => page.getByRole("region", { name: "Player status" }).locator("p");

async function saveLocally(page) {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Settings", exact: true });
  await settings.getByRole("button", { name: "Save game", exact: true }).click();
  await expect(settings.getByText("Local save available")).toBeVisible();
  await settings.getByRole("button", { name: "Close", exact: true }).click();
  await expect(hudStatus(page).first()).toContainText("Game saved locally");
}

test("a local save survives a reload and restores the world", async ({ page, appUrl }) => {
  await page.goto(appUrl);
  await page.getByRole("button", { name: "Enter World", exact: true }).click();
  await saveLocally(page);

  await page.reload();
  await page.getByRole("button", { name: "Load Saved Game", exact: true }).click();
  await expect(hudStatus(page).first()).toContainText("Loaded local save");
});

test("an imported save stays loaded when browser storage cannot keep it", async ({
  page,
  appUrl,
}) => {
  await page.goto(appUrl);
  await page.getByRole("button", { name: "Enter World", exact: true }).click();
  await saveLocally(page);
  const saveText = await page.evaluate((key) => localStorage.getItem(key), SAVE_KEY);
  expect(saveText).toContain('"format": "arpg-save"');

  await page.evaluate(() => localStorage.clear());
  await page.addInitScript((key) => {
    const setItem = Storage.prototype.setItem;
    Storage.prototype.setItem = function (name, value) {
      if (name === key) throw new DOMException("Quota exceeded", "QuotaExceededError");
      return setItem.call(this, name, value);
    };
  }, SAVE_KEY);
  await page.reload();

  await page.getByLabel("Save file").setInputFiles({
    name: "portable.json",
    mimeType: "application/json",
    buffer: Buffer.from(saveText),
  });
  const status = hudStatus(page).first();
  await expect(status).toContainText("Imported portable.json");
  await expect(status).toContainText("not kept as the local save");
  await expect(page.getByRole("button", { name: "Settings", exact: true })).toBeVisible();
});

test("refused save imports keep the running game and the local save", async ({ page, appUrl }) => {
  await page.goto(appUrl);
  await page.getByRole("button", { name: "Enter World", exact: true }).click();
  await saveLocally(page);
  const kept = await page.evaluate((key) => localStorage.getItem(key), SAVE_KEY);
  const status = hudStatus(page).first();

  const current = saveFile(coreFixture("current.json"));
  const refused = [
    [
      "previous-schema.json",
      saveFile(coreFixture("previous-schema.json")),
      "unsupported ARPG save schema version",
    ],
    [
      "corrupt.json",
      saveFile(coreFixture("corrupt.json")),
      "saved player checkpoint is not a cleared room",
    ],
    ["partial.json", current.slice(0, current.length / 2), "SyntaxError"],
    ["oversized.json", current + " ".repeat(4 * 1024 * 1024), "exceeds"],
  ];
  const importFile = async (name, text) => {
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    const settings = page.getByRole("dialog", { name: "Settings", exact: true });
    await settings.getByLabel("Save file").setInputFiles({
      name,
      mimeType: "application/json",
      buffer: Buffer.from(text),
    });
    return settings;
  };
  for (const [name, text, reason] of refused) {
    const settings = await importFile(name, text);
    await expect(status).toContainText("Could not import save");
    await expect(status).toContainText(reason);
    expect(await page.evaluate((key) => localStorage.getItem(key), SAVE_KEY)).toBe(kept);
    await settings.getByRole("button", { name: "Close", exact: true }).click();
    await expect(page.getByRole("button", { name: "Settings", exact: true })).toBeVisible();
  }

  await importFile("current.json", current);
  await expect(status).toContainText("Imported current.json");
  await expect(status).toContainText("tick 50");
});
