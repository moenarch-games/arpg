import { describe, expect, test } from "bun:test";
import { attachCueAudio, audioEntity, cueSound, listenerPose, soundKind } from "./cue-audio.js";
import { createPresentationCues } from "./presentation-cues.js";

const player = (patch = {}) => ({
  id: 1,
  position: [0, 0, 0],
  alive: true,
  action: null,
  counter: null,
  ...patch,
});
const monster = (patch = {}) => ({
  id: 1,
  definition: "monster.brute",
  position: [300, 0, 400],
  alive: true,
  ...patch,
});
const frame = (tick, patch = {}) => ({
  tick,
  runSeed: 42,
  scenario: "training",
  contentRevision: "r1",
  worldUnitsPerMeter: 100,
  players: [player()],
  monsters: [monster()],
  rooms: [{ id: 1, encounterState: "active" }],
  arrows: [],
  strikeEvents: [],
  interactionEvents: [],
  ...patch,
});
const strike = (result, patch = {}) => ({
  order: 0,
  source: { kind: "player", id: 1 },
  strikeTick: 5,
  definition: "sword.lightSwing",
  target: { kind: "monster", id: 1 },
  result,
  ...patch,
});
const hit = (patch = {}) => strike({ kind: "hit", damage: 25, defeated: false }, patch);

// A game-audio handle double that records calls and, like the director, sounds a key once.
function fakeAudio(state = "running") {
  const calls = [];
  const seen = new Set();
  return {
    calls,
    current: state,
    triggered: () => calls.filter(([name]) => name === "trigger").map(([, cue]) => cue),
    state() {
      return this.current;
    },
    trigger(cue) {
      calls.push(["trigger", cue]);
      if (seen.has(cue.key)) return "duplicate";
      seen.add(cue.key);
      return "started";
    },
    setListener: (position, right) => calls.push(["setListener", position, right]),
    setEntityPosition: (entity, position) => calls.push(["setEntityPosition", entity, position]),
    stopEntity: (entity) => calls.push(["stopEntity", entity]),
    reset: () => calls.push(["reset"]),
    pump: () => calls.push(["pump"]),
  };
}

function setup(state) {
  const cues = createPresentationCues();
  const audio = fakeAudio(state);
  const attached = attachCueAudio({ cues, audio, getPlayerId: () => 1 });
  return { cues, audio, attached };
}

describe("cue to sound mapping", () => {
  test("player and monster ids never collide as audio entities", () => {
    expect(audioEntity({ kind: "player", id: 1 })).toBe(2);
    expect(audioEntity({ kind: "monster", id: 1 })).toBe(3);
    expect(audioEntity({ kind: "player", id: 0 })).toBe(0);
    for (const party of [
      { kind: "crate", id: 1 },
      { kind: "player", id: -1 },
      { kind: "monster", id: 1.5 },
      { kind: "monster", id: Number.MAX_SAFE_INTEGER },
      null,
    ])
      expect(audioEntity(party)).toBeNull();
  });

  test("each 67a cue kind maps to its manifest kind for the local player", () => {
    const me = { kind: "player", id: 1 };
    const other = { kind: "player", id: 2 };
    const brute = { kind: "monster", id: 1 };
    const cases = [
      [{ kind: "hit", source: me, target: brute }, "combat.hit"],
      [{ kind: "hit", source: me, target: brute, defeated: true }, "combat.hit.defeated"],
      [{ kind: "hit", source: brute, target: me }, "combat.hurt"],
      [{ kind: "hit", source: brute, target: other }, "combat.hit"],
      [{ kind: "blocked", source: brute, target: me }, "combat.blocked"],
      [
        { kind: "blocked", source: me, target: brute, definition: "bow.arrow" },
        "combat.arrowBlocked",
      ],
      [{ kind: "guardBroken", source: brute, target: me }, "combat.guardBroken"],
      [{ kind: "obstructed", source: me, target: brute }, "combat.obstructed"],
      [{ kind: "whiff", playerId: 1 }, "combat.whiff"],
      [{ kind: "defeat", target: me }, "combat.defeat"],
      [{ kind: "defeat", target: other }, null],
      [{ kind: "defeat", target: brute }, null],
      [{ kind: "pickedUp", playerId: 2, gold: 5 }, "world.pickedUp.gold"],
      [{ kind: "pickedUp", playerId: 1, gold: 0 }, null],
      [{ kind: "opened", playerId: 1, gold: 3 }, "world.opened"],
      [{ kind: "refused", playerId: 1 }, "world.refused"],
      [{ kind: "refused", playerId: 2 }, null],
      [{ kind: "exited", playerId: 1 }, null],
      [{ kind: "arrowRelease", playerId: 1 }, "combat.arrowRelease"],
      [{ kind: "arrowLanded", playerId: 1 }, "combat.arrowLanded"],
      [{ kind: "counterOpen", playerId: 1 }, "combat.counterOpen"],
      [{ kind: "counterOpen", playerId: 2 }, null],
      [{ kind: "counterUsed", playerId: 2 }, "combat.counterUsed"],
      [{ kind: "counterExpired", playerId: 1 }, null],
      [{ kind: "counterLost", playerId: 1 }, null],
      [{ kind: "encounter", state: "active" }, "encounter.start"],
      [{ kind: "encounter", state: "cleared" }, "encounter.clear"],
      [{ kind: "encounter", state: "dormant" }, null],
      [{ kind: "somethingNew" }, null],
    ];
    for (const [cue, kind] of cases) expect([cue, soundKind(cue, 1)]).toEqual([cue, kind]);
  });

  test("a sound follows its entity and is placed in metres", () => {
    const snapshot = frame(5);
    const brute = { kind: "monster", id: 1 };
    expect(
      cueSound(
        {
          key: "k",
          kind: "hit",
          source: { kind: "player", id: 1 },
          target: brute,
          position: [100, 0, 200],
        },
        { playerId: 1, snapshot, epoch: 3 },
      ),
    ).toEqual({ key: "3:k", kind: "combat.hit", entity: 3, position: [1, 0, 2] });
    // Without a cue position, the entity's snapshot position.
    expect(cueSound({ key: "w", kind: "whiff", playerId: 1 }, { playerId: 1, snapshot })).toEqual({
      key: "0:w",
      kind: "combat.whiff",
      entity: 2,
      position: [0, 0, 0],
    });
    // A world arrow impact sounds where it landed, attached to nothing.
    expect(
      cueSound(
        { key: "a", kind: "arrowLanded", playerId: 1, position: [500, 0, 0] },
        { playerId: 1, snapshot },
      ),
    ).toEqual({ key: "0:a", kind: "combat.arrowLanded", entity: null, position: [5, 0, 0] });
    expect(cueSound({ key: "x", kind: "exited", playerId: 1 }, { playerId: 1, snapshot })).toBe(
      null,
    );
  });

  test("the listener stands on the camera target with its right ear along screen-right", () => {
    const { position, right } = listenerPose({ target: [2, 0, 3], position: [12, 11, 13] });
    expect(position).toEqual([2, 0, 3]);
    expect(right[0]).toBeCloseTo(Math.SQRT1_2);
    expect(right[1]).toBe(0);
    expect(right[2]).toBeCloseTo(-Math.SQRT1_2);
  });
});

describe("cue audio stream", () => {
  test("a cue sounds once; a replayed or late snapshot never sounds again", () => {
    const { cues, audio } = setup();
    cues.observe(frame(4), 1);
    const event = frame(5, { strikeEvents: [hit()] });
    cues.observe(event, 1);
    cues.observe({ ...event }, 1);
    cues.observe(frame(3, { strikeEvents: [hit()] }), 1);
    cues.observe(frame(6, { strikeEvents: [hit()] }), 1);
    expect(audio.triggered()).toEqual([
      { key: "1:strike:player:1:5:monster:1", kind: "combat.hit", entity: 3, position: [3, 0, 4] },
    ]);
  });

  test("nothing sounds, or queues, until the audio context runs", () => {
    const { cues, audio } = setup("suspended");
    cues.observe(frame(4), 1);
    cues.observe(frame(5, { strikeEvents: [hit()] }), 1);
    audio.current = "running";
    cues.observe(frame(6), 1);
    expect(audio.triggered()).toEqual([]);
    cues.observe(frame(7, { strikeEvents: [hit({ strikeTick: 7 })] }), 1);
    expect(audio.triggered()).toHaveLength(1);
  });

  test("unavailable audio stays silent and the cue projection is unchanged", () => {
    const { cues, audio } = setup("unavailable");
    cues.observe(frame(4), 1);
    expect(cues.observe(frame(5, { strikeEvents: [hit()] }), 1)).toHaveLength(1);
    expect(cues.getCues()).toHaveLength(1);
    expect(audio.triggered()).toEqual([]);
  });

  test("a new source resets the director, and a restarted run's identical cues sound again", () => {
    const { cues, audio } = setup();
    cues.observe(frame(4), 1);
    cues.observe(frame(5, { strikeEvents: [hit()] }), 1);
    const resets = audio.calls.filter(([name]) => name === "reset").length;
    // Restart with the same seed: a new source replays the same ticks.
    cues.observe(frame(4), 2);
    expect(audio.calls.filter(([name]) => name === "reset").length).toBe(resets + 1);
    cues.observe(frame(5, { strikeEvents: [hit()] }), 2);
    const keys = audio.triggered().map((cue) => cue.key);
    expect(keys).toHaveLength(2);
    expect(new Set(keys).size).toBe(2);
  });

  test("entities are tracked in metres and their voices stop when they leave", () => {
    const { cues, audio } = setup();
    cues.observe(frame(4), 1);
    cues.observe(frame(5), 1);
    expect(audio.calls).toContainEqual(["setEntityPosition", 3, [3, 0, 4]]);
    cues.observe(frame(6, { monsters: [] }), 1);
    expect(audio.calls).toContainEqual(["stopEntity", 3]);
    expect(audio.calls).not.toContainEqual(["stopEntity", 2]);
  });

  test("each frame moves the listener and pumps; dispose detaches", () => {
    const { cues, audio, attached } = setup();
    attached.frame({ target: [0, 0, 0], position: [10, 11, 10] });
    expect(audio.calls.at(-2)[0]).toBe("setListener");
    expect(audio.calls.at(-1)).toEqual(["pump"]);
    attached.dispose();
    cues.observe(frame(4), 1);
    cues.observe(frame(5, { strikeEvents: [hit()] }), 1);
    expect(audio.triggered()).toEqual([]);
  });
});
