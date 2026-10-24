import { expect, test } from "bun:test";
import { PROTOCOL_VERSION, contentRevisionMismatch, decodeSnapshot } from "./wire-protocol.js";

const payload = {
  tick: 0,
  runSeed: 42,
  worldUnitsPerMeter: 100,
  players: [],
  monsters: [],
  rooms: [],
  staticColliders: [],
  groundLoot: [],
  arrows: [],
  scenario: "dungeon",
  contentRevision: "0123456789abcdef",
  strikeEvents: [],
  chests: [],
  interactionEvents: [],
};
const encode = (value) => JSON.stringify({ protocolVersion: PROTOCOL_VERSION, payload: value });

test("admits bounded presentation data and rejects malformed vectors and collections", () => {
  expect(decodeSnapshot(encode(payload))).toEqual(payload);
  for (const candidate of [
    { ...payload, players: {} },
    { ...payload, worldUnitsPerMeter: 0 },
    { ...payload, tick: -1 },
    { ...payload, runSeed: 0x100000000 },
    { ...payload, monsters: [{ id: 1, alive: true, position: [0, 0] }] },
    {
      ...payload,
      staticColliders: [{ id: 1, kind: "wall", position: [0, 0, 0], halfExtents: [1, -1, 1] }],
    },
  ])
    expect(() => decodeSnapshot(encode(candidate))).toThrow();
  expect(() => decodeSnapshot(" ".repeat(65_536))).toThrow("size");
});

const player = {
  id: 1,
  position: [0, 50, 0],
  facing: [1, 0],
  alive: true,
  health: 100,
  maxHealth: 100,
  level: 1,
  experienceIntoLevel: 0,
  experienceForNextLevel: 100,
  attackDamage: 25,
  gold: 0,
  guard: { phase: "raised", ticksRemaining: 0 },
  guardPoints: 70,
  maxGuardPoints: 100,
  reaction: { kind: "blocked", ticksRemaining: 8 },
  counter: { blockedMonsterId: 3, blockedAtTick: 9, usableFromTick: 10, expiresAtTick: 40 },
  action: {
    kind: "lightFollowUp",
    phase: "recovery",
    ticksRemaining: 5,
    phaseTicks: 8,
    facing: [1, 0],
    connected: true,
    buffered: "heavy",
    charge: 0,
  },
  weapon: "bow",
  drawTicks: 12,
  interaction: { kind: "available", target: { kind: "chest", id: 50002 } },
  motion: { grounded: false, verticalVelocity: -3, feetY: 42, groundY: 0 },
};

test("admits a player's authoritative height and support and rejects malformed motion", () => {
  const decoded = decodeSnapshot(encode({ ...payload, players: [player] })).players[0];
  expect(decoded.motion).toEqual({ grounded: false, verticalVelocity: -3, feetY: 42, groundY: 0 });
  // No ground below the body is published as null.
  expect(() =>
    decodeSnapshot(
      encode({ ...payload, players: [{ ...player, motion: { ...player.motion, groundY: null } }] }),
    ),
  ).not.toThrow();
  for (const motion of [
    undefined,
    null,
    { ...player.motion, grounded: "no" },
    { ...player.motion, verticalVelocity: 0.5 },
    { ...player.motion, feetY: undefined },
    { ...player.motion, groundY: "floor" },
  ]) {
    expect(() => decodeSnapshot(encode({ ...payload, players: [{ ...player, motion }] }))).toThrow(
      "Malformed ARPG presentation snapshot",
    );
  }
});

test("admits region membership and platforms and rejects malformed ones", () => {
  for (const region of [
    { kind: "region", id: 2 },
    { kind: "transit", connection: 1 },
  ]) {
    const decoded = decodeSnapshot(encode({ ...payload, players: [{ ...player, region }] }));
    expect(decoded.players[0].region).toEqual(region);
  }
  for (const region of [{ kind: "region" }, { kind: "transit", connection: -1 }, { kind: "air" }])
    expect(() =>
      decodeSnapshot(encode({ ...payload, players: [{ ...player, region }] })),
    ).toThrow();
  const platform = { id: 9, kind: "platform", position: [0, 30, 0], halfExtents: [100, 30, 100] };
  const ceiling = { id: 10, kind: "ceiling", position: [0, 160, 0], halfExtents: [100, 20, 100] };
  const step = { id: 11, kind: "step", position: [0, 10, 0], halfExtents: [60, 10, 20] };
  const ramp = { id: 12, kind: "ramp", position: [0, 30, 190], halfExtents: [40, 30, 90] };
  const decoded = decodeSnapshot(
    encode({ ...payload, scenario: "traversal", staticColliders: [platform, ceiling, step, ramp] }),
  );
  expect(decoded.staticColliders).toEqual([platform, ceiling, step, ramp]);
});

test("admits a slowed monster and rejects a malformed slow", () => {
  const slowed = { ...monster, slow: { ticksRemaining: 90 } };
  expect(decodeSnapshot(encode({ ...payload, monsters: [slowed] })).monsters[0].slow).toEqual({
    ticksRemaining: 90,
  });
  expect(() =>
    decodeSnapshot(
      encode({ ...payload, monsters: [{ ...monster, slow: { ticksRemaining: -1 } }] }),
    ),
  ).toThrow();
});

test("admits poison pools and poisoned players and rejects malformed ones", () => {
  const hazard = { id: 50000, roomId: 2, position: [0, 0, 0], radius: 60 };
  const poison = { source: 50000, ticksRemaining: 179, nextDamageIn: 29 };
  const decoded = decodeSnapshot(
    encode({ ...payload, hazards: [hazard], players: [{ ...player, poison }] }),
  );
  expect(decoded.hazards).toEqual([hazard]);
  expect(decoded.players[0].poison).toEqual(poison);
  for (const candidate of [
    { ...payload, hazards: [{ ...hazard, radius: -1 }] },
    { ...payload, players: [{ ...player, poison: { ...poison, ticksRemaining: "a" } }] },
  ])
    expect(() => decodeSnapshot(encode(candidate))).toThrow();
});

test("admits damageable crates and strikes on them and rejects malformed ones", () => {
  const crate = {
    id: 40000,
    roomId: 2,
    position: [0, 50, 0],
    halfExtents: [25, 50, 25],
    health: 5,
    maxHealth: 30,
  };
  const strike = {
    order: 0,
    source: { kind: "player", id: 1 },
    target: { kind: "crate", id: 40000 },
    strikeTick: 3,
    definition: "sword.lightSwing",
    result: { kind: "hit", damage: 25, defeated: false },
  };
  const collider = { id: 40000, kind: "crate", position: [0, 50, 0], halfExtents: [25, 50, 25] };
  const decoded = decodeSnapshot(
    encode({ ...payload, crates: [crate], strikeEvents: [strike], staticColliders: [collider] }),
  );
  expect(decoded.crates).toEqual([crate]);
  expect(decoded.strikeEvents[0].target).toEqual({ kind: "crate", id: 40000 });
  for (const malformed of [
    { ...crate, health: 31 },
    { ...crate, position: [0, 0] },
  ])
    expect(() => decodeSnapshot(encode({ ...payload, crates: [malformed] }))).toThrow();
});

test("admits shield guard state and rejects malformed guard data", () => {
  expect(decodeSnapshot(encode({ ...payload, players: [player] })).players[0].guard.phase).toBe(
    "raised",
  );
  for (const candidate of [
    { ...player, guard: { phase: "lowered", ticksRemaining: 0 } },
    { ...player, guardPoints: 101 },
    { ...player, guardPoints: undefined },
    { ...player, reaction: { kind: "parried", ticksRemaining: 1 } },
    { ...player, counter: { usableFromTick: 40, expiresAtTick: 10 } },
    { ...player, action: { ...player.action, buffered: "special" } },
    { ...player, action: { ...player.action, connected: undefined } },
    { ...player, weapon: "spear" },
    { ...player, drawTicks: -1 },
    { ...player, interaction: { kind: "available", target: { kind: "door", id: 1 } } },
    { ...player, interaction: { kind: "unavailable", reason: "tooTired" } },
  ])
    expect(() => decodeSnapshot(encode({ ...payload, players: [candidate] }))).toThrow();
});

test("admits aim intent and a target lock and rejects malformed ones", () => {
  const aimed = {
    ...player,
    aim: [-250, 1000],
    lockedMonsterId: 3,
    action: { ...player.action, aim: [927, -375] },
  };
  const decoded = decodeSnapshot(encode({ ...payload, players: [aimed] })).players[0];
  expect(decoded.aim).toEqual([-250, 1000]);
  expect(decoded.lockedMonsterId).toBe(3);
  expect(decoded.action.aim).toEqual([927, -375]);
  // Absent and null both mean "no aim" and "no lock".
  expect(
    decodeSnapshot(
      encode({ ...payload, players: [{ ...player, aim: null, lockedMonsterId: null }] }),
    ).players[0].aim,
  ).toBeNull();
  for (const candidate of [
    { ...player, aim: [0, 0] },
    { ...player, aim: [1001, 0] },
    { ...player, aim: [0.5, 1] },
    { ...player, aim: [1] },
    { ...player, lockedMonsterId: -1 },
    { ...player, action: { ...player.action, aim: [0, 2000] } },
  ])
    expect(() => decodeSnapshot(encode({ ...payload, players: [candidate] }))).toThrow();
});

test("admits workbench scenario and strike events and rejects malformed ones", () => {
  const event = {
    order: 0,
    source: { kind: "player", id: 1 },
    strikeTick: 4,
    definition: "sword.lightSwing",
    target: { kind: "monster", id: 2 },
    result: { kind: "hit", damage: 25, defeated: false },
  };
  const decoded = decodeSnapshot(
    encode({ ...payload, scenario: "archery", strikeEvents: [event] }),
  );
  expect(decoded.strikeEvents[0].result.kind).toBe("hit");
  for (const candidate of [
    { ...payload, scenario: "lava" },
    { ...payload, strikeEvents: [{ ...event, source: { kind: "ghost", id: 1 } }] },
    { ...payload, strikeEvents: [{ ...event, result: { kind: "parried" } }] },
    { ...payload, strikeEvents: [{ ...event, definition: "x".repeat(65) }] },
  ])
    expect(() => decodeSnapshot(encode(candidate))).toThrow();
});

test("admits chests and interaction results and rejects malformed ones", () => {
  const chest = { id: 50002, roomId: 2, position: [0, 50, 0], opened: false, available: true };
  const picked = {
    order: 0,
    playerId: 1,
    result: { kind: "pickedUp", target: { kind: "loot", id: 30000 }, gold: 10 },
  };
  const refused = { order: 1, playerId: 2, result: { kind: "refused", reason: "busy" } };
  const decoded = decodeSnapshot(
    encode({ ...payload, chests: [chest], interactionEvents: [picked, refused] }),
  );
  expect(decoded.interactionEvents).toHaveLength(2);
  for (const candidate of [
    { ...payload, chests: [{ ...chest, opened: "yes" }] },
    { ...payload, interactionEvents: [{ ...picked, result: { ...picked.result, gold: -1 } }] },
    { ...payload, interactionEvents: [{ ...refused, result: { kind: "refused", reason: "x" } }] },
  ])
    expect(() => decodeSnapshot(encode(candidate))).toThrow();
});

test("admits the run's exit and state (#66) and rejects malformed ones", () => {
  const exit = { id: 60000, roomId: 1, position: [0, 50, 0], available: false };
  const exited = {
    order: 0,
    playerId: 1,
    result: { kind: "exited", target: { kind: "exit", id: 60000 } },
  };
  const decoded = decodeSnapshot(
    encode({ ...payload, exit, runState: "completed", interactionEvents: [exited] }),
  );
  expect(decoded.runState).toBe("completed");
  for (const candidate of [
    { ...payload, exit: { ...exit, available: "no" } },
    { ...payload, exit, runState: "abandoned" },
    { ...payload, interactionEvents: [{ ...exited, result: { kind: "exited", target: null } }] },
  ])
    expect(() => decodeSnapshot(encode(candidate))).toThrow();
});

test("rooms publish their encounter definition and malformed ones are rejected", () => {
  const room = { id: 2, minX: 0, maxX: 100, minZ: 0, maxZ: 100, kind: "combat" };
  const start = { ...room, id: 1, kind: "start", encounter: null };
  const combat = { ...room, encounter: "encounter.brute" };
  const decoded = decodeSnapshot(encode({ ...payload, rooms: [start, combat] }));
  expect(decoded.rooms[0].encounter).toBeNull();
  expect(decoded.rooms[1].encounter).toBe("encounter.brute");
  for (const encounter of ["", "e".repeat(65), 7])
    expect(() => decodeSnapshot(encode({ ...payload, rooms: [{ ...room, encounter }] }))).toThrow();
});

test("requires the authority's content revision", () => {
  for (const contentRevision of [undefined, "", "XYZ", "0123456789abcdef0"])
    expect(() => decodeSnapshot(encode({ ...payload, contentRevision }))).toThrow();
});

const monster = {
  id: 1,
  definition: "monster.brute",
  roomId: 2,
  position: [0, 50, 0],
  health: 100,
  maxHealth: 100,
  alive: true,
  action: null,
  reaction: null,
  behavior: "pursuing",
  targetPlayerId: 1,
  guard: null,
  guardPoints: 0,
  maxGuardPoints: 0,
};

test("shield monsters publish their guard like players and malformed guards are rejected", () => {
  const defender = {
    ...monster,
    definition: "monster.defender",
    health: 140,
    maxHealth: 140,
    guard: { phase: "raised", ticksRemaining: 0 },
    guardPoints: 80,
    maxGuardPoints: 100,
  };
  const decoded = decodeSnapshot(encode({ ...payload, monsters: [monster, defender] }));
  expect(decoded.monsters[0].guard).toBeNull();
  expect(decoded.monsters[1].guard).toEqual({ phase: "raised", ticksRemaining: 0 });
  expect(decoded.monsters[1].guardPoints).toBe(80);
  for (const candidate of [
    { ...defender, guard: { phase: "lowered", ticksRemaining: 0 } },
    { ...defender, guardPoints: 101 },
    { ...defender, guardPoints: undefined },
    { ...defender, maxGuardPoints: -1 },
  ])
    expect(() => decodeSnapshot(encode({ ...payload, monsters: [candidate] }))).toThrow();
});

test("monsters publish their authoritative behaviour and engaged target", () => {
  const decoded = decodeSnapshot(encode({ ...payload, monsters: [monster] }));
  expect(decoded.monsters[0].behavior).toBe("pursuing");
  expect(decoded.monsters[0].targetPlayerId).toBe(1);
  for (const behavior of [
    "retreating",
    "returning",
    "searching",
    "holding",
    "idle",
    "dormant",
    "dead",
  ])
    expect(
      decodeSnapshot(
        encode({ ...payload, monsters: [{ ...monster, behavior, targetPlayerId: null }] }),
      ).monsters[0].behavior,
    ).toBe(behavior);
  for (const candidate of [
    { ...monster, behavior: undefined },
    { ...monster, behavior: "fleeing" },
    { ...monster, targetPlayerId: -1 },
    { ...monster, definition: undefined },
    { ...monster, definition: "" },
    { ...monster, definition: "m".repeat(65) },
    { ...monster, maxHealth: undefined },
    { ...monster, health: 101 },
  ])
    expect(() => decodeSnapshot(encode({ ...payload, monsters: [candidate] }))).toThrow();
  expect(() =>
    decodeSnapshot(
      JSON.stringify({ protocolVersion: 16, payload: { ...payload, monsters: [monster] } }),
    ),
  ).toThrow("Unsupported");
});

test("monsters publish their content definition and maximum health", () => {
  const skirmisher = { ...monster, definition: "monster.skirmisher", health: 60, maxHealth: 60 };
  const decoded = decodeSnapshot(encode({ ...payload, monsters: [monster, skirmisher] }));
  expect(decoded.monsters.map(({ definition }) => definition)).toEqual([
    "monster.brute",
    "monster.skirmisher",
  ]);
  expect(decoded.monsters[1].maxHealth).toBe(60);
});

test("monster attacks publish how they land and shots share the arrow list", () => {
  const shooting = {
    ...monster,
    definition: "monster.archer",
    health: 50,
    maxHealth: 50,
    behavior: "attacking",
    action: {
      phase: "windup",
      ticksRemaining: 24,
      targetPlayerId: 1,
      range: 640,
      delivery: "projectile",
    },
  };
  const shot = {
    id: 3,
    source: { kind: "monster", id: 1 },
    launchedAtTick: 24,
    position: [500, 50, 0],
    velocity: [-20, 0, 0],
    damage: 8,
    ticksRemaining: 40,
  };
  const arrow = { ...shot, id: 4, source: { kind: "player", id: 1 } };
  const decoded = decodeSnapshot(
    encode({ ...payload, monsters: [shooting], arrows: [shot, arrow], scenario: "ranged" }),
  );
  expect(decoded.monsters[0].action.delivery).toBe("projectile");
  expect(decoded.arrows.map(({ source }) => source.kind)).toEqual(["monster", "player"]);
  expect(decodeSnapshot(encode({ ...payload, scenario: "heavy" })).scenario).toBe("heavy");
  // The bruiser's ground burst (#64) is its own delivery.
  const bursting = { ...shooting, action: { ...shooting.action, range: 220, delivery: "burst" } };
  expect(
    decodeSnapshot(encode({ ...payload, monsters: [bursting] })).monsters[0].action.delivery,
  ).toBe("burst");
  for (const candidate of [
    { ...payload, monsters: [{ ...shooting, action: { ...shooting.action, delivery: "beam" } }] },
    {
      ...payload,
      monsters: [{ ...shooting, action: { ...shooting.action, delivery: undefined } }],
    },
    { ...payload, arrows: [{ ...shot, source: undefined, ownerId: 1 }] },
    { ...payload, arrows: [{ ...shot, source: { kind: "trap", id: 1 } }] },
  ])
    expect(() => decodeSnapshot(encode(candidate))).toThrow();
});

test("a content revision mismatch names both revisions; a match passes", () => {
  expect(contentRevisionMismatch("0123456789abcdef", payload, "host")).toBeNull();
  expect(contentRevisionMismatch("fedcba9876543210", payload, "dedicated authority")).toBe(
    "Incompatible game content: the dedicated authority runs content revision " +
      "0123456789abcdef, this client runs fedcba9876543210. Both sides need the same game build.",
  );
});
