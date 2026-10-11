// Dedicated one-shot commands are delivered reliably (#143).
//
// The client runtime sends every one-shot command on the dedicated session's reliable
// command lane and every held-state command as a datagram. Held input keeps the #139
// refresh; one-shots are never repeated by it. This drives the real client runtime with a
// modelled dedicated session whose authority keeps one watermark per lane, as
// `game_server::MatchRuntime` does.
//
// API contract (web/src/game-client-runtime.ts):
//
// - `DedicatedSessionLike` gains `sendReliableCommand(sequence, payload): Promise<unknown>`
//   (implemented by `DedicatedGameSession`, see reliable-command-lane.test.js). It
//   resolves once the authority acknowledged the command and rejects for a `Rejected` ack
//   or a closed session.
// - In dedicated mode, `dispatch` sends the one-shot types `primaryAttack`,
//   `secondaryAttack`, `interact`, `equipWeapon`, `cycleTarget` and `clearTarget` with
//   `sendReliableCommand`, and so the bow transitions `drawBow`, `releaseBow` and
//   `cancelBow`: a release consumes its draw, so a lost release followed by a new draw
//   would merge two shots into one. The held types `setMovement`, `setAim` and `setGuard`
//   (including every refresh) keep using `sendCommand`.
// - Both lanes share the runtime's one sequence counter, so sequences strictly increase in
//   send order across lanes.
// - A rejected one-shot surfaces as a status message containing "rejected" and leaves the
//   session running.
import { afterEach, beforeEach, expect, jest, test } from "bun:test";
import { createGameClientRuntime, TICK_INTERVAL_MS } from "./game-client-runtime.ts";
import { createSnapshotStore } from "./snapshot-store.js";
import { PROTOCOL_VERSION } from "./wire-protocol.js";

const LOCAL_REVISION = "0123456789abcdef";
const PLAYER_ID = 1;
const CONVERGENCE_BOUND_MS = 500;
const ONE_SHOTS = [
  { type: "primaryAttack" },
  { type: "secondaryAttack" },
  { type: "interact" },
  { type: "equipWeapon", weapon: "bow" },
  { type: "cycleTarget" },
  { type: "clearTarget" },
];
const ONE_SHOT_TYPES = new Set(ONE_SHOTS.map((command) => command.type));
const HELD_TYPES = new Set(["setGuard", "setMovement", "setAim"]);
// Bow transitions travel on the reliable lane too, but are not one-shot actions.
const BOW_TYPES = new Set(["drawBow", "releaseBow", "cancelBow"]);

const snapshotJson = (tick) =>
  JSON.stringify({
    protocolVersion: PROTOCOL_VERSION,
    payload: {
      tick,
      runSeed: 7,
      worldUnitsPerMeter: 100,
      players: [
        {
          id: PLAYER_ID,
          position: [0, 0, 0],
          facing: [0, 1],
          alive: true,
          health: 10,
          maxHealth: 10,
          level: 1,
          experienceIntoLevel: 0,
          experienceForNextLevel: 100,
          attackDamage: 1,
          gold: 0,
          guardPoints: 100,
          maxGuardPoints: 100,
          weapon: "swordAndShield",
          interaction: { kind: "unavailable", reason: "nothingInRange" },
        },
      ],
      monsters: [],
      rooms: [],
      staticColliders: [],
      groundLoot: [],
      arrows: [],
      scenario: "dungeon",
      contentRevision: LOCAL_REVISION,
      strikeEvents: [],
      chests: [],
      interactionEvents: [],
    },
  });

const decodeCommand = (payload) => JSON.parse(new TextDecoder().decode(payload)).payload;

async function flush(rounds = 25) {
  for (let round = 0; round < rounds; round += 1) await Promise.resolve();
}

// `MatchRuntime` + `arpg-core` for this player: one watermark per lane.
class LaneAuthority {
  watermarks = { datagram: 0, reliable: 0 };
  applied = [];
  guard = false;
  movement = [0, 0];

  receive(lane, sequence, command) {
    if (sequence <= this.watermarks[lane]) return "duplicate";
    this.watermarks[lane] = sequence;
    this.applied.push({ lane, sequence, command });
    if (command.type === "setGuard") this.guard = command.raised;
    if (command.type === "setMovement") this.movement = [command.x, command.z];
    return "applied";
  }

  oneShots() {
    return this.applied.filter((entry) => ONE_SHOT_TYPES.has(entry.command.type));
  }
}

class ModelDedicatedSession {
  callbacks = null;
  sent = [];
  // Datagram delivery: "deliver", "lose" (every datagram) or "hold" (delayed past later
  // reliable traffic, i.e. reordered).
  datagrams = "deliver";
  heldBack = [];
  // Reliable delivery: acknowledged immediately unless `reliableDelayed`, and rejected
  // for the types in `rejectTypes`.
  reliableDelayed = false;
  pendingReliable = [];
  rejectTypes = new Set();
  closed = false;

  constructor(authority) {
    this.authority = authority;
  }

  async connect(callbacks) {
    this.callbacks = callbacks;
    callbacks.onStateChange?.("connecting");
    await Promise.resolve();
    callbacks.onStateChange?.("connected");
    callbacks.onWelcome?.({ playerId: PLAYER_ID, tickHz: 60 });
    return {};
  }

  async sendCommand(sequence, payload) {
    if (this.closed) throw new Error("Dedicated session is closed");
    const command = decodeCommand(payload);
    this.sent.push({ lane: "datagram", sequence, command });
    if (this.datagrams === "deliver") this.authority.receive("datagram", sequence, command);
    if (this.datagrams === "hold") this.heldBack.push({ sequence, command });
  }

  sendReliableCommand(sequence, payload) {
    if (this.closed) return Promise.reject(new Error("Dedicated session is closed"));
    const command = decodeCommand(payload);
    this.sent.push({ lane: "reliable", sequence, command });
    return new Promise((resolve, reject) => {
      const deliver = () => {
        if (this.rejectTypes.has(command.type)) {
          reject(new Error(`Dedicated command ${sequence} was rejected`));
          return;
        }
        resolve(this.authority.receive("reliable", sequence, command));
      };
      if (this.reliableDelayed) this.pendingReliable.push(deliver);
      else deliver();
    });
  }

  releaseReliable() {
    for (const deliver of this.pendingReliable.splice(0)) deliver();
  }

  releaseHeldBack() {
    for (const { sequence, command } of this.heldBack.splice(0)) {
      this.authority.receive("datagram", sequence, command);
    }
  }

  emitSnapshot(tick) {
    this.callbacks?.onSnapshot?.({ payload: new TextEncoder().encode(snapshotJson(tick)) });
  }

  close() {
    this.closed = true;
  }
}

async function dedicatedFixture() {
  const authority = new LaneAuthority();
  const sessions = [];
  const statuses = [];
  const runtime = createGameClientRuntime({
    snapshots: createSnapshotStore(),
    createGame: () => {
      throw new Error("dedicated play creates no local authority");
    },
    createPeerSession: () => {
      throw new Error("dedicated play creates no peer session");
    },
    attachPeerSession: () => () => {},
    createDedicatedSession: () => {
      const session = new ModelDedicatedSession(authority);
      sessions.push(session);
      return session;
    },
    supportsWebTransport: () => true,
    freshRunSeed: () => 1,
    localContentRevision: () => LOCAL_REVISION,
    onStatus: (status) => statuses.push(status),
    setInterval: (callback, ms) => globalThis.setInterval(callback, ms),
    clearInterval: (handle) => globalThis.clearInterval(handle as never),
  });
  await runtime.startDedicated("https://server.test/arpg");
  await flush();
  expect(runtime.getState()).toMatchObject({ lifecycle: "running", playerId: PLAYER_ID });
  const session = sessions[0];
  let tick = 0;
  return {
    runtime,
    authority,
    session,
    statuses,
    async run(ms) {
      const ticks = Math.ceil(ms / TICK_INTERVAL_MS);
      for (let index = 0; index < ticks; index += 1) {
        jest.advanceTimersByTime(TICK_INTERVAL_MS);
        tick += 1;
        session.emitSnapshot(tick);
        await flush();
      }
    },
    lanesOf(predicate) {
      return session.sent.filter((entry) => predicate(entry.command.type)).map((e) => e.lane);
    },
    close() {
      runtime.dispose();
    },
  };
}

beforeEach(() => {
  jest.useFakeTimers();
});

afterEach(() => {
  jest.useRealTimers();
});

test("one-shots use the reliable lane and held input stays on datagrams", async () => {
  const fixture = await dedicatedFixture();
  for (const command of ONE_SHOTS) fixture.runtime.dispatch(command);
  fixture.runtime.setGuard("keyboard", true);
  fixture.runtime.setHeldMovement("forward", true);
  fixture.runtime.setAim([0, 1000]);
  fixture.runtime.setBowDraw("keyboard", true);
  fixture.runtime.setBowDraw("keyboard", false);
  await fixture.run(2000);

  const oneShotLanes = fixture.lanesOf((type) => ONE_SHOT_TYPES.has(type));
  expect(oneShotLanes).toEqual(ONE_SHOTS.map(() => "reliable"));
  const heldLanes = fixture.lanesOf((type) => HELD_TYPES.has(type));
  expect(heldLanes.length).toBeGreaterThan(5);
  expect(new Set(heldLanes)).toEqual(new Set(["datagram"]));
  expect(fixture.lanesOf((type) => BOW_TYPES.has(type))).toEqual(["reliable", "reliable"]);
  expect(
    fixture.session.sent.every(
      (entry) =>
        HELD_TYPES.has(entry.command.type) ||
        ONE_SHOT_TYPES.has(entry.command.type) ||
        BOW_TYPES.has(entry.command.type),
    ),
  ).toBe(true);

  // Each one-shot was sent once, in order, and the refresh never repeated one.
  expect(fixture.authority.oneShots().map((entry) => entry.command)).toEqual(ONE_SHOTS);
  const sequences = fixture.session.sent.map((entry) => entry.sequence);
  for (let index = 1; index < sequences.length; index += 1) {
    expect(sequences[index]).toBeGreaterThan(sequences[index - 1]);
  }
  fixture.close();
});

test("losing every datagram loses no one-shot", async () => {
  const fixture = await dedicatedFixture();
  fixture.session.datagrams = "lose";
  for (const command of ONE_SHOTS) {
    fixture.runtime.setHeldMovement("right", true);
    fixture.runtime.dispatch(command);
    fixture.runtime.setHeldMovement("right", false);
    await fixture.run(100);
  }
  await fixture.run(1000);
  expect(fixture.authority.oneShots().map((entry) => entry.command)).toEqual(ONE_SHOTS);
  expect(fixture.authority.applied.every((entry) => entry.lane === "reliable")).toBe(true);
  fixture.close();
});

test("held datagrams that overtake a slow one-shot neither drop it nor wait for it", async () => {
  const fixture = await dedicatedFixture();
  fixture.session.reliableDelayed = true;
  fixture.runtime.dispatch({ type: "primaryAttack" });
  fixture.runtime.setGuard("keyboard", true);
  await fixture.run(CONVERGENCE_BOUND_MS);
  // Held input is not queued behind the unacknowledged one-shot.
  expect(fixture.authority.guard).toBe(true);
  const attack = fixture.session.sent.find((entry) => entry.command.type === "primaryAttack");
  expect(fixture.authority.watermarks.datagram).toBeGreaterThan(attack.sequence);

  fixture.session.releaseReliable();
  await flush();
  expect(fixture.authority.oneShots()).toEqual([
    { lane: "reliable", sequence: attack.sequence, command: { type: "primaryAttack" } },
  ]);
  fixture.close();
});

test("a reordered held datagram and one lost held datagram still converge beside one-shots", async () => {
  const fixture = await dedicatedFixture();
  fixture.runtime.setGuard("keyboard", true);
  await fixture.run(TICK_INTERVAL_MS);
  expect(fixture.authority.guard).toBe(true);

  // The release is held back behind newer traffic and arrives only after the refresh
  // repaired it, as a stale duplicate.
  fixture.session.datagrams = "hold";
  fixture.runtime.setGuard("keyboard", false);
  fixture.runtime.dispatch({ type: "interact" });
  fixture.session.datagrams = "deliver";
  fixture.runtime.setHeldMovement("left", true);
  await fixture.run(CONVERGENCE_BOUND_MS);
  fixture.session.releaseHeldBack();
  await fixture.run(CONVERGENCE_BOUND_MS);

  expect(fixture.authority.guard).toBe(false);
  expect(fixture.authority.movement).toEqual([-1, 0]);
  expect(fixture.authority.oneShots().map((entry) => entry.command)).toEqual([
    { type: "interact" },
  ]);
  fixture.close();
});

test("a rejected one-shot surfaces as a status and the session keeps running", async () => {
  const fixture = await dedicatedFixture();
  fixture.session.rejectTypes.add("equipWeapon");
  fixture.runtime.dispatch({ type: "equipWeapon", weapon: "bow" });
  await fixture.run(TICK_INTERVAL_MS);
  expect(fixture.statuses.some((status) => status.includes("rejected"))).toBe(true);
  expect(fixture.runtime.getState()).toMatchObject({ lifecycle: "running", playerId: PLAYER_ID });

  fixture.runtime.dispatch({ type: "primaryAttack" });
  await fixture.run(TICK_INTERVAL_MS);
  expect(fixture.authority.oneShots().map((entry) => entry.command)).toEqual([
    { type: "primaryAttack" },
  ]);
  fixture.close();
});

// Codex review on #151: a release that is lost or overtaken, followed by a new draw before
// the next refresh, must not merge two intended shots into one. Bow transitions travel on
// the reliable lane, so a held-back datagram cannot delay or drop them.
test("a delayed release followed by a new draw and a second release gives two shots", async () => {
  const fixture = await dedicatedFixture();
  fixture.runtime.dispatch({ type: "equipWeapon", weapon: "bow" });
  fixture.runtime.setBowDraw("keyboard", true);
  await fixture.run(500);

  // A datagram sent now is held back behind the later traffic; the next draw follows
  // before any refresh could repeat the release.
  fixture.session.datagrams = "hold";
  fixture.runtime.setBowDraw("keyboard", false);
  fixture.session.datagrams = "deliver";
  fixture.runtime.setBowDraw("keyboard", true);
  await fixture.run(500);
  fixture.runtime.setBowDraw("keyboard", false);
  await fixture.run(500);
  fixture.session.releaseHeldBack();
  await fixture.run(500);

  // The authority's bow: a release shoots the draw it consumes.
  let drawing = false;
  let shots = 0;
  for (const { command } of fixture.authority.applied) {
    if (command.type === "drawBow") drawing = true;
    if (command.type === "cancelBow") drawing = false;
    if (command.type === "releaseBow" && drawing) {
      drawing = false;
      shots += 1;
    }
  }
  expect(shots).toBe(2);
  expect(fixture.lanesOf((type) => BOW_TYPES.has(type))).toEqual([
    "reliable",
    "reliable",
    "reliable",
    "reliable",
  ]);
  fixture.close();
});
