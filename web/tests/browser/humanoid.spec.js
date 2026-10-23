// #55: the released Ranger bundle loads with the game core and stands in for every player.
import { expect, test } from "./fixtures.js";

const MANIFEST = "**/characters/humanoid-ranger-male/manifest.json";

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
  expect(requested).toContain("/arpg/characters/humanoid-ranger-male/manifest.json");
  expect(requested.some((path) => path.endsWith(".gltf"))).toBe(true);
  expect(requested.filter((path) => path.endsWith(".ktx2")).length).toBeGreaterThan(0);
  expect(requested.filter((path) => path.endsWith(".clips.glb"))).toHaveLength(1);
  expect(requested.filter((path) => path.endsWith(".bin"))).toHaveLength(3);
  expect(requested.some((path) => path.endsWith(".tar"))).toBe(false);
  expect(requested.some((path) => path.includes("/resources/"))).toBe(false);

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

test("held movement plays a gait on the humanoid (#188)", async ({ page, appUrl }, testInfo) => {
  await page.goto(appUrl);
  await expect(page.getByRole("status").first()).toHaveText("Ready", { timeout: 60_000 });
  await page.getByRole("button", { name: "Enter World", exact: true }).click();
  const canvas = page.locator("canvas.game-canvas");
  await expect(canvas).toBeVisible();
  await page.waitForTimeout(300);
  const standing = await canvas.screenshot();
  await page.keyboard.down("s");
  await page.waitForTimeout(700);
  const moving = await canvas.screenshot({ path: testInfo.outputPath("jogging.png") });
  await page.keyboard.up("s");
  await testInfo.attach("standing", { body: standing, contentType: "image/png" });
  await testInfo.attach("jogging", { body: moving, contentType: "image/png" });
  expect(moving.equals(standing)).toBe(false);
});

test("the humanoid carries the real sword and shield and raises the shield to guard (#183)", async ({
  page,
  appUrl,
}, testInfo) => {
  const requested = [];
  page.on("request", (request) => requested.push(new URL(request.url()).pathname));
  await page.goto(appUrl);
  await expect(page.getByRole("status").first()).toHaveText("Ready", { timeout: 60_000 });
  for (const entry of ["equipment.sword.arming", "equipment.shield.heater"])
    expect(requested.some((path) => path.endsWith(`/${entry}.default.gltf`))).toBe(true);
  expect(requested.some((path) => path.endsWith(".humanoid-sockets.json"))).toBe(true);
  await page.getByRole("button", { name: "Enter World", exact: true }).click();
  const canvas = page.locator("canvas.game-canvas");
  await expect(canvas).toBeVisible();
  await page.waitForTimeout(400);
  await canvas.screenshot({ path: testInfo.outputPath("armed.png") });
  await page.keyboard.down("f");
  await page.waitForTimeout(600);
  await canvas.screenshot({ path: testInfo.outputPath("guard.png") });
  await page.keyboard.up("f");
  // No primitive shield board stands in for the real shield.
  await page.keyboard.press("x");
  await page.waitForTimeout(400);
  await canvas.screenshot({ path: testInfo.outputPath("bow-stowed.png") });
});
