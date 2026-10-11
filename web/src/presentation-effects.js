// Maps local presentation preferences to the effects they govern (#61). Pure: no DOM access
// at import. Presentation only; nothing here reaches the authority or a character save.
//
// - Motion: the scene frame's stagger hop and the UI's CSS transitions/hover transforms.
// - Flashes: one-tick combat states (an active swing, a blocked shield, a monster's active
//   strike and telegraph, a stagger) normally switch to much brighter colours for one frame.
//   The reduced palette keeps every state distinguishable (RGB distance >= 40) while no
//   one-tick state brightens by more than 0.1 relative luminance over the state it follows,
//   the WCAG 2.3.1 general-flash threshold.
// - Text size: every UI size is rem-based, so the root font size scales the whole UI.

import { normalizePresentation } from "./preferences.js";

const STAGGER_LIFT = 0.08;

const MONSTER_BEHAVIOR = Object.freeze({
  pursuing: "#a3483b",
  retreating: "#8a5a7a",
  holding: "#8f4037",
  searching: "#8a6a3c",
  returning: "#5f5866",
  idle: "#73433d",
});

// Exactly the colours drawn before #61, so the default changes nothing.
const FULL_PALETTE = Object.freeze({
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
  monsterBehavior: MONSTER_BEHAVIOR,
  telegraphWindup: "#824239",
  telegraphActive: "#f06a4e",
});

// Only the one-tick states change: each moves to a hue near its full colour at a luminance
// at most ~0.09 above its predecessor (the darkest monster behaviour for a stagger).
const REDUCED_PALETTE = Object.freeze({
  ...FULL_PALETTE,
  playerActive: "#d4c23c",
  shieldBlocked: "#6aa6ee",
  monsterActive: "#f2644a",
  monsterStagger: "#b0600c",
  telegraphActive: "#c44636",
});

export function combatPalette(reducedFlashes) {
  return reducedFlashes ? REDUCED_PALETTE : FULL_PALETTE;
}

export function resolvePresentationEffects(preferences, { prefersReducedMotion = false } = {}) {
  const { accessibility } = normalizePresentation(preferences);
  const reducedMotion =
    accessibility.motion === "reduced" ||
    (accessibility.motion === "system" && prefersReducedMotion === true);
  return {
    reducedMotion,
    reducedFlashes: accessibility.reducedFlashes,
    textScale: accessibility.textScale,
    staggerLift: reducedMotion ? 0 : STAGGER_LIFT,
    palette: combatPalette(accessibility.reducedFlashes),
  };
}

export const FULL_EFFECTS = Object.freeze(resolvePresentationEffects(undefined));

// Applies the document-wide effects to the root element (`document.documentElement`).
export function applyDocumentPresentation(root, effects) {
  root.style.fontSize = `${effects.textScale * 100}%`;
  root.dataset.motion = effects.reducedMotion ? "reduced" : "full";
  root.dataset.flashes = effects.reducedFlashes ? "reduced" : "full";
}
