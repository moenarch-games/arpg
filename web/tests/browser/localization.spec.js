// Browser acceptance for localization-ready UI text (#61, 61b). Player-facing text comes
// from the English message catalog (`src/messages.ts`); the development-only
// `?locale=x-long` test locale renders every message about 60% longer, and the shell must
// keep its actions visible, inside the viewport and without clipped or overflowing text on
// a desktop and a narrow phone.
import { expect, test } from "./fixtures.js";
import { longLabelCatalog } from "../../src/messages.ts";

const long = longLabelCatalog();

const VIEWPORTS = [
  { name: "desktop", width: 1280, height: 800, touch: false },
  { name: "narrow phone", width: 360, height: 740, touch: true },
];

for (const viewport of VIEWPORTS) {
  test(`long labels keep the ${viewport.name} layout intact`, async ({ browser, appUrl }) => {
    const { width, height } = viewport;
    const context = await browser.newContext({
      viewport: { width, height },
      hasTouch: viewport.touch,
      isMobile: viewport.touch,
    });
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    const fits = async (locator) => {
      await expect(locator).toBeVisible();
      await locator.scrollIntoViewIfNeeded();
      const box = await locator.boundingBox();
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.y).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width).toBeLessThanOrEqual(width);
      expect(box.y + box.height).toBeLessThanOrEqual(height);
      // Text stays inside its control instead of overflowing or being clipped.
      const overflow = await locator.evaluate((element) => ({
        x: element.scrollWidth - element.clientWidth,
        y: element.scrollHeight - element.clientHeight,
      }));
      const label = await locator.textContent();
      expect(overflow.x, `${label} overflows horizontally`).toBeLessThanOrEqual(1);
      expect(overflow.y, `${label} overflows vertically`).toBeLessThanOrEqual(1);
      return box;
    };
    const noPageScroll = async () =>
      expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
        true,
      );
    const button = (scope, key) => scope.getByRole("button", { name: long[key], exact: true });
    try {
      await page.goto(`${appUrl}?locale=x-long`);
      const enter = button(page, "select.enterWorld");
      await expect(enter).toBeEnabled();
      await expect(page.getByRole("status")).toHaveText(long["select.ready"]);
      for (const key of ["select.loadSaved", "select.importSave", "select.training"])
        await fits(button(page, key));
      await fits(enter);
      await noPageScroll();

      await enter.click();
      const header = [button(page, "header.menu"), button(page, "header.settings")];
      const boxes = [];
      for (const locator of header) boxes.push(await fits(locator));
      await expect(page.getByRole("region", { name: long["hud.region"] })).toContainText(
        long["hud.weapon.sword"],
      );
      if (viewport.touch) {
        const actions = await page
          .getByRole("region", { name: long["combat.region"] })
          .getByRole("button")
          .all();
        expect(actions.length).toBeGreaterThanOrEqual(5);
        for (const locator of actions) boxes.push(await fits(locator));
      }
      for (const [index, box] of boxes.entries())
        for (const other of boxes.slice(index + 1))
          expect(
            box.x + box.width <= other.x ||
              other.x + other.width <= box.x ||
              box.y + box.height <= other.y ||
              other.y + other.height <= box.y,
          ).toBe(true);
      await noPageScroll();

      await button(page, "header.menu").click();
      const menu = page.getByRole("dialog", { name: long["menu.heading"], exact: true });
      // Narrow layouts hide the menu notice (existing styles), so only the desktop reads it.
      if (!viewport.touch) await expect(menu.getByRole("status")).toHaveText(long["menu.paused"]);
      for (const key of ["menu.resume", "menu.settings", "menu.leave"])
        await fits(button(menu, key));

      await button(menu, "menu.settings").click();
      const settings = page.getByRole("dialog", { name: long["settings.heading"], exact: true });
      await fits(button(settings, "settings.close"));
      for (const key of ["settings.saveGame", "settings.resetDisplay", "settings.connectDedicated"])
        await fits(button(settings, key));
      await fits(settings.getByLabel(long["settings.motion"], { exact: true }));
      expect(await settings.evaluate((dialog) => dialog.scrollWidth <= dialog.clientWidth)).toBe(
        true,
      );
      await button(settings, "settings.close").click();

      await button(menu, "menu.leave").click();
      const leave = page.getByRole("dialog", { name: long["leave.heading"], exact: true });
      await expect(leave).toContainText(long["leave.local"]);
      await fits(button(leave, "leave.cancel"));
      await fits(button(leave, "leave.confirm"));
      await noPageScroll();
      expect(errors).toEqual([]);
    } finally {
      await context.close();
    }
  });
}
