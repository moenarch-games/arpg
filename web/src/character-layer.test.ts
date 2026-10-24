import { expect, test } from "bun:test";
import { RANGER_BUNDLE } from "./character-bundles.js";
import { chooseClip, createCharacterLayer, presentedFacing } from "./character-layer.ts";

const animation = RANGER_BUNDLE.animation;
const DEATH_SECONDS = 2.4;

function fakeAsset(extraClips: { name: string; duration: number }[] = []) {
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
      ...extraClips,
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
          this.clipTime = 0;
        },
        advance(seconds: number) {
          this.advanced += seconds;
          this.clipTime += seconds;
        },
        // Seconds into the clip last started.
        clipTime: 0,
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
  const layer = createCharacterLayer({
    getAsset: () => asset,
    animation,
    tickHz: 60,
  });
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
  expect(
    chooseClip(animation, {
      alive: true,
      speed: 0,
      guarding: true,
      engaged: true,
    }),
  ).toBe("Idle_Shield_Loop");
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
  layer.sync({
    tick: 0,
    worldUnitsPerMeter: 100,
    players: [{ id: 1, position: [0, 0, 0] }],
  });
  layer.sync({
    tick: 0,
    worldUnitsPerMeter: 100,
    players: [{ id: 1, position: [0, 0, 0] }],
  });
  expect(instance.advanced).toBe(0);
  layer.sync({
    tick: 1,
    worldUnitsPerMeter: 100,
    players: [{ id: 1, position: [0, 0, 0] }],
  });
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
  layer.sync({
    tick: 1,
    worldUnitsPerMeter: 100,
    players: [{ id: 1, position: [0, 0, 0] }],
  });
  expect(instance.advanced).toBe(advanced);
  expect(transitions(instance).at(-1)).toBe("Idle_Loop");
});

test("departed players release their instance; dispose releases all", () => {
  const asset = fakeAsset();
  const layer = createCharacterLayer({
    getAsset: () => asset,
    animation,
    tickHz: 60,
  });
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
  const layer = createCharacterLayer({
    getAsset: () => asset,
    animation,
    tickHz: 60,
  });
  expect(
    layer.sync({
      tick: 0,
      worldUnitsPerMeter: 100,
      players: [{ id: 1, position: [0, 0, 0] }],
    }),
  ).toEqual({ roots: [], changed: false });
  asset = fakeAsset();
  expect(
    layer.sync({
      tick: 1,
      worldUnitsPerMeter: 100,
      players: [{ id: 1, position: [0, 0, 0] }],
    }).changed,
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
  expect(
    presentedFacing({
      id: 1,
      position: [0, 0, 0],
      facing: [1, 0],
      aim: [0, 1],
    }),
  ).toEqual([0, 1]);
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

test("the humanoid stands at the body's feet and rises with a jump without changing gait (#59)", () => {
  const { layer, instance } = fixture();
  const motion = (feetY: number) => ({
    grounded: feetY === 0,
    feetY,
    groundY: 0,
  });
  layer.sync({
    tick: 1,
    worldUnitsPerMeter: 100,
    players: [{ id: 1, position: [0, 50, 0], facing: [1, 0], motion: motion(0) }],
  });
  // The character's origin is its feet: on the floor, not at the body's centre.
  expect(instance.root.position.value).toEqual([0, 0, 0]);
  // Rising straight up at 6 units per tick: no horizontal speed, so it keeps standing.
  for (let tick = 2; tick <= 12; tick += 1) {
    const feet = (tick - 1) * 6;
    layer.sync({
      tick,
      worldUnitsPerMeter: 100,
      players: [
        {
          id: 1,
          position: [0, 50 + feet, 0],
          facing: [1, 0],
          motion: motion(feet),
        },
      ],
    });
    expect(instance.root.position.value[1]).toBeCloseTo(feet / 100, 6);
  }
  expect(transitions(instance)).toEqual([]);
});

// The Ranger's combat, reaction, bow and jump clips (humanoid-clips durations).
const COMBAT_CLIPS = [
  { name: "Sword_Regular_A", duration: 0.4333 },
  { name: "Sword_Regular_A_Rec", duration: 0.9667 },
  { name: "Sword_Regular_B", duration: 0.5333 },
  { name: "Sword_Regular_B_Rec", duration: 1.0333 },
  { name: "Sword_Regular_C", duration: 2 },
  { name: "Hit_Chest", duration: 0.3333 },
  { name: "Sword_Block", duration: 1.2333 },
  { name: "Idle_Shield_Break", duration: 1.0667 },
  { name: "Bow_Draw", duration: 0.7 },
  { name: "Bow_Hold_Loop", duration: 2.5 },
  { name: "Bow_Release", duration: 0.4667 },
  { name: "Bow_Recovery", duration: 1.1 },
  { name: "Jump_Start", duration: 1.3333 },
  { name: "Jump_Loop", duration: 2.5 },
  { name: "Jump_Land", duration: 1.2667 },
];

function combatFixture() {
  const asset = fakeAsset(COMBAT_CLIPS);
  const layer = createCharacterLayer({
    getAsset: () => asset,
    animation,
    tickHz: 60,
  });
  const at = (tick: number, extra: object = {}) =>
    layer.sync({
      tick,
      worldUnitsPerMeter: 100,
      players: [{ id: 1, position: [0, 0, 0], facing: [1, 0], ...extra }],
    });
  at(0);
  return { asset, layer, at, instance: asset.created[0] };
}
// The light swing's authored timing: 5 wind-up, 1 active, 8 recovery ticks.
const light = (
  phase: string,
  ticksRemaining: number,
  phaseTicks: number,
  kind = "primaryAttack",
) => ({
  action: { kind, phase, ticksRemaining, phaseTicks, facing: [1, 0] },
});

test("each action phase plays its stretch of the strike: contact lands as the active phase opens", () => {
  const { at, instance } = combatFixture();
  at(1, light("windup", 5, 5));
  expect(instance.calls.at(-1)).toEqual(["transition", "Sword_Regular_A", 0.08]);
  expect(instance.clipTime).toBe(0);
  at(2, light("windup", 4, 5));
  expect(instance.clipTime).toBeCloseTo(0.25 / 5, 6);
  at(5, light("windup", 1, 5));
  expect(instance.clipTime).toBeCloseTo(0.2, 6);
  at(6, light("active", 1, 1));
  // The contact time of Sword_Regular_A, no restart.
  expect(instance.clipTime).toBeCloseTo(0.25, 6);
  expect(transitions(instance)).toEqual(["Sword_Regular_A"]);
  at(7, light("recovery", 8, 8));
  expect(transitions(instance)).toEqual(["Sword_Regular_A", "Sword_Regular_A_Rec"]);
  at(11, light("recovery", 4, 8));
  expect(instance.clipTime).toBeCloseTo(0.9667 / 2, 4);
  // The action ends: back to standing.
  at(15);
  expect(transitions(instance).at(-1)).toBe("Idle_Loop");
});

test("duplicate and late snapshots move the strike to the timeline without restarting it", () => {
  const { at, instance } = combatFixture();
  at(1, light("windup", 5, 5));
  at(1, light("windup", 5, 5));
  expect(instance.advanced).toBe(0);
  // Three ticks were lost: the pose lands where the phase is.
  at(4, light("windup", 2, 5));
  expect(instance.clipTime).toBeCloseTo(0.15, 6);
  // A stale snapshot repeating an earlier tick changes nothing.
  at(4, light("windup", 2, 5));
  expect(instance.clipTime).toBeCloseTo(0.15, 6);
  expect(transitions(instance)).toEqual(["Sword_Regular_A"]);
});

test("a combo strike and the heavy strike crossfade into their own clips", () => {
  const { at, instance } = combatFixture();
  at(1, light("windup", 5, 5));
  at(7, light("recovery", 8, 8));
  at(8, light("windup", 6, 6, "lightFollowUp"));
  at(20, {
    action: {
      kind: "secondaryAttack",
      phase: "windup",
      ticksRemaining: 10,
      phaseTicks: 20,
      facing: [1, 0],
    },
  });
  expect(transitions(instance)).toEqual([
    "Sword_Regular_A",
    "Sword_Regular_A_Rec",
    "Sword_Regular_B",
    "Sword_Regular_C",
  ]);
  // Halfway through the heavy wind-up is halfway to its 0.65 s contact.
  expect(instance.clipTime).toBeCloseTo(0.325, 6);
});

test("a hit interrupts the strike; a renewed hit restarts the flinch; death overrides", () => {
  const { at, instance } = combatFixture();
  at(1, light("windup", 5, 5));
  at(2, { reaction: { kind: "hurt", ticksRemaining: 12 } });
  expect(transitions(instance)).toEqual(["Sword_Regular_A", "Hit_Chest"]);
  at(8, { reaction: { kind: "hurt", ticksRemaining: 6 } });
  expect(instance.clipTime).toBeCloseTo(0.1, 6);
  at(9, { reaction: { kind: "hurt", ticksRemaining: 12 } });
  expect(transitions(instance)).toEqual(["Sword_Regular_A", "Hit_Chest", "Hit_Chest"]);
  expect(instance.clipTime).toBe(0);
  at(10, { alive: false, ...light("windup", 5, 5) });
  expect(transitions(instance).at(-1)).toBe("Death01");
});

test("the bow draw follows the authoritative draw ticks, then holds", () => {
  const { at, instance } = combatFixture();
  at(1, { drawTicks: 1 });
  at(31, { drawTicks: 31 });
  expect(transitions(instance)).toEqual(["Bow_Draw"]);
  expect(instance.clipTime).toBeCloseTo(31 / 60, 6);
  at(60, { drawTicks: 60 });
  expect(transitions(instance)).toEqual(["Bow_Draw", "Bow_Hold_Loop"]);
  expect(instance.clipTime).toBeCloseTo(1 - 0.7, 6);
});

test("a jump takes off, falls and lands from authoritative support", () => {
  const { at, instance } = combatFixture();
  const air = (grounded: boolean, verticalVelocity: number) => ({
    motion: {
      grounded,
      verticalVelocity,
      feetY: grounded ? 0 : 50,
      groundY: 0,
    },
  });
  at(1, air(true, 0));
  at(2, air(false, 300));
  expect(transitions(instance)).toEqual(["Jump_Start"]);
  at(100, air(false, -300));
  expect(transitions(instance)).toEqual(["Jump_Start", "Jump_Loop"]);
  at(101, air(true, 0));
  expect(transitions(instance)).toEqual(["Jump_Start", "Jump_Loop", "Jump_Land"]);
  at(130, air(true, 0));
  expect(transitions(instance).at(-1)).toBe("Idle_Loop");
});

test("a brief step off a ledge never reads as a jump", () => {
  const { at, instance } = combatFixture();
  at(1, {
    motion: { grounded: false, verticalVelocity: -10, feetY: 0, groundY: 0 },
  });
  at(3, {
    motion: { grounded: true, verticalVelocity: 0, feetY: 0, groundY: 0 },
  });
  expect(transitions(instance)).toEqual([]);
});

test("mapped clips the asset lacks fall back to locomotion and are reported", () => {
  const { layer, instance } = fixture();
  layer.sync({
    tick: 1,
    worldUnitsPerMeter: 100,
    players: [{ id: 1, position: [0, 0, 0], ...light("windup", 5, 5) }],
  });
  expect(transitions(instance)).toEqual([]);
  expect(layer.missingClips()).toEqual(["Sword_Regular_A"]);
});
