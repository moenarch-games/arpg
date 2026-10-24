// Runtime acceptance (#61, first slice): accessibility presentation preferences never reach
// the authority. Two runs of the same seeded workbench scenario receive the same inputs on the
// same ticks through the real WASM authority. One run starts with every accessibility
// preference changed (stored `arpg-presentation` version 2 document) and changes them again
// mid-run through the Settings dialog. Their authoritative evidence must be identical:
// the recorded reproduction (accepted commands and ticks), the exported save's core state,
// the strike/event timeline and the training diagnostics. The save file carries no
// presentation preference, and the presentation document carries no game state.
import { readFile } from "node:fs/promises";

import { expect, test } from "./fixtures.js";

const KEY = "arpg-presentation";
const SCENARIO = "?scenario=training&seed=42&fixture=enemy";
const CLOCK_START = Date.UTC(2026, 0, 1);

async function download(page, trigger) {
  const pending = page.waitForEvent("download");
  await trigger();
  return JSON.parse(await readFile(await (await pending).path(), "utf8"));
}

async function changeSettings(page, { motion, reducedFlashes, textScale, graphics }) {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Settings", exact: true });
  if (graphics) {
    // Quality changes recreate the renderer mid-play (#61, 61a); the authority must not.
    await settings.getByLabel("Enable shadows").setChecked(graphics.shadows);
    await settings.getByLabel("Pixel ratio limit").selectOption(String(graphics.pixelRatioLimit));
  }
  const accessibility = settings.getByRole("region", { name: "Accessibility", exact: true });
  await accessibility.getByLabel("Motion effects", { exact: true }).selectOption(motion);
  await accessibility
    .getByLabel("Reduce bright flashes", { exact: true })
    .setChecked(reducedFlashes);
  await accessibility.getByLabel("Text size", { exact: true }).selectOption(String(textScale));
  await settings.getByRole("button", { name: "Close", exact: true }).click();
  await expect(settings).not.toBeVisible();
}

async function run(browser, appUrl, { stored = null, midRun = null } = {}) {
  const context = await browser.newContext();
  const page = await context.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  try {
    if (stored)
      await page.addInitScript(
        ([key, value]) => localStorage.setItem(key, value),
        [KEY, JSON.stringify(stored)],
      );
    // Freeze page time until training is paused, so both runs pause before the authority's
    // first tick instead of after however many real-time ticks the page load took.
    await page.clock.install({ time: CLOCK_START });
    await page.clock.pauseAt(CLOCK_START + 1_000);
    await page.goto(`${appUrl}${SCENARIO}`);
    await page.getByRole("button", { name: "Pause", exact: true }).click();
    await page.clock.resume();
    if (stored) {
      // The preferences really are in effect for this run.
      await expect(page.locator("html")).toHaveAttribute("data-flashes", "reduced");
      await expect(page.locator("html")).toHaveAttribute("data-motion", "reduced");
    }
    const step = page.getByRole("button", { name: "Step", exact: true });
    const attack = page.getByRole("button", { name: "Primary attack", exact: true });
    for (let tick = 0; tick < 60; tick += 1) {
      if (tick === 0 || tick === 30) await attack.click();
      if (tick === 15 && midRun) await changeSettings(page, midRun);
      await step.click();
    }

    const timeline = await page.getByRole("region", { name: "Event timeline" }).textContent();
    const diagnostics = await page.locator(".training-diagnostics").textContent();
    const reproduction = await download(page, () =>
      page.getByRole("button", { name: "Export reproduction", exact: true }).click(),
    );
    await page.getByRole("button", { name: "Settings", exact: true }).click();
    const save = await download(page, () =>
      page
        .getByRole("dialog", { name: "Settings", exact: true })
        .getByRole("button", { name: "Export save", exact: true })
        .click(),
    );
    const presentation = await page.evaluate(
      (key) => JSON.parse(localStorage.getItem(key) ?? "null"),
      KEY,
    );
    expect(errors).toEqual([]);
    return { timeline, diagnostics, reproduction, save, presentation };
  } finally {
    await context.close();
  }
}

test("accessibility and quality preferences do not change the authoritative trace", async ({
  browser,
  appUrl,
}) => {
  const baseline = await run(browser, appUrl);
  const adjusted = await run(browser, appUrl, {
    stored: {
      version: 2,
      graphics: { shadows: true, pixelRatioLimit: 2 },
      accessibility: { motion: "reduced", reducedFlashes: true, textScale: 1.5 },
    },
    midRun: {
      motion: "full",
      reducedFlashes: false,
      textScale: 1.25,
      graphics: { shadows: false, pixelRatioLimit: 1 },
    },
  });

  // The scenario really exercised combat during the compared window.
  expect(baseline.reproduction.commands.map((entry) => entry.command)).toContainEqual(
    expect.objectContaining({ type: "primaryAttack" }),
  );
  expect(baseline.reproduction.ticks).toBeGreaterThanOrEqual(60);

  expect(adjusted.reproduction).toEqual(baseline.reproduction);
  expect(adjusted.save.coreState).toEqual(baseline.save.coreState);
  expect(adjusted.timeline).toBe(baseline.timeline);
  expect(adjusted.diagnostics).toBe(baseline.diagnostics);

  // Presentation preferences and character saves stay separate.
  const saveText = JSON.stringify(adjusted.save);
  for (const field of ["reducedFlashes", "textScale", "accessibility", "pixelRatioLimit"])
    expect(saveText).not.toContain(field);
  expect(adjusted.presentation).toEqual({
    version: 2,
    graphics: { shadows: false, pixelRatioLimit: 1 },
    accessibility: { motion: "full", reducedFlashes: false, textScale: 1.25 },
    camera: { zoom: 1 },
    audio: { master: 1, effects: 1, ui: 1, muted: false },
  });
});
