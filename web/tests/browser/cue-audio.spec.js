import { expect, test } from "./fixtures.js";

// Combat sound (#67, 67c; ADR 0004) over the real WASM authority and the staged CC0 sound
// bundle: a cue starts a voice once the page has had a gesture, a replay never sounds again,
// and blocked or missing audio stays silent while the visual feedback is unchanged.

// Counts Web Audio voices the page starts; `block` removes Web Audio entirely.
async function countVoices(page, { block = false } = {}) {
  await page.addInitScript((blocked) => {
    globalThis.__arpgVoices = 0;
    if (blocked) {
      delete globalThis.AudioContext;
      delete globalThis.webkitAudioContext;
      return;
    }
    const start = AudioBufferSourceNode.prototype.start;
    AudioBufferSourceNode.prototype.start = function (...args) {
      globalThis.__arpgVoices += 1;
      return start.apply(this, args);
    };
  }, block);
  return () => page.evaluate(() => globalThis.__arpgVoices);
}

// The dummy training fixture, paused (the Pause click is the page's first gesture).
async function hitDummy(page, appUrl) {
  await page.goto(`${appUrl}?scenario=training&seed=42&fixture=dummy`);
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  await page.evaluate(() => document.activeElement?.blur());
  const feed = page.getByRole("list", { name: "Combat feedback" });
  const step = page.getByRole("button", { name: "Step", exact: true });
  // Clips decode in the background; give them a moment before the swing.
  await page.waitForTimeout(500);
  await page.keyboard.press("r");
  for (let tick = 0; tick < 60 && !(await feed.textContent()).includes("Hit "); tick += 1)
    await step.click();
  await expect(feed).toContainText("Hit ");
  await expect(page.locator(".damage-number")).toHaveText(["25"]);
  return feed;
}

async function soundNote(page) {
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Settings", exact: true });
  const note = settings.getByRole("region", { name: "Sound" }).locator(".settings-note");
  const text = await note.textContent();
  await settings.getByRole("button", { name: "Close", exact: true }).click();
  return text;
}

test("a hit starts a voice after a gesture, and a restored session does not replay it", async ({
  page,
  appUrl,
}) => {
  const voices = await countVoices(page);
  await hitDummy(page, appUrl);
  await expect.poll(voices).toBeGreaterThan(0);
  const heard = await voices();
  expect(await soundNote(page)).toContain("first click");

  // Restoring the session that contains the hit is a baseline, not a replay.
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Settings", exact: true });
  await settings.getByRole("button", { name: "Save game", exact: true }).click();
  await settings.getByRole("button", { name: "Load saved game", exact: true }).click();
  await expect(page.getByRole("region", { name: "Player status" })).toContainText(
    "Loaded local save",
  );
  await page.waitForTimeout(500);
  expect(await voices()).toBe(heard);
});

test("without Web Audio the game is silent and the feedback is unchanged", async ({
  page,
  appUrl,
}) => {
  const voices = await countVoices(page, { block: true });
  await hitDummy(page, appUrl);
  expect(await voices()).toBe(0);
  expect(await soundNote(page)).toBe("Sound is unavailable in this browser.");
});

test("missing sound files stay silent and the feedback is unchanged", async ({ page, appUrl }) => {
  const voices = await countVoices(page);
  await page.route("**/sounds/**/*.wav", (route) => route.fulfill({ status: 404, body: "" }));
  await hitDummy(page, appUrl);
  await page.waitForTimeout(300);
  expect(await voices()).toBe(0);
});
