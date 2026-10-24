import { expect, test } from "bun:test";
import {
  PLAY,
  closeLayer,
  escapeFrom,
  leaveConsequence,
  leaveNeedsConfirmation,
  menuPausesWorld,
  openLeaveConfirmation,
  openMenu,
  openSettings,
} from "./game-menu.js";

test("Escape opens the menu from play and steps out of the innermost layer", () => {
  expect(escapeFrom(PLAY)).toEqual(openMenu("resume"));
  expect(escapeFrom(openMenu())).toEqual(PLAY);
  expect(escapeFrom(openSettings("menu"))).toEqual(openMenu("settings"));
  expect(escapeFrom(openLeaveConfirmation())).toEqual(openMenu("leave"));
  expect(escapeFrom(openSettings("play"))).toEqual(PLAY);
});

test("closing settings returns to where they were opened from", () => {
  expect(closeLayer(openSettings("menu"))).toEqual(openMenu("settings"));
  expect(closeLayer(openSettings("header"))).toEqual(PLAY);
  expect(closeLayer(openLeaveConfirmation())).toEqual(openMenu("leave"));
});

test("only a non-training run confirms leaving and names its consequence", () => {
  expect(leaveNeedsConfirmation({ training: true })).toBe(false);
  expect(leaveNeedsConfirmation({ training: false })).toBe(true);
  expect(leaveConsequence("local")).toContain("last save");
  expect(leaveConsequence("host")).toContain("everyone");
  expect(leaveConsequence("guest")).toContain("disconnect");
});

test("only local play pauses for the menu", () => {
  expect(["local", "host", "guest", "dedicated"].filter(menuPausesWorld)).toEqual(["local"]);
});
