import { expect, test } from "bun:test";
import { aimFromPointer, movementFromAim } from "./aim-input.js";

const WIDTH = 1200;
const HEIGHT = 800;
const angleOf = ([x, z]) => (Math.atan2(z, x) * 180) / Math.PI;
// Within one two-degree quantisation step of the expected screen direction.
const expectAngle = (aim, expected) =>
  expect(Math.abs(angleOf(aim) - expected)).toBeLessThanOrEqual(1);

test("the pointer aims from the player towards the ground it covers", () => {
  // The camera sits at +x/+z looking back at the player: screen right is world (+x, −z)
  // and screen up is world (−x, −z).
  expectAngle(aimFromPointer(WIDTH, HEIGHT / 2, WIDTH, HEIGHT), -45);
  expectAngle(aimFromPointer(WIDTH / 2, 0, WIDTH, HEIGHT), -135);
  expectAngle(aimFromPointer(0, HEIGHT / 2, WIDTH, HEIGHT), 135);
  expectAngle(aimFromPointer(WIDTH / 2, HEIGHT, WIDTH, HEIGHT), 45);
});

test("aim directions are bounded integers quantised to two-degree steps", () => {
  for (let x = 0; x <= WIDTH; x += 97) {
    for (let y = 0; y <= HEIGHT; y += 89) {
      const aim = aimFromPointer(x, y, WIDTH, HEIGHT);
      if (!aim) continue;
      expect(aim.every(Number.isInteger)).toBe(true);
      expect(aim.every((component) => Math.abs(component) <= 1000)).toBe(true);
      expect(aim.some((component) => component !== 0)).toBe(true);
      expect(Math.abs(angleOf(aim) / 2 - Math.round(angleOf(aim) / 2))).toBeLessThan(0.05);
    }
  }
  // Small pointer jitter yields the same command.
  expect(aimFromPointer(900, 300, WIDTH, HEIGHT)).toEqual(aimFromPointer(901, 300, WIDTH, HEIGHT));
});

test("the player's own position and empty canvases give no aim", () => {
  expect(aimFromPointer(WIDTH / 2, HEIGHT / 2, WIDTH, HEIGHT)).toBeNull();
  expect(aimFromPointer(10, 10, 0, 0)).toBeNull();
});

test("mouse locomotion maps aim to canonical eight-way movement", () => {
  expect(movementFromAim(null)).toEqual([0, 0]);
  expect(movementFromAim([1000, 0])).toEqual([1, 0]);
  expect(movementFromAim([0, -1000])).toEqual([0, -1]);
  expect(movementFromAim([707, -707])).toEqual([1, -1]);
  expect(movementFromAim([-1000, -1000])).toEqual([-1, -1]);
  expect(movementFromAim([200, 1000])).toEqual([0, 1]);
  expect(movementFromAim([-1000, -100])).toEqual([-1, 0]);
});
