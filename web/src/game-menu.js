// In-world game menu navigation (#60). One overlay layer is open at a time:
//   null (play) → "menu" (Resume / Settings / Leave) → "settings" | "confirmLeave".
// Escape and Back step out of the innermost layer; from play, Escape opens the menu.
// Settings opened from the header shortcut returns straight to play. `focus` names the menu
// button that takes focus when the menu (re)opens, so closing a layer returns to its opener.

export const PLAY = Object.freeze({ layer: null, settingsFrom: null, focus: null });

export function openMenu(focus = "resume") {
  return { layer: "menu", settingsFrom: null, focus };
}

export function openSettings(from) {
  return { layer: "settings", settingsFrom: from === "menu" ? "menu" : "play", focus: null };
}

export function openLeaveConfirmation() {
  return { layer: "confirmLeave", settingsFrom: null, focus: null };
}

// The Escape key: closes the innermost layer, or opens the menu during play.
export function escapeFrom(state) {
  if (state.layer === null) return openMenu();
  return closeLayer(state);
}

// Closing or cancelling the innermost layer.
export function closeLayer(state) {
  if (state.layer === "confirmLeave") return openMenu("leave");
  if (state.layer === "settings" && state.settingsFrom === "menu") return openMenu("settings");
  return PLAY;
}

// Leaving a run discards unsaved progress, so it always asks first until run checkpoints
// exist (#172). Training is disposable and never asks.
export function leaveNeedsConfirmation({ training }) {
  return !training;
}

// What leaving means for this session, shown in the confirmation.
export function leaveConsequence(mode) {
  if (mode === "host") return "Leaving ends the hosted session for everyone in it.";
  if (mode === "guest" || mode === "dedicated") return "You will disconnect from this session.";
  return "Progress since your last save is lost.";
}

// Only a local authority pauses for the menu: a hosted world is shared with guests, and a
// guest or dedicated client does not own the world it sees.
export function menuPausesWorld(mode) {
  return mode === "local";
}

// Whether the open menu suspends the world, as the menu tells the player.
export function menuWorldNotice(pausesWorld) {
  return pausesWorld ? "Game paused" : "World continues · this menu does not pause a shared game";
}
