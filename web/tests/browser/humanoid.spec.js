// #55: the released Ranger bundle loads with the game core and stands in for every player.
import { expect, test } from "./fixtures.js";

const MANIFEST = "**/characters/humanoid-ranger-male-v1/manifest.json";

test("the game waits for the character bundle, then enters the world with the humanoid", async ({
  page,
  appUrl,
}, testInfo) => {
  const requested = [];
  page.on("request", (request) => {
    const path = new URL(request.url()).pathname;
    if (path.includes("/characters/")) requested.push(path);
  });
  await page.goto(appUrl);
  await expect(page.getByRole("status").first()).toHaveText("Ready", { timeout: 60_000 });
  expect(requested).toContain("/arpg/characters/humanoid-ranger-male-v1/manifest.json");
  expect(requested.some((path) => path.endsWith(".gltf"))).toBe(true);
  expect(requested.filter((path) => path.includes("/resources/"))).toHaveLength(14);

  await page.getByRole("button", { name: "Enter World", exact: true }).click();
  const canvas = page
    .getByRole("img", { name: "ARPG game world" })
    .or(page.locator("canvas.game-canvas"));
  await expect(canvas.first()).toBeVisible();
  await page.waitForTimeout(500);
  await canvas.first().screenshot({ path: testInfo.outputPath("in-world.png") });
  await testInfo.attach("in-world-humanoid", {
    body: await canvas.first().screenshot(),
    contentType: "image/png",
  });

  // Leaving and entering again reuses the loaded bundle.
  const before = requested.length;
  await page.getByRole("button", { name: "Characters", exact: true }).click();
  await page.getByRole("button", { name: "Enter World", exact: true }).click();
  await expect(canvas.first()).toBeVisible();
  expect(requested.length).toBe(before);
});

test("a missing character bundle is reported and Retry recovers", async ({ page, appUrl }) => {
  await page.route(MANIFEST, (route) => route.fulfill({ status: 404, body: "missing" }));
  await page.goto(appUrl);
  const status = page.getByRole("status").first();
  await expect(status).toContainText("Character assets failed to load", { timeout: 60_000 });
  await expect(page.getByRole("button", { name: "Enter World", exact: true })).toBeDisabled();

  await page.unroute(MANIFEST);
  await page.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(status).toHaveText("Ready", { timeout: 60_000 });
  await expect(page.getByRole("button", { name: "Enter World", exact: true })).toBeEnabled();
});
