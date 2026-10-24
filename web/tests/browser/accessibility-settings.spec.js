// Browser acceptance for accessibility presentation settings (#61, first slice).
//
// Stored shape (see src/presentation-preferences.test.js):
//   localStorage["arpg-presentation"] = { version: 2,
//     graphics: { shadows, pixelRatioLimit },
//     accessibility: { motion: "system" | "reduced" | "full", reducedFlashes, textScale },
//     camera: { zoom } }   // the follow-camera zoom (wheel, pinch, C resets)
// migrated read-only from the legacy `arpg-graphics-v1` document.
//
// UI contract, inside the existing Settings dialog (no second settings surface):
// - a section named by its heading, `region` "Accessibility", holding in this tab order:
//   - select "Motion effects": options "Follow system" (system), "Reduced" (reduced),
//     "Full" (full);
//   - checkbox "Reduce bright flashes";
//   - select "Text size": options "100%" (1), "125%" (1.25), "150%" (1.5);
//   - button "Reset display settings", which restores the Graphics and Accessibility
//     defaults (not the input profile or mouse aim) and keeps focus on itself.
// - The existing Graphics controls ("Enable shadows", "Pixel ratio limit") read and write
//   the same versioned document.
//
// Effects, applied immediately while playing (no reload, no new authority, no new renderer):
//   <html style="font-size: 150%" data-motion="reduced|full" data-flashes="reduced|full">
// `data-motion` is the resolved value: "system" follows `prefers-reduced-motion` live.
// Under data-motion="reduced", CSS transitions/hover transforms are disabled.
import { expect, leaveWorld, test } from "./fixtures.js";

const KEY = "arpg-presentation";
const LEGACY_GRAPHICS_KEY = "arpg-graphics-v1";
const MOUSE_AIM_KEY = "arpg-mouse-aim-v1";
const TRAINING = "?scenario=training&seed=42";
const DEFAULT_DOCUMENT = {
  version: 2,
  graphics: { shadows: true, pixelRatioLimit: 2 },
  accessibility: { motion: "system", reducedFlashes: false, combatEffects: true, textScale: 1 },
  camera: { zoom: 1 },
};

// Counts WebGL/WebGPU context requests: every scene renderer creation asks for one, so an
// unchanged count proves the renderer was not recreated.
function countRendererContexts() {
  const original = HTMLCanvasElement.prototype.getContext;
  window.__rendererContexts = 0;
  HTMLCanvasElement.prototype.getContext = function getContext(type, ...rest) {
    if (/^(webgl2?|experimental-webgl|webgpu)$/.test(type)) window.__rendererContexts += 1;
    return original.call(this, type, ...rest);
  };
}

// Seeds localStorage once per test, before the app's first read, and never again on reload.
async function seedStorage(page, entries) {
  await page.addInitScript((values) => {
    if (sessionStorage.getItem("__seeded")) return;
    sessionStorage.setItem("__seeded", "1");
    for (const [key, value] of Object.entries(values)) localStorage.setItem(key, value);
  }, entries);
}

const root = (page) => page.locator("html");
const rootFontSize = (page) =>
  page.evaluate(() => getComputedStyle(document.documentElement).fontSize);
const storedDocument = (page) =>
  page.evaluate((key) => JSON.parse(localStorage.getItem(key) ?? "null"), KEY);
const tickOf = async (page) =>
  Number(
    (
      await page
        .getByRole("complementary", { name: "Training arena controls" })
        .getByText(/^Tick \d+$/)
        .textContent()
    ).slice(5),
  );

async function openSettings(page) {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Settings", exact: true });
  await expect(settings).toBeVisible();
  const accessibility = settings.getByRole("region", { name: "Accessibility", exact: true });
  return {
    settings,
    accessibility,
    motion: accessibility.getByLabel("Motion effects", { exact: true }),
    flashes: accessibility.getByLabel("Reduce bright flashes", { exact: true }),
    textSize: accessibility.getByLabel("Text size", { exact: true }),
    reset: settings.getByRole("button", { name: "Reset display settings", exact: true }),
    close: settings.getByRole("button", { name: "Close", exact: true }),
  };
}

test("accessibility settings apply during play without a new authority or renderer", async ({
  page,
  appUrl,
}) => {
  await page.addInitScript(countRendererContexts);
  await page.goto(`${appUrl}${TRAINING}`);
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  const step = page.getByRole("button", { name: "Step", exact: true });
  await page.getByRole("button", { name: "Primary attack", exact: true }).click();
  for (let tick = 0; tick < 3; tick += 1) await step.click();
  const tick = await tickOf(page);
  const contexts = await page.evaluate(() => window.__rendererContexts);
  expect(contexts).toBeGreaterThan(0);
  await expect(root(page)).toHaveAttribute("data-motion", "full");
  await expect(root(page)).toHaveAttribute("data-flashes", "full");
  expect(await rootFontSize(page)).toBe("16px");

  const ui = await openSettings(page);
  await expect(ui.motion).toHaveValue("system");
  await expect(ui.flashes).not.toBeChecked();
  await expect(ui.textSize).toHaveValue("1");
  await ui.motion.selectOption("reduced");
  await expect(root(page)).toHaveAttribute("data-motion", "reduced");
  await ui.flashes.check();
  await expect(root(page)).toHaveAttribute("data-flashes", "reduced");
  await ui.textSize.selectOption("1.5");
  expect(await rootFontSize(page)).toBe("24px");
  await ui.close.click();
  await expect(ui.settings).not.toBeVisible();

  // Same paused authority (a recreated one would restart its tick) and the same renderer.
  expect(await tickOf(page)).toBe(tick);
  expect(await page.evaluate(() => window.__rendererContexts)).toBe(contexts);
  await expect(page.locator("canvas")).toHaveCount(1);
  await step.click();
  await expect.poll(() => tickOf(page)).toBe(tick + 1);

  // Positive control: a creation-time graphics option does replace the renderer, so the
  // probe above would have seen a recreation.
  const graphics = await openSettings(page);
  await graphics.settings.getByLabel("Enable shadows").uncheck();
  await graphics.close.click();
  await expect.poll(() => page.evaluate(() => window.__rendererContexts)).toBeGreaterThan(contexts);
  expect(await tickOf(page)).toBe(tick + 1);
});

test("accessibility settings persist across reload as one version 2 document", async ({
  page,
  appUrl,
}) => {
  await page.goto(`${appUrl}${TRAINING}`);
  let ui = await openSettings(page);
  await ui.motion.selectOption("full");
  await ui.flashes.check();
  await ui.textSize.selectOption("1.25");
  await ui.settings.getByLabel("Pixel ratio limit").selectOption("1.5");
  expect(await storedDocument(page)).toEqual({
    version: 2,
    graphics: { shadows: true, pixelRatioLimit: 1.5 },
    accessibility: { motion: "full", reducedFlashes: true, combatEffects: true, textScale: 1.25 },
    camera: { zoom: 1 },
  });

  await page.reload();
  await expect(page.getByRole("button", { name: "Settings", exact: true })).toBeVisible();
  await expect(root(page)).toHaveAttribute("data-motion", "full");
  await expect(root(page)).toHaveAttribute("data-flashes", "reduced");
  expect(await rootFontSize(page)).toBe("20px");
  ui = await openSettings(page);
  await expect(ui.motion).toHaveValue("full");
  await expect(ui.flashes).toBeChecked();
  await expect(ui.textSize).toHaveValue("1.25");
  await expect(ui.settings.getByLabel("Pixel ratio limit")).toHaveValue("1.5");
  // Every choice shows its value as readable text, not only as a position.
  await expect(ui.textSize.locator("option:checked")).toHaveText("125%");
  await expect(ui.motion.locator("option:checked")).toHaveText("Full");
});

test("the camera zoom persists in the presentation document across reload", async ({
  page,
  appUrl,
}) => {
  await page.goto(`${appUrl}${TRAINING}`);
  const canvas = page.locator("canvas");
  await expect(canvas).toHaveCount(1);
  // Wheel toward the user zooms out: 120px is e^(120/600) of the default offset.
  await canvas.dispatchEvent("wheel", { deltaY: 120, deltaMode: 0 });
  await expect.poll(async () => (await storedDocument(page))?.camera.zoom).toBeCloseTo(1.2214, 3);
  const zoom = (await storedDocument(page)).camera.zoom;

  // A display setting change keeps the zoom instead of restoring a stale one.
  const ui = await openSettings(page);
  await ui.textSize.selectOption("1.25");
  await ui.reset.click();
  expect((await storedDocument(page)).camera.zoom).toBe(zoom);
  await ui.close.click();

  await page.reload();
  await expect(page.locator("canvas")).toHaveCount(1);
  expect((await storedDocument(page)).camera.zoom).toBe(zoom);
  // The camera reset key restores and saves the default zoom.
  await expect(page.getByRole("button", { name: "Pause", exact: true })).toBeVisible();
  await page.evaluate(() => document.activeElement?.blur());
  await page.keyboard.press("c");
  await expect.poll(async () => (await storedDocument(page)).camera.zoom).toBe(1);
  expect(await storedDocument(page)).toEqual(DEFAULT_DOCUMENT);
});

test("legacy graphics settings migrate and the legacy value is kept", async ({ page, appUrl }) => {
  await seedStorage(page, {
    [LEGACY_GRAPHICS_KEY]: JSON.stringify({ shadows: false, pixelRatioLimit: 1 }),
  });
  await page.goto(`${appUrl}${TRAINING}`);
  const ui = await openSettings(page);
  await expect(ui.settings.getByLabel("Enable shadows")).not.toBeChecked();
  await expect(ui.settings.getByLabel("Pixel ratio limit")).toHaveValue("1");
  await expect(ui.motion).toHaveValue("system");
  await expect(ui.flashes).not.toBeChecked();
  await expect(ui.textSize).toHaveValue("1");
  // Loading migrates in memory only.
  expect(await storedDocument(page)).toBeNull();

  await ui.textSize.selectOption("1.5");
  expect(await storedDocument(page)).toEqual({
    version: 2,
    graphics: { shadows: false, pixelRatioLimit: 1 },
    accessibility: { motion: "system", reducedFlashes: false, combatEffects: true, textScale: 1.5 },
    camera: { zoom: 1 },
  });
  expect(
    await page.evaluate((key) => JSON.parse(localStorage.getItem(key)), LEGACY_GRAPHICS_KEY),
  ).toEqual({ shadows: false, pixelRatioLimit: 1 });
});

test("invalid stored settings fall back to safe defaults", async ({ page, appUrl }) => {
  await seedStorage(page, {
    [KEY]: JSON.stringify({
      version: 2,
      graphics: { shadows: "off", pixelRatioLimit: 8 },
      accessibility: { motion: "slow", reducedFlashes: "yes", textScale: 4 },
    }),
  });
  await page.goto(`${appUrl}${TRAINING}`);
  await expect(page.getByRole("button", { name: "Primary attack", exact: true })).toBeVisible();
  expect(await rootFontSize(page)).toBe("16px");
  await expect(root(page)).toHaveAttribute("data-flashes", "full");
  const ui = await openSettings(page);
  await expect(ui.motion).toHaveValue("system");
  await expect(ui.flashes).not.toBeChecked();
  await expect(ui.textSize).toHaveValue("1");
  await expect(ui.settings.getByLabel("Enable shadows")).toBeChecked();
  await expect(ui.settings.getByLabel("Pixel ratio limit")).toHaveValue("2");
});

test("reset restores display defaults, keeps input preferences and keeps focus", async ({
  page,
  appUrl,
}) => {
  await page.goto(`${appUrl}${TRAINING}`);
  const ui = await openSettings(page);
  await ui.motion.selectOption("reduced");
  await ui.flashes.check();
  await ui.textSize.selectOption("1.5");
  await ui.settings.getByLabel("Enable shadows").uncheck();
  const mouseAim = ui.settings.getByLabel("Aim melee and bow with the mouse");
  // Mouse facing is on by default; opting out stores an explicit input preference.
  await mouseAim.uncheck();

  await ui.reset.focus();
  await page.keyboard.press("Enter");
  await expect(ui.reset).toBeFocused();
  await expect(ui.motion).toHaveValue("system");
  await expect(ui.flashes).not.toBeChecked();
  await expect(ui.textSize).toHaveValue("1");
  await expect(ui.settings.getByLabel("Enable shadows")).toBeChecked();
  await expect(ui.settings.getByLabel("Pixel ratio limit")).toHaveValue("2");
  expect(await rootFontSize(page)).toBe("16px");
  await expect(root(page)).toHaveAttribute("data-motion", "full");
  await expect(root(page)).toHaveAttribute("data-flashes", "full");
  expect(await storedDocument(page)).toEqual(DEFAULT_DOCUMENT);
  // Input preferences have their own owner and reset.
  await expect(mouseAim).not.toBeChecked();
  expect(await page.evaluate((key) => localStorage.getItem(key), MOUSE_AIM_KEY)).toBe("off");
});

test("the system motion preference is followed live unless overridden", async ({
  page,
  appUrl,
}) => {
  await page.emulateMedia({ reducedMotion: "reduce" });
  await page.goto(`${appUrl}${TRAINING}`);
  await expect(root(page)).toHaveAttribute("data-motion", "reduced");
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await expect(root(page)).toHaveAttribute("data-motion", "full");
  await page.emulateMedia({ reducedMotion: "reduce" });
  await expect(root(page)).toHaveAttribute("data-motion", "reduced");

  const ui = await openSettings(page);
  await ui.motion.selectOption("full");
  await expect(root(page)).toHaveAttribute("data-motion", "full");
  await ui.motion.selectOption("reduced");
  await page.emulateMedia({ reducedMotion: "no-preference" });
  await expect(root(page)).toHaveAttribute("data-motion", "reduced");
  await ui.close.click();

  // Reduced motion removes the UI's motion: the character cards no longer slide.
  await leaveWorld(page);
  const card = page.locator(".character-card").first();
  await expect(card).toBeVisible();
  const motionOf = () =>
    card.evaluate((element) => {
      const style = getComputedStyle(element);
      return { duration: style.transitionDuration, transform: style.transform };
    });
  await card.hover();
  expect(await motionOf()).toEqual({ duration: "0s", transform: "none" });
});

test("settings are keyboard operable in order and focus returns to the opener", async ({
  page,
  appUrl,
}) => {
  await page.goto(`${appUrl}${TRAINING}`);
  const opener = page.getByRole("button", { name: "Settings", exact: true });
  await opener.focus();
  await page.keyboard.press("Enter");
  const settings = page.getByRole("dialog", { name: "Settings", exact: true });
  const accessibility = settings.getByRole("region", { name: "Accessibility", exact: true });
  const motion = accessibility.getByLabel("Motion effects", { exact: true });
  const flashes = accessibility.getByLabel("Reduce bright flashes", { exact: true });
  const combatEffects = accessibility.getByLabel("Combat particles and camera impulse", {
    exact: true,
  });
  const textSize = accessibility.getByLabel("Text size", { exact: true });
  const reset = settings.getByRole("button", { name: "Reset display settings", exact: true });
  await expect(settings.getByRole("button", { name: "Close", exact: true })).toBeFocused();

  await motion.focus();
  await page.keyboard.press("Tab");
  await expect(flashes).toBeFocused();
  await page.keyboard.press("Space");
  await expect(flashes).toBeChecked();
  await page.keyboard.press("Tab");
  await expect(combatEffects).toBeFocused();
  await page.keyboard.press("Space");
  await expect(combatEffects).not.toBeChecked();
  await page.keyboard.press("Tab");
  await expect(textSize).toBeFocused();
  await page.keyboard.press("ArrowDown");
  await expect(textSize).toHaveValue("1.25");
  // Re-layout from the larger text keeps focus on the control being changed.
  await expect(textSize).toBeFocused();
  expect(await rootFontSize(page)).toBe("20px");
  await page.keyboard.press("Tab");
  await expect(reset).toBeFocused();
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.press("Shift+Tab");
  await page.keyboard.press("Shift+Tab");
  await expect(motion).toBeFocused();

  await page.keyboard.press("Escape");
  await expect(settings).not.toBeVisible();
  await expect(opener).toBeFocused();
  await page.keyboard.press("Enter");
  await expect(settings.getByRole("button", { name: "Close", exact: true })).toBeFocused();
  await expect(flashes).toBeChecked();
  await expect(combatEffects).not.toBeChecked();
  await expect(textSize).toHaveValue("1.25");
});

test("enlarged text on a narrow phone keeps every required action visible and operable", async ({
  browser,
  appUrl,
}) => {
  const width = 360;
  const height = 740;
  const context = await browser.newContext({
    viewport: { width, height },
    hasTouch: true,
    isMobile: true,
  });
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  try {
    await seedStorage(page, {
      [KEY]: JSON.stringify({
        ...DEFAULT_DOCUMENT,
        accessibility: {
          motion: "system",
          reducedFlashes: false,
          combatEffects: true,
          textScale: 1.5,
        },
      }),
    });
    await page.goto(`${appUrl}${TRAINING}`);
    await expect(page.getByRole("button", { name: "Primary attack", exact: true })).toBeVisible();
    expect(await rootFontSize(page)).toBe("24px");

    const inViewport = async (locator) => {
      await expect(locator).toBeVisible();
      const box = await locator.boundingBox();
      expect(box.x).toBeGreaterThanOrEqual(0);
      expect(box.y).toBeGreaterThanOrEqual(0);
      expect(box.x + box.width).toBeLessThanOrEqual(width);
      expect(box.y + box.height).toBeLessThanOrEqual(height);
      return box;
    };
    const required = [
      page.getByRole("button", { name: "Menu", exact: true }),
      page.getByRole("button", { name: "Settings", exact: true }),
      page.getByRole("group", { name: "Movement joystick" }),
      ...(await page.getByRole("region", { name: "Combat actions" }).getByRole("button").all()),
    ];
    expect(required.length).toBeGreaterThanOrEqual(7);
    const boxes = [];
    for (const locator of required) boxes.push(await inViewport(locator));
    for (const [index, box] of boxes.entries())
      for (const other of boxes.slice(index + 1))
        expect(
          box.x + box.width <= other.x ||
            other.x + other.width <= box.x ||
            box.y + box.height <= other.y ||
            other.y + other.height <= box.y,
        ).toBe(true);
    expect(await page.evaluate(() => document.documentElement.scrollWidth <= innerWidth)).toBe(
      true,
    );

    // Operable by touch alone, without hover.
    await page.getByRole("button", { name: "Pause", exact: true }).tap();
    const attack = page.getByRole("button", { name: "Primary attack", exact: true });
    await attack.tap();
    await page.getByRole("button", { name: "Step", exact: true }).tap();
    await expect(attack).toHaveAttribute("data-phase", "windup");

    await page.getByRole("button", { name: "Settings", exact: true }).tap();
    const settings = page.getByRole("dialog", { name: "Settings", exact: true });
    await expect(settings).toBeVisible();
    await inViewport(settings.getByRole("button", { name: "Close", exact: true }));
    const accessibility = settings.getByRole("region", { name: "Accessibility", exact: true });
    for (const control of [
      accessibility.getByLabel("Motion effects", { exact: true }),
      accessibility.getByLabel("Reduce bright flashes", { exact: true }),
      accessibility.getByLabel("Text size", { exact: true }),
      settings.getByRole("button", { name: "Reset display settings", exact: true }),
    ]) {
      await control.scrollIntoViewIfNeeded();
      await inViewport(control);
    }
    expect(await settings.evaluate((dialog) => dialog.scrollWidth <= dialog.clientWidth)).toBe(
      true,
    );
    await accessibility.getByLabel("Reduce bright flashes", { exact: true }).tap();
    await expect(accessibility.getByLabel("Reduce bright flashes", { exact: true })).toBeChecked();
    await settings.getByRole("button", { name: "Close", exact: true }).tap();
    await expect(settings).not.toBeVisible();
    expect(errors).toEqual([]);
  } finally {
    await context.close();
  }
});

test("denied or full storage keeps settings for the session without breaking play", async ({
  browser,
  appUrl,
}) => {
  for (const failure of ["denied", "quota"]) {
    const context = await browser.newContext();
    const page = await context.newPage();
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    try {
      await page.addInitScript((mode) => {
        if (mode === "denied") {
          Object.defineProperty(window, "localStorage", {
            get() {
              throw new DOMException("Blocked", "SecurityError");
            },
          });
        } else {
          const setItem = Storage.prototype.setItem;
          Storage.prototype.setItem = function guardedSetItem(key, value) {
            if (key === "arpg-presentation")
              throw new DOMException("The quota has been exceeded.", "QuotaExceededError");
            return setItem.call(this, key, value);
          };
        }
      }, failure);
      await page.goto(`${appUrl}${TRAINING}`);
      const ui = await openSettings(page);
      await ui.textSize.selectOption("1.5");
      await ui.flashes.check();
      expect(await rootFontSize(page)).toBe("24px");
      await expect(root(page)).toHaveAttribute("data-flashes", "reduced");
      await expect(ui.settings).toBeVisible();
      await ui.close.click();
      await page.getByRole("button", { name: "Pause", exact: true }).click();
      const attack = page.getByRole("button", { name: "Primary attack", exact: true });
      await attack.click();
      await page.getByRole("button", { name: "Step", exact: true }).click();
      await expect(attack).toHaveAttribute("data-phase", "windup");

      // Nothing was persisted: a reload starts from the defaults again.
      await page.reload();
      await expect(page.getByRole("button", { name: "Settings", exact: true })).toBeVisible();
      expect(await rootFontSize(page)).toBe("16px");
      expect(errors).toEqual([]);
    } finally {
      await context.close();
    }
  }
});
