import { expect, test } from "bun:test";
import {
  GRAPHICS_KEY,
  MOUSE_AIM_KEY,
  loadGraphics,
  loadMouseAim,
  persistStoredValue,
  readStoredValue,
} from "./preferences.js";

test("storage failure leaves the current session usable", () => {
  const blocked = {
    getItem() {
      throw new Error("blocked");
    },
    setItem() {
      throw new Error("blocked");
    },
  };
  expect(readStoredValue("endpoint", "fallback", blocked)).toBe("fallback");
  expect(persistStoredValue("endpoint", "new-value", blocked)).toBe(false);
});

test("graphics accepts only supported settings and discards unrelated persisted fields", () => {
  const storage = (value) => ({
    getItem: (key) => (key === GRAPHICS_KEY ? JSON.stringify(value) : null),
  });
  for (const pixelRatioLimit of [-1, 0, 100, "2", null]) {
    expect(loadGraphics(storage({ shadows: false, pixelRatioLimit }))).toEqual({
      shadows: true,
      pixelRatioLimit: 2,
    });
  }
  expect(
    loadGraphics(storage({ shadows: false, pixelRatioLimit: 1.5, other: "untrusted" })),
  ).toEqual({ shadows: false, pixelRatioLimit: 1.5 });
});

test("mouse facing is enabled unless the player explicitly opts out", () => {
  const storage = (value) => ({ getItem: (key) => (key === MOUSE_AIM_KEY ? value : null) });
  expect(loadMouseAim(storage(null))).toBe(true);
  expect(loadMouseAim(storage("yes"))).toBe(true);
  expect(loadMouseAim(storage("on"))).toBe(true);
  expect(loadMouseAim(storage("off"))).toBe(false);
  expect(
    loadMouseAim({
      getItem: () => {
        throw new Error("blocked");
      },
    }),
  ).toBe(true);
});
