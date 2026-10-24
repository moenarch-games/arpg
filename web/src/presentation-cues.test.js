import { describe, expect, test } from "bun:test";
import { attachPresentationCues, createPresentationCues } from "./presentation-cues.js";
import { createSnapshotStore } from "./snapshot-store.js";

const player = (patch = {}) => ({
  id: 1,
  position: [0, 0, 0],
  health: 100,
  maxHealth: 100,
  alive: true,
  action: null,
  counter: null,
  ...patch,
});
const monster = (patch = {}) => ({
  id: 7,
  definition: "monster.brute",
  position: [100, 0, 0],
  health: 60,
  maxHealth: 60,
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
const swing = (phase, connected = false) => ({
  kind: "primaryAttack",
  phase,
  ticksRemaining: 3,
  phaseTicks: 4,
  connected,
  buffered: null,
});
const strike = (result, patch = {}) => ({
  order: 0,
  source: { kind: "player", id: 1 },
  strikeTick: 5,
  definition: "sword.lightSwing",
  target: { kind: "monster", id: 7 },
  result,
  ...patch,
});
const hit = (damage = 25) => strike({ kind: "hit", damage, defeated: false });

const kinds = (cues) => cues.map((cue) => cue.kind);

describe("presentation cues", () => {
  test("the first snapshot of a source is a baseline and emits nothing", () => {
    const cues = createPresentationCues();
    expect(cues.observe(frame(5, { strikeEvents: [hit()] }))).toEqual([]);
    expect(cues.getCues()).toEqual([]);
  });

  test("a strike becomes one keyed cue with its authoritative damage and position", () => {
    const cues = createPresentationCues();
    cues.observe(frame(4));
    const [cue] = cues.observe(frame(5, { strikeEvents: [hit(25)] }));
    expect(cue).toMatchObject({
      key: "strike:player:1:5:monster:7",
      kind: "hit",
      damage: 25,
      position: [100, 0, 0],
    });
  });

  test("a replayed or late snapshot never repeats a cue", () => {
    const cues = createPresentationCues();
    cues.observe(frame(4));
    const event = frame(5, { strikeEvents: [hit()] });
    expect(cues.observe(event)).toHaveLength(1);
    // The same tick again (a reconnect replay) and an older one (a late datagram).
    expect(cues.observe({ ...event })).toEqual([]);
    expect(cues.observe(frame(3, { strikeEvents: [hit()] }))).toEqual([]);
    // The same strike reported in a later tick of its active window is the same cue.
    expect(cues.observe(frame(6, { strikeEvents: [hit()] }))).toEqual([]);
    expect(cues.getCues()).toHaveLength(1);
  });

  test("a missed swing is a whiff, a blocked one is not, and neither has damage", () => {
    const whiff = createPresentationCues();
    whiff.observe(frame(4, { players: [player({ action: swing("active") })] }));
    const missed = whiff.observe(frame(5, { players: [player({ action: swing("recovery") })] }));
    expect(kinds(missed)).toEqual(["whiff"]);
    expect(missed[0].damage).toBeUndefined();
    // Later recovery snapshots of the same swing do not repeat it.
    expect(
      whiff.observe(
        frame(6, { players: [player({ action: { ...swing("recovery"), ticksRemaining: 2 } })] }),
      ),
    ).toEqual([]);

    const blocked = createPresentationCues();
    blocked.observe(frame(4, { players: [player({ action: swing("windup") })] }));
    const block = blocked.observe(
      frame(5, {
        players: [player({ action: swing("active") })],
        strikeEvents: [strike({ kind: "blocked", guardDamage: 10 })],
      }),
    );
    expect(kinds(block)).toEqual(["blocked"]);
    expect(blocked.observe(frame(6, { players: [player({ action: swing("recovery") })] }))).toEqual(
      [],
    );
  });

  test("counter opportunities report use, expiry and loss distinctly", () => {
    const counter = {
      blockedMonsterId: 7,
      blockedAtTick: 10,
      usableFromTick: 11,
      expiresAtTick: 41,
    };
    const open = (tick) => frame(tick, { players: [player({ counter })] });

    const used = createPresentationCues();
    used.observe(frame(9));
    expect(kinds(used.observe(open(10)))).toEqual(["counterOpen"]);
    expect(
      kinds(
        used.observe(
          frame(12, { players: [player({ action: { ...swing("windup"), kind: "counter" } })] }),
        ),
      ),
    ).toEqual(["counterUsed"]);

    const expired = createPresentationCues();
    expired.observe(open(40));
    expect(kinds(expired.observe(frame(41)))).toEqual(["counterExpired"]);

    const lost = createPresentationCues();
    lost.observe(open(20));
    expect(kinds(lost.observe(frame(21)))).toEqual(["counterLost"]);
  });

  test("arrows report release and a world impact, but not an impact that struck a target", () => {
    const arrow = { id: 3, source: { kind: "player", id: 1 }, position: [50, 40, 0] };
    const cues = createPresentationCues();
    cues.observe(frame(1));
    expect(kinds(cues.observe(frame(2, { arrows: [arrow] })))).toEqual(["arrowRelease"]);
    expect(kinds(cues.observe(frame(3)))).toEqual(["arrowLanded"]);

    const struck = createPresentationCues();
    struck.observe(frame(1, { arrows: [arrow] }));
    const impact = struck.observe(
      frame(2, { strikeEvents: [{ ...hit(), definition: "bow.arrow", strikeTick: 2 }] }),
    );
    expect(kinds(impact)).toEqual(["hit"]);
  });

  test("defeats, pickups and encounter changes follow authoritative transitions", () => {
    const cues = createPresentationCues();
    cues.observe(frame(1));
    const found = cues.observe(
      frame(2, {
        monsters: [monster({ alive: false, health: 0 })],
        rooms: [{ id: 1, encounterState: "cleared" }],
        interactionEvents: [
          {
            order: 1,
            playerId: 1,
            result: { kind: "pickedUp", target: { kind: "loot", id: 2 }, gold: 5 },
          },
        ],
      }),
    );
    expect(kinds(found)).toEqual(["pickedUp", "defeat", "encounter"]);
    expect(found[2].state).toBe("cleared");
  });

  test("overflow drops cosmetic cues beyond the per-frame cap and keeps a bounded history", () => {
    const cues = createPresentationCues({ frameLimit: 2, capacity: 3 });
    cues.observe(frame(1));
    const many = (tick) =>
      frame(tick, {
        strikeEvents: [1, 2, 3].map((id) =>
          strike(
            { kind: "hit", damage: id, defeated: false },
            {
              order: id,
              strikeTick: tick,
              target: { kind: "crate", id },
            },
          ),
        ),
      });
    expect(cues.observe(many(2)).map((cue) => cue.damage)).toEqual([1, 2]);
    expect(cues.stats().dropped).toBe(1);
    cues.observe(many(3));
    expect(cues.getCues().map((cue) => `${cue.tick}:${cue.damage}`)).toEqual(["2:2", "3:1", "3:2"]);
  });

  test("cues expire after their lifetime in authoritative ticks", () => {
    const cues = createPresentationCues({ lifetimeTicks: 10 });
    cues.observe(frame(4));
    cues.observe(frame(5, { strikeEvents: [hit()] }));
    cues.observe(frame(14));
    expect(cues.getCues()).toHaveLength(1);
    cues.observe(frame(15));
    expect(cues.getCues()).toEqual([]);
  });

  test("cues about a monster or player that left the snapshot are dropped", () => {
    const cues = createPresentationCues();
    cues.observe(frame(4));
    cues.observe(frame(5, { strikeEvents: [hit()] }));
    cues.observe(frame(6, { monsters: [] }));
    expect(cues.getCues()).toEqual([]);
  });

  test("a new source, a cleared store or another run resets the projection", () => {
    const store = createSnapshotStore();
    const cues = createPresentationCues();
    const detach = attachPresentationCues(store, cues);
    store.publish(frame(4));
    store.publish(frame(5, { strikeEvents: [hit()] }));
    expect(cues.getCues()).toHaveLength(1);

    // Restart/restore (a new source) at a later tick: a baseline, not a replay.
    store.beginSource();
    store.publish(frame(9, { strikeEvents: [hit()] }));
    expect(cues.getCues()).toEqual([]);
    // After the reset the same strike identity may play again on the new timeline.
    store.publish(frame(10, { strikeEvents: [hit()] }));
    expect(cues.getCues()).toHaveLength(1);

    // Another run seed in the same source (a seek into another run) also resets.
    store.publish(frame(11, { runSeed: 7 }));
    expect(cues.getCues()).toEqual([]);

    store.publish(null);
    store.publish(frame(1));
    expect(cues.observe(frame(1))).toEqual([]);
    detach();
    store.publish(frame(2, { strikeEvents: [hit()] }));
    expect(cues.getCues()).toEqual([]);
  });

  test("cues never change the snapshot they come from", () => {
    const cues = createPresentationCues();
    const before = frame(4, { players: [player({ action: swing("active") })] });
    const after = frame(5, { players: [player({ action: swing("recovery") })], strikeEvents: [] });
    const copy = structuredClone(after);
    cues.observe(before);
    cues.observe(after);
    expect(after).toEqual(copy);
  });
});
