export const SETUP_URL_KEY = "arpg-setup-service-url-v1";
export const DEDICATED_URL_KEY = "arpg-dedicated-url-v1";
export const GRAPHICS_KEY = "arpg-graphics-v1";
export const MOUSE_AIM_KEY = "arpg-mouse-aim-v1";

export function readStoredValue(key, fallback, storage) {
  try {
    return (storage ?? globalThis.localStorage)?.getItem(key) ?? fallback;
  } catch {
    // Browser policy can block both storage access and reads; session defaults remain usable.
    return fallback;
  }
}

export function persistStoredValue(key, value, storage) {
  try {
    (storage ?? globalThis.localStorage).setItem(key, value);
    return true;
  } catch {
    // Persistence is optional; callers retain and apply the in-memory preference.
    return false;
  }
}

export function loadGraphics(storage) {
  try {
    const parsed = JSON.parse(readStoredValue(GRAPHICS_KEY, "null", storage));
    if (
      parsed &&
      typeof parsed.shadows === "boolean" &&
      [1, 1.5, 2].includes(parsed.pixelRatioLimit)
    ) {
      return { shadows: parsed.shadows, pixelRatioLimit: parsed.pixelRatioLimit };
    }
  } catch {
    // Invalid local presentation settings fall back to known-safe defaults.
  }
  return { shadows: true, pixelRatioLimit: 2 };
}

// Mouse facing is the desktop default; an explicit opt-out keeps legacy controls available.
export function loadMouseAim(storage) {
  return readStoredValue(MOUSE_AIM_KEY, "on", storage) !== "off";
}

// Versioned local presentation preferences (#61): graphics and accessibility in one
// document. Local browser state only; never part of a character save or the authority.
export const PRESENTATION_KEY = "arpg-presentation";
export const PRESENTATION_VERSION = 2;
export const MOTION_PREFERENCES = Object.freeze(["system", "reduced", "full"]);
export const TEXT_SCALES = Object.freeze([1, 1.25, 1.5]);
const PIXEL_RATIO_LIMITS = [1, 1.5, 2];

export const DEFAULT_PRESENTATION = Object.freeze({
  graphics: Object.freeze({ shadows: true, pixelRatioLimit: 2 }),
  accessibility: Object.freeze({ motion: "system", reducedFlashes: false, textScale: 1 }),
});

const isRecord = (value) => value !== null && typeof value === "object" && !Array.isArray(value);
const pick = (value, valid, fallback) => (valid ? value : fallback);

// Any value to valid preferences; each field falls back to its default on its own.
export function normalizePresentation(value) {
  const graphics = isRecord(value?.graphics) ? value.graphics : {};
  const accessibility = isRecord(value?.accessibility) ? value.accessibility : {};
  const defaults = DEFAULT_PRESENTATION;
  return {
    graphics: {
      shadows: pick(
        graphics.shadows,
        typeof graphics.shadows === "boolean",
        defaults.graphics.shadows,
      ),
      pixelRatioLimit: pick(
        graphics.pixelRatioLimit,
        PIXEL_RATIO_LIMITS.includes(graphics.pixelRatioLimit),
        defaults.graphics.pixelRatioLimit,
      ),
    },
    accessibility: {
      motion: pick(
        accessibility.motion,
        MOTION_PREFERENCES.includes(accessibility.motion),
        defaults.accessibility.motion,
      ),
      reducedFlashes: pick(
        accessibility.reducedFlashes,
        typeof accessibility.reducedFlashes === "boolean",
        defaults.accessibility.reducedFlashes,
      ),
      textScale: pick(
        accessibility.textScale,
        TEXT_SCALES.includes(accessibility.textScale),
        defaults.accessibility.textScale,
      ),
    },
  };
}

// Reads the version 2 document, else migrates the legacy graphics document in memory.
// Read-only: the legacy key stays for rollback and replaying the migration is idempotent.
export function loadPresentation(storage) {
  let parsed = null;
  try {
    parsed = JSON.parse(readStoredValue(PRESENTATION_KEY, "null", storage));
  } catch {
    // An unreadable document counts as absent.
  }
  if (isRecord(parsed) && parsed.version === PRESENTATION_VERSION)
    return normalizePresentation(parsed);
  return normalizePresentation({ graphics: loadGraphics(storage) });
}

// Never throws: denied storage or a full quota returns false and the caller keeps the
// in-memory preferences for the session. Never touches the legacy or input keys.
export function persistPresentation(preferences, storage) {
  return persistStoredValue(
    PRESENTATION_KEY,
    JSON.stringify({ version: PRESENTATION_VERSION, ...normalizePresentation(preferences) }),
    storage,
  );
}
