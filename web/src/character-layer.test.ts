import { expect, test } from "bun:test";
import { createCharacterLayer, presentedFacing } from "./character-layer.ts";

function fakeAsset() {
  const created: any[] = [];
  return {
    created,
    createInstance() {
      const instance = {
        root: {
          position: {
            value: [0, 0, 0],
            set(x: number, y: number, z: number) {
              this.value = [x, y, z];
            },
          },
          rotation: {
            value: [0, 0, 0],
            set(x: number, y: number, z: number) {
              this.value = [x, y, z];
            },
          },
        },
        seeks: [] as unknown[],
        advanced: 0,
        disposed: false,
        seek(clip: string, seconds: number) {
          this.seeks.push([clip, seconds]);
        },
        advance(seconds: number) {
          this.advanced += seconds;
        },
        dispose() {
          this.disposed = true;
        },
      };
      created.push(instance);
      return instance;
    },
  };
}

const player = (id: number, x: number, z: number, facing = [0, 1]) => ({
  id,
  position: [x, 0, z],
  facing,
});

test("one idle humanoid per player, placed in metres and turned to its facing", () => {
  const asset = fakeAsset();
  let time = 1000;
  const layer = createCharacterLayer({
    getAsset: () => asset,
    idleClip: "Idle_Loop",
    now: () => time,
  });
  const first = layer.sync({
    worldUnitsPerMeter: 100,
    players: [player(1, 250, -100, [1, 0]), player(2, 0, 0)],
  });
  expect(first.changed).toBe(true);
  expect(first.roots).toHaveLength(2);
  const [one, two] = asset.created;
  expect(one.seeks).toEqual([["Idle_Loop", 0]]);
  expect(one.root.position.value).toEqual([2.5, 0, -1]);
  expect(one.root.rotation.value[1]).toBeCloseTo(Math.PI / 2);
  expect(two.root.rotation.value[1]).toBeCloseTo(0);

  time = 1500;
  const again = layer.sync({
    worldUnitsPerMeter: 100,
    players: [player(1, 300, -100, [1, 0]), player(2, 0, 0)],
  });
  expect(again.changed).toBe(false);
  expect(asset.created).toHaveLength(2);
  expect(one.advanced).toBeCloseTo(0.5);
  expect(one.root.position.value).toEqual([3, 0, -1]);
});

test("departed players release their instance; dispose releases all", () => {
  const asset = fakeAsset();
  const layer = createCharacterLayer({
    getAsset: () => asset,
    idleClip: "Idle_Loop",
    now: () => 0,
  });
  layer.sync({ worldUnitsPerMeter: 100, players: [player(1, 0, 0), player(2, 0, 0)] });
  const left = layer.sync({ worldUnitsPerMeter: 100, players: [player(1, 0, 0)] });
  expect(left.changed).toBe(true);
  expect(left.roots).toHaveLength(1);
  expect(asset.created[1].disposed).toBe(true);
  layer.dispose();
  expect(asset.created[0].disposed).toBe(true);
});

test("no instances exist before the asset is loaded", () => {
  let asset: ReturnType<typeof fakeAsset> | null = null;
  const layer = createCharacterLayer({
    getAsset: () => asset,
    idleClip: "Idle_Loop",
    now: () => 0,
  });
  expect(layer.sync({ worldUnitsPerMeter: 100, players: [player(1, 0, 0)] })).toEqual({
    roots: [],
    changed: false,
  });
  asset = fakeAsset();
  expect(layer.sync({ worldUnitsPerMeter: 100, players: [player(1, 0, 0)] }).changed).toBe(true);
});

test("the humanoid faces the committed action, then a lock's facing, then aim", () => {
  expect(
    presentedFacing({
      id: 1,
      position: [0, 0, 0],
      facing: [1, 0],
      aim: [0, 1],
      action: { facing: [-1, 0], aim: null },
    }),
  ).toEqual([-1, 0]);
  expect(
    presentedFacing({
      id: 1,
      position: [0, 0, 0],
      facing: [1, 0],
      aim: [0, 1],
      lockedMonsterId: 3,
    }),
  ).toEqual([1, 0]);
  expect(presentedFacing({ id: 1, position: [0, 0, 0], facing: [1, 0], aim: [0, 1] })).toEqual([
    0, 1,
  ]);
});
