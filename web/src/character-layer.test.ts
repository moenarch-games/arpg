import { expect, test } from "bun:test";
import { RANGER_BUNDLE } from "./character-bundles.js";
import { chooseClip, createCharacterLayer, presentedFacing } from "./character-layer.ts";

const animation = RANGER_BUNDLE.animation;
const DEATH_SECONDS = 2.4;

function fakeAsset() {
  const created: any[] = [];
  return {
    created,
    clips: [
      { name: "Idle_Loop", duration: 2.5 },
      { name: "Walk_Loop", duration: 1.33 },
      { name: "Jog_Fwd_Loop", duration: 0.93 },
      { name: "Sprint_Loop", duration: 0.67 },
      { name: "Death01", duration: DEATH_SECONDS },
      { name: "Sword_Idle", duration: 1.67 },
      { name: "Idle_Shield_Loop", duration: 2.5 },
    ],
    equipped: [] as unknown[][],
    equip(instance: unknown, weapon: string) {
      const entry = [instance, weapon, true];
      this.equipped.push(entry);
      return () => {
        entry[2] = false;
      };
    },
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
        calls: [] as unknown[][],
        advanced: 0,
        disposed: false,
        seek(clip: string, seconds: number) {
          this.calls.push(["seek", clip, seconds]);
        },
        transition(clip: string, seconds: number) {
          this.calls.push(["transition", clip, seconds]);
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

// 100 world units per metre at 60 Hz: `perTick` units per tick is perTick * 0.6 m/s.
function run(
  layer: ReturnType<typeof createCharacterLayer>,
  from: number,
  ticks: number,
  perTick: number,
  extra: object = {},
) {
  let x = from;
  for (let tick = 1; tick <= ticks; tick += 1) {
    x += perTick;
    layer.sync({
      tick: tickBase + tick,
      worldUnitsPerMeter: 100,
      players: [{ id: 1, position: [x, 0, 0], facing: [1, 0], ...extra }],
    });
  }
  tickBase += ticks;
  return x;
}
let tickBase = 0;
function fixture() {
  tickBase = 0;
  const asset = fakeAsset();
  const layer = createCharacterLayer({ getAsset: () => asset, animation, tickHz: 60 });
  layer.sync({
    tick: 0,
    worldUnitsPerMeter: 100,
    players: [{ id: 1, position: [0, 0, 0], facing: [1, 0] }],
  });
  return { asset, layer, instance: asset.created[0] };
}
const transitions = (instance: any) =>
  instance.calls
    .filter((call: unknown[]) => call[0] === "transition")
    .map((call: unknown[]) => call[1]);

test("clips are chosen by authoritative speed and life", () => {
  expect(chooseClip(animation, { alive: true, speed: 0 })).toBe("Idle_Loop");
  expect(chooseClip(animation, { alive: true, speed: 1.2 })).toBe("Walk_Loop");
  expect(chooseClip(animation, { alive: true, speed: 4.2 })).toBe("Jog_Fwd_Loop");
  expect(chooseClip(animation, { alive: true, speed: 7 })).toBe("Sprint_Loop");
  expect(chooseClip(animation, { alive: false, speed: 4 })).toBe("Death01");
  // Standing: guarding beats engaged, engaged draws the sword; moving keeps the gait.
  expect(chooseClip(animation, { alive: true, speed: 0, guarding: true, engaged: true })).toBe(
    "Idle_Shield_Loop",
  );
  expect(chooseClip(animation, { alive: true, speed: 0, engaged: true })).toBe("Sword_Idle");
  expect(chooseClip(animation, { alive: true, speed: 4.2, guarding: true })).toBe("Jog_Fwd_Loop");
});

test("a new character idles, places in metres and faces its direction", () => {
  const { instance } = fixture();
  expect(instance.calls).toEqual([["seek", "Idle_Loop", 0]]);
  expect(instance.root.rotation.value[1]).toBeCloseTo(Math.PI / 2);
});

test("moving at full player speed jogs with a scaled rate and crossfades once", () => {
  const { layer, instance } = fixture();
  // 7 units per tick is the player's 4.2 m/s.
  run(layer, 0, 60, 7);
  expect(transitions(instance)).toEqual(["Walk_Loop", "Jog_Fwd_Loop"]);
  const before = instance.advanced;
  run(layer, 420, 60, 7);
  // One simulated second at 4.2 m/s on a 3.4 m/s stride.
  expect(instance.advanced - before).toBeCloseTo(4.2 / 3.4, 2);
  expect(transitions(instance)).toEqual(["Walk_Loop", "Jog_Fwd_Loop"]);
  run(layer, 840, 30, 0);
  expect(transitions(instance).at(-1)).toBe("Idle_Loop");
});

test("animation time follows ticks: a repeated tick (pause) advances nothing", () => {
  const { layer, instance } = fixture();
  layer.sync({ tick: 0, worldUnitsPerMeter: 100, players: [{ id: 1, position: [0, 0, 0] }] });
  layer.sync({ tick: 0, worldUnitsPerMeter: 100, players: [{ id: 1, position: [0, 0, 0] }] });
  expect(instance.advanced).toBe(0);
  layer.sync({ tick: 1, worldUnitsPerMeter: 100, players: [{ id: 1, position: [0, 0, 0] }] });
  expect(instance.advanced).toBeCloseTo(1 / 60);
});

test("death plays once and holds its last pose until revived", () => {
  const { layer, instance } = fixture();
  run(layer, 0, 180, 0, { alive: false });
  expect(transitions(instance)).toEqual(["Death01"]);
  // Held just short of the end, never looping.
  expect(instance.advanced).toBeCloseTo(DEATH_SECONDS - 1 / 60, 3);
  run(layer, 0, 10, 0, { alive: true });
  expect(transitions(instance)).toEqual(["Death01", "Idle_Loop"]);
});

test("a tick going backwards (new session) resets speed without advancing", () => {
  const { layer, instance } = fixture();
  run(layer, 0, 30, 7);
  const advanced = instance.advanced;
  layer.sync({ tick: 1, worldUnitsPerMeter: 100, players: [{ id: 1, position: [0, 0, 0] }] });
  expect(instance.advanced).toBe(advanced);
  expect(transitions(instance).at(-1)).toBe("Idle_Loop");
});

test("departed players release their instance; dispose releases all", () => {
  const asset = fakeAsset();
  const layer = createCharacterLayer({ getAsset: () => asset, animation, tickHz: 60 });
  layer.sync({
    tick: 0,
    worldUnitsPerMeter: 100,
    players: [
      { id: 1, position: [0, 0, 0] },
      { id: 2, position: [0, 0, 0] },
    ],
  });
  const left = layer.sync({
    tick: 1,
    worldUnitsPerMeter: 100,
    players: [{ id: 1, position: [0, 0, 0] }],
  });
  expect(left.changed).toBe(true);
  expect(asset.created[1].disposed).toBe(true);
  layer.dispose();
  expect(asset.created[0].disposed).toBe(true);
});

test("no instances exist before the asset is loaded", () => {
  let asset: ReturnType<typeof fakeAsset> | null = null;
  const layer = createCharacterLayer({ getAsset: () => asset, animation, tickHz: 60 });
  expect(
    layer.sync({ tick: 0, worldUnitsPerMeter: 100, players: [{ id: 1, position: [0, 0, 0] }] }),
  ).toEqual({ roots: [], changed: false });
  asset = fakeAsset();
  expect(
    layer.sync({ tick: 1, worldUnitsPerMeter: 100, players: [{ id: 1, position: [0, 0, 0] }] })
      .changed,
  ).toBe(true);
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

test("each character carries its weapon's equipment and swaps it when the weapon changes", () => {
  const { asset, layer, instance } = fixture();
  expect(asset.equipped).toEqual([]);
  const at = (tick: number, weapon: string) =>
    layer.sync({
      tick,
      worldUnitsPerMeter: 100,
      players: [{ id: 1, position: [0, 0, 0], weapon }],
    });
  at(1, "swordAndShield");
  expect(asset.equipped).toEqual([[instance, "swordAndShield", true]]);
  at(2, "swordAndShield");
  expect(asset.equipped).toHaveLength(1);
  at(3, "bow");
  expect(asset.equipped).toEqual([
    [instance, "swordAndShield", false],
    [instance, "bow", true],
  ]);
  layer.dispose();
  expect(asset.equipped[1][2]).toBe(false);
});

test("a monster engaged with the character draws its sword while standing", () => {
  const { layer, instance } = fixture();
  layer.sync({
    tick: 1,
    worldUnitsPerMeter: 100,
    players: [{ id: 1, position: [0, 0, 0] }],
    monsters: [{ alive: true, targetPlayerId: 1 }],
  });
  expect(transitions(instance)).toEqual(["Sword_Idle"]);
  layer.sync({
    tick: 2,
    worldUnitsPerMeter: 100,
    players: [{ id: 1, position: [0, 0, 0], guard: { phase: "raised" } }],
  });
  expect(transitions(instance)).toEqual(["Sword_Idle", "Idle_Shield_Loop"]);
});
