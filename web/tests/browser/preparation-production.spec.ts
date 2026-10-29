import { test as base, expect, type Page } from "@playwright/test";
import { preview } from "vite";

// Serve the actual `bun run build` artifact; never fake WASM, snapshots or assets.
const test = base.extend<{ productionUrl: string }>({
  productionUrl: async ({ browserName }, use) => {
    if (browserName !== "chromium") {
      throw new Error("Acceptance requires Chromium for real WASM/WebGL");
    }
    const server = await preview({ preview: { host: "127.0.0.1", port: 0 } });
    const address = server.httpServer.address();
    if (!address || typeof address === "string") {
      throw new Error("Production preview has no TCP address");
    }
    try {
      await use(`http://127.0.0.1:${address.port}/arpg/`);
    } finally {
      server.httpServer.closeAllConnections();
      await new Promise<void>((resolve, reject) => {
        server.httpServer.close((error) => (error ? reject(error) : resolve()));
      });
    }
  },
});
const preparation = (page: Page) => page.getByRole("main", { name: /preparation/i });
const start = (page: Page) => page.getByRole("button", { name: "Start", exact: true });
const coreRequest = /arpg_web_wasm_bg[^/]*\.wasm(?:\?.*)?$/;

for (const input of ["keyboard", "touch"] as const) {
  test(`production preparation starts local play and re-enters using ${input}`, async ({
    browser,
    productionUrl,
  }, testInfo) => {
    const context = await browser.newContext(
      input === "touch"
        ? { viewport: { width: 390, height: 844 }, hasTouch: true, isMobile: true }
        : { viewport: { width: 1280, height: 800 } },
    );
    const page = await context.newPage();
    const errors: string[] = [];
    page.on("pageerror", (error) => errors.push(error.message));
    try {
      await page.goto(productionUrl);
      for (let round = 0; round < 2; round += 1) {
        await expect(preparation(page)).toBeVisible();
        await expect(preparation(page)).toContainText(/knight/i);
        await expect(preparation(page)).toContainText(/undead crypt/i);
        await expect(start(page)).toBeEnabled();
        await testInfo.attach(`preparation-${input}-${round}`, {
          body: await page.screenshot(),
          contentType: "image/png",
        });
        if (input === "keyboard") {
          await start(page).focus();
          await page.keyboard.press("Enter");
        } else {
          await start(page).tap();
        }
        await expect(preparation(page)).toBeHidden();
        await expect(page.getByLabel("ARPG game world", { exact: true })).toBeVisible();
        await expect(page.locator("canvas")).toHaveCount(1);
        await expect(page.getByText("Sword & shield", { exact: true })).toBeVisible();
        const guard = page.getByRole("button", { name: "Hold shield", exact: true });
        await expect(guard).toBeEnabled();
        // HUD guard phase is a projection of the real Rust/WASM authority.
        if (input === "keyboard") {
          await page.evaluate(() => document.activeElement?.blur());
          await page.keyboard.down("f");
          await expect(guard).toHaveAttribute("data-guard", "raised");
          await page.keyboard.up("f");
        } else {
          const box = await guard.boundingBox();
          if (!box) {
            throw new Error("Touch guard has no bounds");
          }
          const client = await context.newCDPSession(page);
          await client.send("Input.dispatchTouchEvent", {
            type: "touchStart",
            touchPoints: [{ x: box.x + box.width / 2, y: box.y + box.height / 2 }],
          });
          await expect(guard).toHaveAttribute("data-guard", "raised");
          await client.send("Input.dispatchTouchEvent", { type: "touchEnd", touchPoints: [] });
          await client.detach();
        }
        await expect(guard).not.toHaveAttribute("data-guard", /.+/);
        await page.getByRole("button", { name: "Settings", exact: true }).click();
        const settings = page.getByRole("dialog", { name: "Settings", exact: true });
        await expect(settings).toBeVisible();
        await settings.getByRole("button", { name: "Close", exact: true }).click();
        await testInfo.attach(`play-${input}-${round}`, {
          body: await page.screenshot(),
          contentType: "image/png",
        });
        expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
          true,
        );
        await page.getByRole("button", { name: "Characters", exact: true }).click();
        await expect(preparation(page)).toBeVisible();
        await expect(page.getByLabel("ARPG game world", { exact: true })).toHaveCount(0);
      }
      await page.reload();
      await expect(preparation(page)).toBeVisible();
      expect(errors).toEqual([]);
    } finally {
      await context.close();
    }
  });
}

test("production pending core prevents entry and cancel cannot publish late play", async ({
  page,
  productionUrl,
}) => {
  let release: () => void = () => {};
  const pending = new Promise<void>((resolve) => {
    release = resolve;
  });
  let requested: () => void = () => {};
  const seen = new Promise<void>((resolve) => {
    requested = resolve;
  });
  await page.route(coreRequest, async (route) => {
    requested();
    await pending;
    await route.continue();
  });
  try {
    await page.goto(productionUrl);
    await seen;
    await expect(preparation(page)).toBeVisible();
    await expect(preparation(page)).toContainText(/loading/i);
    await expect(start(page)).toBeDisabled();
    await expect(page.getByLabel("ARPG game world", { exact: true })).toHaveCount(0);
    await page.getByRole("button", { name: /cancel|back/i }).click();
    release();
    // The real runtime publishes Ready only after WASM initialization settles.
    // Its pending activation runs in that same completion callback, before React
    // can render this returned selection surface. A released HTTP route alone
    // does not establish cancellation of a late activation.
    await expect(
      page.getByRole("main", { name: "Character selection", exact: true }).getByRole("status"),
    ).toHaveText("Ready");
    await expect(preparation(page)).toBeHidden();
    await expect(page.getByLabel("ARPG game world", { exact: true })).toHaveCount(0);
    await page.reload();
    await expect(preparation(page)).toBeVisible();
    await expect(start(page)).toBeEnabled();
  } finally {
    release();
  }
});

test("production core failure recovers and duplicate Start yields one play surface", async ({
  page,
  productionUrl,
}) => {
  await page.route(coreRequest, (route) => route.abort("failed"));
  await page.goto(productionUrl);
  await expect(preparation(page)).toContainText(/fail|unable|error/i);
  await expect(start(page)).toBeDisabled();
  await expect(page.getByLabel("ARPG game world", { exact: true })).toHaveCount(0);
  await page.unroute(coreRequest);
  await page.getByRole("button", { name: "Retry", exact: true }).click();
  await expect(start(page)).toBeEnabled();
  await start(page).evaluate((button: HTMLButtonElement) => {
    button.click();
    button.click();
  });
  await expect(page.getByRole("button", { name: "Hold shield", exact: true })).toBeEnabled();
  await expect(page.locator("canvas")).toHaveCount(1);
});
