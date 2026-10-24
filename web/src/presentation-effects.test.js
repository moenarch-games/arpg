// Acceptance contract for how accessibility presentation preferences map to their effects
// (#61, first slice). See `presentation-preferences.test.js` for the stored shape.
//
// Effects that exist on main and that these settings govern (preflight):
// - Motion: a staggered monster hops 0.08 m up in the scene frame; character cards slide on
//   hover through a CSS transform transition. There is no camera/screen shake, so no shake
//   setting is exposed.
// - Flashes: one-tick combat states switch to much brighter colours for a single drawn frame
//   (a player's active swing, a blocked shield, a monster's active strike and its active
//   telegraph ring, a staggered monster). The active-cue tracker guarantees that frame.
// - Text/UI scale: every UI size is rem-based, so the root font size scales the whole HUD,
//   the settings dialog and the touch controls.
//
// API of the new module `web/src/presentation-effects.js` (pure; no DOM access at import):
//
//   resolvePresentationEffects(preferences, { prefersReducedMotion = false } = {}) => {
//     reducedMotion: boolean,      // motion "reduced", or "system" with prefersReducedMotion
//     reducedFlashes: boolean,
//     textScale: 1 | 1.25 | 1.5,
//     staggerLift: number,         // 0.08 (metres) normally, 0 with reduced motion
//     palette,                     // combatPalette(reducedFlashes)
//   }
//   Invalid preferences are normalized first (preferences.js normalizePresentation).
//
//   FULL_EFFECTS                   // frozen resolvePresentationEffects(DEFAULT_PRESENTATION)
//
//   combatPalette(reducedFlashes) => frozen {
//     playerWindup, playerActive, playerRecovery, playerHurt, playerGuardBroken, playerPeer,
//     shieldRising, shieldRaised, shieldBlocked,
//     monsterWindup, monsterActive, monsterRecovery, monsterStagger, monsterDefault,
//     monsterBehavior: { pursuing, retreating, holding, searching, returning, idle },
//     telegraphWindup, telegraphActive,
//   }                               // "#rrggbb" strings
//   The full palette is exactly today's colours (no visual change by default). The reduced
//   palette keeps every state change visible (RGB distance >= 40) but no one-tick state may
//   brighten by more than 0.1 relative luminance over the state it follows: the WCAG 2.3.1
//   general-flash threshold, so none of these transitions is a flash.
//
//   applyDocumentPresentation(root, effects)
//     root.style.fontSize = `${textScale * 100}%`  ("100%", "125%", "150%")
//     root.dataset.motion = "reduced" | "full"     (styles.css disables transitions and
//                                                   hover transforms under "reduced")
//     root.dataset.flashes = "reduced" | "full"
import { expect, test } from "bun:test";
import {
  FULL_EFFECTS,
  applyDocumentPresentation,
  combatPalette,
  resolvePresentationEffects,
} from "./presentation-effects.js";

const preferences = (accessibility = {}) => ({
  graphics: { shadows: true, pixelRatioLimit: 2 },
  accessibility: {
    motion: "system",
    reducedFlashes: false,
    combatEffects: true,
    textScale: 1,
    ...accessibility,
  },
});

function luminance(hex) {
  expect(hex).toMatch(/^#[0-9a-f]{6}$/i);
  const [r, g, b] = [1, 3, 5].map((index) => {
    const channel = Number.parseInt(hex.slice(index, index + 2), 16) / 255;
    return channel <= 0.04045 ? channel / 12.92 : ((channel + 0.055) / 1.055) ** 2.4;
  });
  return 0.2126 * r + 0.7152 * g + 0.0722 * b;
}

function distance(left, right) {
  const rgb = (hex) => [1, 3, 5].map((index) => Number.parseInt(hex.slice(index, index + 2), 16));
  const [a, b] = [rgb(left), rgb(right)];
  return Math.hypot(a[0] - b[0], a[1] - b[1], a[2] - b[2]);
}

// [state it follows, one-tick state] for every bright one-tick transition on main.
function flashTransitions(palette) {
  return [
    ["playerWindup", "playerActive"],
    ["shieldRaised", "shieldBlocked"],
    ["monsterWindup", "monsterActive"],
    ["telegraphWindup", "telegraphActive"],
    ["monsterDefault", "monsterStagger"],
    ...Object.keys(palette.monsterBehavior).map((behavior) => [
      `monsterBehavior.${behavior}`,
      "monsterStagger",
    ]),
  ];
}

const color = (palette, key) =>
  key.startsWith("monsterBehavior.") ? palette.monsterBehavior[key.slice(16)] : palette[key];

test("defaults keep today's motion, flashes and text size", () => {
  const effects = resolvePresentationEffects(preferences());
  expect(effects).toMatchObject({
    reducedMotion: false,
    reducedFlashes: false,
    textScale: 1,
    staggerLift: 0.08,
  });
  expect(effects.palette).toEqual(combatPalette(false));
  expect(FULL_EFFECTS).toEqual(effects);
  expect(Object.isFrozen(FULL_EFFECTS)).toBe(true);
});

test("motion follows the system only when the preference says so", () => {
  const cases = [
    ["system", false, false],
    ["system", true, true],
    ["reduced", false, true],
    ["reduced", true, true],
    ["full", false, false],
    ["full", true, false],
  ];
  for (const [motion, prefersReducedMotion, reducedMotion] of cases) {
    const effects = resolvePresentationEffects(preferences({ motion }), { prefersReducedMotion });
    expect(effects.reducedMotion).toBe(reducedMotion);
    expect(effects.staggerLift).toBe(reducedMotion ? 0 : 0.08);
  }
  expect(resolvePresentationEffects(preferences({ motion: "system" })).reducedMotion).toBe(false);
});

test("reduced flashes and text scale map independently of motion", () => {
  const effects = resolvePresentationEffects(
    preferences({ motion: "full", reducedFlashes: true, combatEffects: true, textScale: 1.5 }),
    { prefersReducedMotion: true },
  );
  expect(effects).toMatchObject({
    reducedMotion: false,
    reducedFlashes: true,
    textScale: 1.5,
    staggerLift: 0.08,
  });
  expect(effects.palette).toEqual(combatPalette(true));
  for (const textScale of [1, 1.25, 1.5])
    expect(resolvePresentationEffects(preferences({ textScale })).textScale).toBe(textScale);
});

test("invalid preferences resolve to the default effects", () => {
  for (const value of [
    null,
    undefined,
    {},
    { accessibility: { motion: "slow", reducedFlashes: "yes", textScale: 3 } },
  ])
    expect(resolvePresentationEffects(value)).toEqual(FULL_EFFECTS);
});

test("the full palette is exactly the colours drawn today", () => {
  expect(combatPalette(false)).toEqual({
    playerWindup: "#e3a45d",
    playerActive: "#fff0a3",
    playerRecovery: "#9d8060",
    playerHurt: "#e05a4f",
    playerGuardBroken: "#9b6bd6",
    playerPeer: "#6f91b6",
    shieldRising: "#55687d",
    shieldRaised: "#8fb3d9",
    shieldBlocked: "#e8f4ff",
    monsterWindup: "#c85b46",
    monsterActive: "#ff6a4f",
    monsterRecovery: "#6e3934",
    monsterStagger: "#e8a25d",
    monsterDefault: "#8f4037",
    monsterBehavior: {
      pursuing: "#a3483b",
      retreating: "#8a5a7a",
      holding: "#8f4037",
      searching: "#8a6a3c",
      returning: "#5f5866",
      idle: "#73433d",
    },
    telegraphWindup: "#824239",
    telegraphActive: "#f06a4e",
  });
  expect(Object.isFrozen(combatPalette(false))).toBe(true);
  expect(Object.isFrozen(combatPalette(true))).toBe(true);
});

test("today's palette really contains bright one-tick flashes", () => {
  // Guards the contract itself: the setting governs a real effect.
  const full = combatPalette(false);
  const flashes = flashTransitions(full).filter(
    ([before, after]) => luminance(color(full, after)) - luminance(color(full, before)) > 0.1,
  );
  expect(flashes.length).toBeGreaterThanOrEqual(4);
});

test("reduced flashes keep each state visible without a bright flash", () => {
  const reduced = combatPalette(true);
  expect(Object.keys(reduced).sort()).toEqual(Object.keys(combatPalette(false)).sort());
  expect(Object.keys(reduced.monsterBehavior).sort()).toEqual(
    Object.keys(combatPalette(false).monsterBehavior).sort(),
  );
  for (const [before, after] of flashTransitions(reduced)) {
    const from = color(reduced, before);
    const to = color(reduced, after);
    expect({
      transition: `${before} -> ${after}`,
      rise: luminance(to) - luminance(from) <= 0.1,
    }).toEqual({ transition: `${before} -> ${after}`, rise: true });
    expect({ transition: `${before} -> ${after}`, visible: distance(from, to) >= 40 }).toEqual({
      transition: `${before} -> ${after}`,
      visible: true,
    });
  }
  // Windup, active and recovery remain three distinguishable states.
  expect(distance(reduced.playerActive, reduced.playerRecovery)).toBeGreaterThanOrEqual(40);
  expect(distance(reduced.monsterActive, reduced.monsterRecovery)).toBeGreaterThanOrEqual(40);
});

test("the document root receives scale, motion and flash state", () => {
  const root = { style: {}, dataset: {} };
  applyDocumentPresentation(
    root,
    resolvePresentationEffects(
      preferences({ motion: "reduced", reducedFlashes: true, combatEffects: true, textScale: 1.5 }),
    ),
  );
  expect(root.style.fontSize).toBe("150%");
  expect(root.dataset).toEqual({ motion: "reduced", flashes: "reduced" });

  applyDocumentPresentation(root, resolvePresentationEffects(preferences({ textScale: 1.25 })));
  expect(root.style.fontSize).toBe("125%");
  expect(root.dataset).toEqual({ motion: "full", flashes: "full" });

  applyDocumentPresentation(root, FULL_EFFECTS);
  expect(root.style.fontSize).toBe("100%");
});
