// Acceptance contract for versioned presentation preferences (#61, first slice:
// accessibility presentation settings).
//
// Owner: `web/src/preferences.js` stays the single owner of local presentation storage. No
// second settings framework; the shared `settings`/`ui` packages are not pinned by ARPG yet.
// Presentation preferences are local browser state, never part of a character save or of the
// authoritative game, and never change rules, hitboxes, cooldowns or rewards.
//
// Storage shape (one versioned document under one key):
//
//   localStorage["arpg-presentation"] = JSON.stringify({
//     version: 2,
//     graphics: { shadows: boolean, pixelRatioLimit: 1 | 1.5 | 2 },
//     accessibility: {
//       motion: "system" | "reduced" | "full",   // "system" follows prefers-reduced-motion
//       reducedFlashes: boolean,                 // dims bright one-tick combat flashes
//       textScale: 1 | 1.25 | 1.5,               // root text/UI scale (styles are rem-based)
//     },
//   })
//
// Version 1 is the existing `arpg-graphics-v1` document `{ shadows, pixelRatioLimit }`, read
// with the existing all-or-nothing `loadGraphics` rules. Mouse aim (`arpg-mouse-aim-v1`) and
// the input profile (`arpg-input-profile-v2`, `input-profile.js`) are input preferences with their own owners and
// are not part of this document.
//
// API added to `web/src/preferences.js`:
//
//   PRESENTATION_KEY = "arpg-presentation"
//   PRESENTATION_VERSION = 2
//   MOTION_PREFERENCES = ["system", "reduced", "full"]
//   TEXT_SCALES = [1, 1.25, 1.5]
//   DEFAULT_PRESENTATION   // frozen: graphics { shadows: true, pixelRatioLimit: 2 },
//                          //   accessibility { motion: "system", reducedFlashes: false,
//                          //   textScale: 1 }
//   normalizePresentation(value) => preferences
//       Any value to valid preferences. Each field falls back to its default on its own;
//       unknown fields are dropped. Always returns fresh objects (never DEFAULT_PRESENTATION
//       itself or one of its nested objects).
//   loadPresentation(storage?) => preferences
//       Reads PRESENTATION_KEY. A version-2 object is normalized field by field. A missing,
//       unparseable, non-object or other-version document counts as absent: the legacy
//       `arpg-graphics-v1` value is migrated (graphics only, accessibility defaults), else
//       defaults. Read-only: it never writes or removes any key, so migration is idempotent
//       and the legacy key stays available for rollback. Never throws.
//   persistPresentation(preferences, storage?) => boolean
//       Writes `{ version: 2, ...normalizePresentation(preferences) }` to PRESENTATION_KEY.
//       Returns false (never throws) when storage access is denied, setItem throws, or the
//       quota is exceeded. Never touches the legacy graphics key or input preference keys.
//
// `storage` defaults to `globalThis.localStorage`, whose getter itself may throw.
import { afterEach, expect, test } from "bun:test";
import {
  DEFAULT_PRESENTATION,
  GRAPHICS_KEY,
  MOTION_PREFERENCES,
  MOUSE_AIM_KEY,
  PRESENTATION_KEY,
  PRESENTATION_VERSION,
  TEXT_SCALES,
  loadGraphics,
  loadPresentation,
  normalizePresentation,
  persistPresentation,
} from "./preferences.js";
import { PROFILE_KEY } from "./input-profile.js";

const DEFAULTS = {
  graphics: { shadows: true, pixelRatioLimit: 2 },
  accessibility: { motion: "system", reducedFlashes: false, textScale: 1 },
};

// An in-memory Storage double that records every write and removal.
function memoryStorage(initial = {}) {
  const values = new Map(Object.entries(initial));
  const writes = [];
  const removals = [];
  return {
    values,
    writes,
    removals,
    getItem: (key) => (values.has(key) ? values.get(key) : null),
    setItem(key, value) {
      writes.push([key, String(value)]);
      values.set(key, String(value));
    },
    removeItem(key) {
      removals.push(key);
      values.delete(key);
    },
  };
}

const stored = (document) => ({ [PRESENTATION_KEY]: JSON.stringify(document) });

const originalLocalStorage = Object.getOwnPropertyDescriptor(globalThis, "localStorage");
afterEach(() => {
  if (originalLocalStorage) Object.defineProperty(globalThis, "localStorage", originalLocalStorage);
  else delete globalThis.localStorage;
});

test("the presentation document is version 2 under its own key with documented choices", () => {
  expect(PRESENTATION_KEY).toBe("arpg-presentation");
  expect(PRESENTATION_VERSION).toBe(2);
  expect([...MOTION_PREFERENCES]).toEqual(["system", "reduced", "full"]);
  expect([...TEXT_SCALES]).toEqual([1, 1.25, 1.5]);
  expect(DEFAULT_PRESENTATION).toEqual(DEFAULTS);
  expect(Object.isFrozen(DEFAULT_PRESENTATION)).toBe(true);
  // Input preferences keep their own keys and owners.
  expect(new Set([PRESENTATION_KEY, GRAPHICS_KEY, MOUSE_AIM_KEY, PROFILE_KEY]).size).toBe(4);
});

test("no stored document yields defaults without writing anything", () => {
  const storage = memoryStorage();
  expect(loadPresentation(storage)).toEqual(DEFAULTS);
  expect(storage.writes).toEqual([]);
  expect(storage.removals).toEqual([]);
});

test("a valid version 2 document round-trips exactly", () => {
  const preferences = {
    graphics: { shadows: false, pixelRatioLimit: 1.5 },
    accessibility: { motion: "reduced", reducedFlashes: true, textScale: 1.25 },
  };
  const storage = memoryStorage();
  expect(persistPresentation(preferences, storage)).toBe(true);
  expect(storage.writes).toHaveLength(1);
  const [key, text] = storage.writes[0];
  expect(key).toBe(PRESENTATION_KEY);
  expect(JSON.parse(text)).toEqual({ version: 2, ...preferences });
  expect(loadPresentation(storage)).toEqual(preferences);
});

test("every documented choice is accepted", () => {
  for (const motion of ["system", "reduced", "full"])
    for (const textScale of [1, 1.25, 1.5])
      for (const reducedFlashes of [false, true])
        for (const pixelRatioLimit of [1, 1.5, 2]) {
          const preferences = {
            graphics: { shadows: false, pixelRatioLimit },
            accessibility: { motion, reducedFlashes, textScale },
          };
          expect(loadPresentation(memoryStorage(stored({ version: 2, ...preferences })))).toEqual(
            preferences,
          );
        }
});

test("the legacy graphics document migrates to version 2 and keeps its values", () => {
  const storage = memoryStorage({
    [GRAPHICS_KEY]: JSON.stringify({ shadows: false, pixelRatioLimit: 1 }),
  });
  const migrated = loadPresentation(storage);
  expect(migrated).toEqual({
    graphics: { shadows: false, pixelRatioLimit: 1 },
    accessibility: DEFAULTS.accessibility,
  });
  // Loading is read-only: replaying the migration is idempotent and the legacy value stays
  // available for rollback to an older build.
  expect(loadPresentation(storage)).toEqual(migrated);
  expect(storage.writes).toEqual([]);
  expect(storage.removals).toEqual([]);
  expect(JSON.parse(storage.getItem(GRAPHICS_KEY))).toEqual({ shadows: false, pixelRatioLimit: 1 });

  // The first persisted change writes version 2; the legacy key is left untouched.
  expect(
    persistPresentation(
      { ...migrated, accessibility: { ...migrated.accessibility, textScale: 1.5 } },
      storage,
    ),
  ).toBe(true);
  expect(storage.writes.map(([key]) => key)).toEqual([PRESENTATION_KEY]);
  expect(JSON.parse(storage.getItem(PRESENTATION_KEY))).toEqual({
    version: 2,
    graphics: { shadows: false, pixelRatioLimit: 1 },
    accessibility: { motion: "system", reducedFlashes: false, textScale: 1.5 },
  });
  expect(JSON.parse(storage.getItem(GRAPHICS_KEY))).toEqual({ shadows: false, pixelRatioLimit: 1 });
});

test("legacy migration applies the existing loadGraphics rules", () => {
  for (const legacy of [
    { shadows: false, pixelRatioLimit: 3 },
    { shadows: "no", pixelRatioLimit: 1 },
    "garbage",
  ]) {
    const storage = memoryStorage({ [GRAPHICS_KEY]: JSON.stringify(legacy) });
    expect(loadPresentation(storage).graphics).toEqual(loadGraphics(storage));
    expect(loadPresentation(storage)).toEqual(DEFAULTS);
  }
  const corrupt = memoryStorage({ [GRAPHICS_KEY]: "{not json" });
  expect(loadPresentation(corrupt)).toEqual(DEFAULTS);
});

test("a version 2 document wins over the legacy graphics document", () => {
  const storage = memoryStorage({
    [GRAPHICS_KEY]: JSON.stringify({ shadows: false, pixelRatioLimit: 1 }),
    ...stored({
      version: 2,
      graphics: { shadows: true, pixelRatioLimit: 1.5 },
      accessibility: { motion: "full", reducedFlashes: false, textScale: 1 },
    }),
  });
  expect(loadPresentation(storage).graphics).toEqual({ shadows: true, pixelRatioLimit: 1.5 });
});

test("an uninterpretable version 2 document counts as absent and falls back to legacy", () => {
  const legacy = { [GRAPHICS_KEY]: JSON.stringify({ shadows: false, pixelRatioLimit: 1.5 }) };
  const expected = {
    graphics: { shadows: false, pixelRatioLimit: 1.5 },
    accessibility: DEFAULTS.accessibility,
  };
  for (const text of [
    "{not json",
    "null",
    "42",
    '"text"',
    "[]",
    JSON.stringify({ graphics: DEFAULTS.graphics }),
    JSON.stringify({ version: 1, ...DEFAULTS }),
    JSON.stringify({ version: 3, ...DEFAULTS }),
    JSON.stringify({ version: "2", ...DEFAULTS }),
  ]) {
    const storage = memoryStorage({ ...legacy, [PRESENTATION_KEY]: text });
    expect(loadPresentation(storage)).toEqual(expected);
    expect(storage.writes).toEqual([]);
    expect(loadPresentation(memoryStorage({ [PRESENTATION_KEY]: text }))).toEqual(DEFAULTS);
  }
});

test("invalid fields fall back one by one and unknown fields are dropped", () => {
  const storage = memoryStorage(
    stored({
      version: 2,
      graphics: { shadows: false, pixelRatioLimit: 3, extra: true },
      accessibility: { motion: "slow", reducedFlashes: true, textScale: 1.25, contrast: "high" },
      untrusted: { gameplay: "godMode" },
    }),
  );
  expect(loadPresentation(storage)).toEqual({
    graphics: { shadows: false, pixelRatioLimit: 2 },
    accessibility: { motion: "system", reducedFlashes: true, textScale: 1.25 },
  });

  for (const accessibility of [
    { motion: "REDUCED", reducedFlashes: "true", textScale: "1.5" },
    { motion: null, reducedFlashes: 1, textScale: 2 },
    { motion: 0, reducedFlashes: null, textScale: 0.5 },
    { motion: ["reduced"], reducedFlashes: {}, textScale: Number.NaN },
    { textScale: Infinity },
    null,
    "reduced",
  ]) {
    expect(
      loadPresentation(
        memoryStorage(stored({ version: 2, graphics: DEFAULTS.graphics, accessibility })),
      ),
    ).toEqual(DEFAULTS);
  }
  expect(
    loadPresentation(
      memoryStorage(
        stored({ version: 2, graphics: "high", accessibility: DEFAULTS.accessibility }),
      ),
    ),
  ).toEqual(DEFAULTS);
});

test("normalization returns fresh objects and never exposes the frozen defaults", () => {
  const first = normalizePresentation(undefined);
  expect(first).toEqual(DEFAULTS);
  expect(first).not.toBe(DEFAULT_PRESENTATION);
  expect(first.graphics).not.toBe(DEFAULT_PRESENTATION.graphics);
  expect(first.accessibility).not.toBe(DEFAULT_PRESENTATION.accessibility);
  first.accessibility.textScale = 1.5;
  first.graphics.shadows = false;
  expect(DEFAULT_PRESENTATION).toEqual(DEFAULTS);
  expect(normalizePresentation(null)).toEqual(DEFAULTS);
  expect(
    normalizePresentation({
      graphics: { shadows: false },
      accessibility: { reducedFlashes: true },
    }),
  ).toEqual({
    graphics: { shadows: false, pixelRatioLimit: 2 },
    accessibility: { motion: "system", reducedFlashes: true, textScale: 1 },
  });
});

test("persisting normalizes, so invalid in-memory values never reach storage", () => {
  const storage = memoryStorage();
  expect(
    persistPresentation(
      { graphics: { shadows: 1 }, accessibility: { motion: "reduced", textScale: 9 }, rules: {} },
      storage,
    ),
  ).toBe(true);
  expect(JSON.parse(storage.getItem(PRESENTATION_KEY))).toEqual({
    version: 2,
    graphics: { shadows: true, pixelRatioLimit: 2 },
    accessibility: { motion: "reduced", reducedFlashes: false, textScale: 1 },
  });
});

test("storage denial and quota errors never throw and keep the previous value", () => {
  const previous = {
    version: 2,
    graphics: { shadows: false, pixelRatioLimit: 1 },
    accessibility: { motion: "full", reducedFlashes: false, textScale: 1 },
  };
  const quota = {
    ...memoryStorage(stored(previous)),
    setItem() {
      throw new DOMException("The quota has been exceeded.", "QuotaExceededError");
    },
  };
  const next = {
    graphics: DEFAULTS.graphics,
    accessibility: { motion: "reduced", reducedFlashes: true, textScale: 1.5 },
  };
  expect(persistPresentation(next, quota)).toBe(false);
  expect(loadPresentation(quota)).toEqual({
    graphics: previous.graphics,
    accessibility: previous.accessibility,
  });

  const blocked = {
    getItem() {
      throw new DOMException("Blocked", "SecurityError");
    },
    setItem() {
      throw new DOMException("Blocked", "SecurityError");
    },
  };
  expect(loadPresentation(blocked)).toEqual(DEFAULTS);
  expect(persistPresentation(next, blocked)).toBe(false);
});

test("a throwing localStorage getter is handled for the default storage", () => {
  Object.defineProperty(globalThis, "localStorage", {
    configurable: true,
    get() {
      throw new DOMException("Blocked", "SecurityError");
    },
  });
  expect(loadPresentation()).toEqual(DEFAULTS);
  expect(persistPresentation(DEFAULTS)).toBe(false);
});

test("the default storage is localStorage", () => {
  const storage = memoryStorage();
  Object.defineProperty(globalThis, "localStorage", { configurable: true, value: storage });
  const preferences = {
    graphics: DEFAULTS.graphics,
    accessibility: { motion: "full", reducedFlashes: true, textScale: 1.25 },
  };
  expect(persistPresentation(preferences)).toBe(true);
  expect(loadPresentation()).toEqual(preferences);
  expect(storage.writes.map(([key]) => key)).toEqual([PRESENTATION_KEY]);
});
