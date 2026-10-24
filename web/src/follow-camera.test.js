import { expect, test } from "bun:test";
import { aimFromPointer } from "./aim-input.js";
import {
  CAMERA_OFFSET,
  DEFAULT_ZOOM,
  ZOOM_MAX,
  ZOOM_MIN,
  cameraPose,
  createFollowCamera,
  createPinchZoom,
  loadCameraZoom,
  persistCameraZoom,
} from "./follow-camera.js";
import { buildFrame } from "./scene-frame.ts";

const memoryStorage = () => {
  const values = new Map();
  return {
    getItem: (key) => values.get(key) ?? null,
    setItem: (key, value) => values.set(key, value),
  };
};

test("the camera snaps onto its first focus and glides frame-rate independently", () => {
  const glide = (frameMs) => {
    const camera = createFollowCamera();
    camera.follow([0, 0, 0], 0);
    for (let frame = 1; frame <= Math.round(200 / frameMs); frame += 1)
      camera.follow([2, 0, 0], frame * frameMs);
    return camera.pose().target[0];
  };
  expect(glide(1000 / 60)).toBeGreaterThan(0.5);
  expect(glide(1000 / 60)).toBeLessThan(2);
  expect(glide(1000 / 60)).toBeCloseTo(glide(1000 / 120), 6);
  expect(glide(1000 / 60)).toBeCloseTo(glide(50), 6);
});

test("the camera settles, and snaps across teleports", () => {
  const camera = createFollowCamera();
  expect(camera.follow([0, 0, 0], 0)).toBe(true);
  expect(camera.follow([1, 0, 0], 16)).toBe(false);
  let time = 16;
  while (!camera.follow([1, 0, 0], (time += 16)) && time < 10_000);
  expect(camera.pose().target).toEqual([1, 0, 0]);
  expect(camera.follow([30, 0, 0], time + 16)).toBe(true);
  expect(camera.pose().target).toEqual([30, 0, 0]);
});

test("zoom is bounded, scales the offset, resets and persists", () => {
  const camera = createFollowCamera();
  camera.follow([0, 0, 0], 0);
  camera.zoomByWheel(100_000);
  expect(camera.zoom()).toBe(ZOOM_MAX);
  camera.zoomByWheel(-100_000);
  expect(camera.zoom()).toBe(ZOOM_MIN);
  expect(camera.pose().position).toEqual(CAMERA_OFFSET.map((value) => value * ZOOM_MIN));
  camera.reset();
  expect(camera.zoom()).toBe(DEFAULT_ZOOM);

  const storage = memoryStorage();
  expect(loadCameraZoom(storage)).toBe(DEFAULT_ZOOM);
  persistCameraZoom(1.25, storage);
  expect(loadCameraZoom(storage)).toBe(1.25);
  storage.setItem("arpg-camera-zoom", "nonsense");
  expect(loadCameraZoom(storage)).toBe(DEFAULT_ZOOM);
  storage.setItem("arpg-camera-zoom", "99");
  expect(loadCameraZoom(storage)).toBe(ZOOM_MAX);
});

test("zoom and reset notify listeners", () => {
  const camera = createFollowCamera();
  let calls = 0;
  camera.subscribe(() => (calls += 1));
  camera.setZoom(1.2);
  camera.setZoom(1.2);
  camera.reset();
  expect(calls).toBe(2);
});

test("spreading two fingers zooms in", () => {
  const pinch = createPinchZoom();
  pinch.down(1, 100, 100);
  expect(pinch.move(1, 90, 100)).toBeNull();
  pinch.down(2, 200, 100);
  expect(pinch.move(2, 310, 100)).toBeCloseTo(0.5, 6);
  pinch.up(2);
  expect(pinch.move(1, 0, 0)).toBeNull();
});

test("pointer aim projects through the drawn pose from the player", () => {
  // A player at (5, 0, 5) under a zoomed camera: pointing at screen centre is the
  // camera's look target, here the player itself.
  const pose = cameraPose([5, 0, 5], 1.4);
  expect(aimFromPointer(600, 400, 1200, 800, pose, [5, 0, 5])).toBeNull();
  const right = aimFromPointer(1200, 400, 1200, 800, pose, [5, 0, 5]);
  expect(Math.abs((Math.atan2(right[1], right[0]) * 180) / Math.PI + 45)).toBeLessThanOrEqual(1);
  // The camera still lagging behind: the player's own position stays the aim origin.
  const lagging = aimFromPointer(600, 400, 1200, 800, cameraPose([4, 0, 5]), [5, 0, 5]);
  expect(lagging).toEqual([-1000, 0]);
});

const wall = (id, position, halfExtents) => ({ id, kind: "wall", position, halfExtents });
const room = (colliders) => ({
  tick: 1,
  worldUnitsPerMeter: 100,
  rooms: [],
  staticColliders: colliders,
  chests: [],
  arrows: [],
  groundLoot: [],
  monsters: [],
  players: [{ id: 1, position: [0, 0, 0], facing: [1, 0], alive: true, weapon: "swordAndShield" }],
});
const heightOf = (frame, id) => frame.nodes.find((node) => node.id === id).geometry.size[1];

test("barriers lower only when they block the actual camera's view", () => {
  // Camera at +x/+z: a wall 2 m toward it blocks the view; one behind the player does not.
  const snapshot = room([
    wall(1, [100, 150, 100], [100, 150, 20]),
    wall(2, [-100, 150, -100], [100, 150, 20]),
  ]);
  const frame = buildFrame(snapshot, 1, 800, 600);
  expect(heightOf(frame, "static-1")).toBeLessThan(1);
  expect(heightOf(frame, "static-2")).toBe(3);
  // A camera drawn elsewhere (still gliding toward the player) uses its own line of sight.
  const elsewhere = buildFrame(snapshot, 1, 800, 600, undefined, undefined, {
    target: [0, 0, 0],
    position: [-10, 11, -10],
  });
  expect(heightOf(elsewhere, "static-1")).toBe(3);
  expect(heightOf(elsewhere, "static-2")).toBeLessThan(1);
  // The authoritative colliders are never changed.
  expect(snapshot.staticColliders[0].halfExtents).toEqual([100, 150, 20]);
});

test("barriers also lower in front of a telegraphing monster", () => {
  const snapshot = room([wall(1, [1000, 150, 80], [100, 150, 20])]);
  snapshot.monsters = [
    { id: 7, position: [1000, 0, 0], alive: true, behavior: "pursuing", action: null },
  ];
  expect(heightOf(buildFrame(snapshot, 1, 800, 600), "static-1")).toBe(3);
  snapshot.monsters[0].action = { phase: "windup" };
  expect(heightOf(buildFrame(snapshot, 1, 800, 600), "static-1")).toBeLessThan(1);
});

test("without smoothing the camera snaps and is settled at once", () => {
  const camera = createFollowCamera();
  camera.follow([0, 0, 0], 0);
  expect(camera.follow([1, 0, 0], 16, false)).toBe(true);
  expect(camera.pose().target).toEqual([1, 0, 0]);
});
