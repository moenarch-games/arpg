// Held input survives a lost dedicated datagram without a reconnect (#139). Dedicated
// commands travel as unreliable WebTransport datagrams. When exactly one held-state change
// (guard, movement, aim or bow draw) is lost and the player then does nothing more, the
// dedicated authority must still converge on what the player currently holds within a
// bounded time, on the same connection, without replaying stale values or one-shot actions.
//
// Acceptance contract, independent of the mechanism (periodic re-assertion of the held set,
// a reliable lane for held-state changes, …):
//
// - The seam is the real client runtime with a modelled dedicated session. `sendCommand` is
//   the datagram lane: the test loses exactly one chosen datagram (the write still resolves,
//   as a queued datagram write does). Any other `send*` method an implementation adds to the
//   session for (sequence, payload) commands is modelled as a reliable lane and delivered.
// - Time is fake and injected (`setInterval`, timers, `Date`, `performance`); the authority
//   keeps streaming snapshots at its 60 Hz tick rate, as a live dedicated server does.
// - The authority models `MatchRuntime`: it applies only sequences above its watermark and
//   treats a repeated held value (e.g. a repeated `setGuard` raise or `drawBow`) as a
//   continued hold, as `arpg-core` does.
//
// Bound: CONVERGENCE_BOUND_MS = 500 ms (30 ticks at 60 Hz). A guard stuck raised or a
// character walking on after release for longer than half a second is a gameplay-visible
// desync (blocks taken or missed, positions drifting), while 500 ms leaves room for any
// sensible re-assertion period or a reliable retransmission over a real round trip.
// Traffic bound: COMMANDS_PER_SECOND_PER_INPUT = 10 for each of the HELD_INPUTS = 3 held
// inputs (guard, movement, aim), i.e. at most 30 commands per second in total. Bow draw,
// release and cancel moved to the reliable command lane (#143 review): a release consumes
// its draw, so they cannot be lost as datagrams and are never re-asserted.
// With nothing lost the runtime may re-assert the current held set (even the empty one,
// while idle), but convergence must not be bought by re-sending every input every 60 Hz
// tick (240 per second): a 10 Hz re-assertion of each input already fits the 500 ms bound.
// Idle with nothing held and a steady held set are both checked over 5 s.
import { afterEach, beforeEach, expect, jest, test } from "bun:test";
import { createGameClientRuntime, TICK_INTERVAL_MS } from "./game-client-runtime.ts";
import { createSnapshotStore } from "./snapshot-store.js";
import { PROTOCOL_VERSION } from "./wire-protocol.js";

const LOCAL_REVISION = "0123456789abcdef";
const PLAYER_ID = 1;
const CONVERGENCE_BOUND_MS = 500;
const COMMANDS_PER_SECOND_PER_INPUT = 10;
const HELD_INPUTS = 3;
const HELD_TYPES = new Set([
  "setGuard",
  "setMovement",
  "setAim",
  "drawBow",
  "releaseBow",
  "cancelBow",
]);

const snapshotJson = (tick, weapon) =>
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
          motion: { grounded: true, verticalVelocity: 0, feetY: 0, groundY: 0 },
          maxGuardPoints: 100,
          weapon,
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

// Drains promise continuations without advancing (fake) time.
async function flush(rounds = 25) {
  for (let round = 0; round < rounds; round += 1) await Promise.resolve();
}

// The dedicated authority as `MatchRuntime` + `arpg-core` see this player's held input.
class DedicatedAuthority {
  watermark = 0;
  received = [];
  guard = false;
  movement = [0, 0];
  aim = null;
  drawing = false;

  receive(sequence, command) {
    if (sequence <= this.watermark) return;
    this.watermark = sequence;
    this.received.push({ sequence, command });
    switch (command.type) {
      case "setGuard":
        this.guard = command.raised;
        break;
      case "setMovement":
        this.movement = [command.x, command.z];
        break;
      case "setAim":
        this.aim = command.direction;
        break;
      case "drawBow":
        this.drawing = true;
        break;
      case "releaseBow":
      case "cancelBow":
        this.drawing = false;
        break;
    }
  }

  held() {
    return { guard: this.guard, movement: this.movement, aim: this.aim, drawing: this.drawing };
  }
}

// One dedicated connection that never reconnects. `sendCommand` is the lossy datagram lane.
class ModelDedicatedSession {
  callbacks = null;
  connects = 0;
  closed = false;
  // Every command the client sent, in call order, with its lane and whether it was lost.
  sent = [];
  lose = null;
  lost = 0;
  tick = 0;

  constructor(authority, weapon) {
    this.authority = authority;
    this.weapon = weapon;
  }

  async connect(callbacks) {
    this.connects += 1;
    this.callbacks = callbacks;
    callbacks.onStateChange?.("connecting");
    await Promise.resolve();
    callbacks.onStateChange?.("connected");
    callbacks.onWelcome?.({ playerId: PLAYER_ID, tickHz: 60 });
    return {};
  }

  async sendCommand(sequence, payload) {
    this.record("datagram", sequence, payload);
  }

  // Delivery over a lane other than the datagram one is reliable.
  sendOtherLane(lane, sequence, payload) {
    this.record(lane, sequence, payload);
  }

  record(lane, sequence, payload) {
    if (this.closed) throw new Error("Dedicated session is closed");
    const command = decodeCommand(payload);
    const lost = lane === "datagram" && this.lose !== null && this.lose(command);
    if (lost) {
      this.lose = null;
      this.lost += 1;
    }
    this.sent.push({ lane, sequence, command, lost });
    if (!lost) this.authority.receive(sequence, command);
  }

  // The next datagram matching `predicate` is lost; exactly one.
  loseNext(predicate) {
    this.lose = predicate;
  }

  emitSnapshot() {
    this.tick += 1;
    this.callbacks?.onSnapshot?.({
      payload: new TextEncoder().encode(snapshotJson(this.tick, this.weapon)),
    });
  }

  close() {
    this.closed = true;
  }
}

// Exposes the model session to the runtime; unknown `send*` methods become reliable lanes.
function sessionFacade(model) {
  return new Proxy(model, {
    get(target, property, receiver) {
      if (property in target) {
        const value = Reflect.get(target, property, receiver);
        return typeof value === "function" ? value.bind(target) : value;
      }
      if (typeof property === "string" && property.startsWith("send")) {
        return async (sequence, payload) => target.sendOtherLane(property, sequence, payload);
      }
      return undefined;
    },
  });
}

async function dedicatedFixture({ weapon = "swordAndShield" } = {}) {
  const authority = new DedicatedAuthority();
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
      const session = new ModelDedicatedSession(authority, weapon);
      sessions.push(session);
      return sessionFacade(session);
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

  return {
    runtime,
    authority,
    session,
    // Lets the dedicated authority run for `ms` of fake time with no player input.
    async run(ms) {
      const ticks = Math.ceil(ms / TICK_INTERVAL_MS);
      for (let tick = 0; tick < ticks; tick += 1) {
        jest.advanceTimersByTime(TICK_INTERVAL_MS);
        session.emitSnapshot();
        await flush();
      }
    },
    // Settles a delivered input before the loss under test.
    async settle() {
      await this.run(TICK_INTERVAL_MS);
    },
    mark() {
      return session.sent.length;
    },
    // The exact loss under test happened, on the one connection, which never reconnected.
    expectSingleLossNoReconnect() {
      expect(session.lost).toBe(1);
      expect(session.lose).toBeNull();
      expect(sessions).toHaveLength(1);
      expect(session.connects).toBe(1);
      expect(session.closed).toBe(false);
      expect(runtime.getState()).toMatchObject({ lifecycle: "running", playerId: PLAYER_ID });
    },
    // A bow transition armed to be lost went out on the reliable lane instead: nothing was
    // lost, on the one connection, which never reconnected.
    expectReliableNoLossNoReconnect(type) {
      expect(session.lost).toBe(0);
      expect(session.sent.filter((entry) => entry.command.type === type)).not.toHaveLength(0);
      for (const entry of session.sent.filter((sent) => sent.command.type === type)) {
        expect(entry.lane).toBe("sendReliableCommand");
      }
      expect(sessions).toHaveLength(1);
      expect(session.connects).toBe(1);
      expect(session.closed).toBe(false);
      expect(runtime.getState()).toMatchObject({ lifecycle: "running", playerId: PLAYER_ID });
    },
    // No stale replay: sequences strictly increase over every lane, and every command sent
    // since `since` (the last player input) carries the currently held value. One-shot
    // actions are never re-sent.
    expectNoStaleReplay(since, expected) {
      const sequences = session.sent.map((entry) => entry.sequence);
      for (let index = 1; index < sequences.length; index += 1) {
        expect(sequences[index]).toBeGreaterThan(sequences[index - 1]);
      }
      const received = authority.received.map((entry) => entry.sequence);
      expect(received).toEqual([...received].sort((left, right) => left - right));
      for (const { command } of session.sent.slice(since)) {
        expect(HELD_TYPES.has(command.type)).toBe(true);
        switch (command.type) {
          case "setGuard":
            expect(command.raised).toBe(expected.guard);
            break;
          case "setMovement":
            expect([command.x, command.z]).toEqual(expected.movement);
            break;
          case "setAim":
            expect(command.direction).toEqual(expected.aim);
            break;
          case "drawBow":
            expect(expected.bow).toBe("drawn");
            break;
          case "releaseBow":
            expect(expected.bow).toBe("released");
            break;
          case "cancelBow":
            expect(expected.bow).not.toBe("drawn");
            break;
        }
      }
    },
    close() {
      runtime.dispose();
    },
  };
}

const NOTHING_HELD = { guard: false, movement: [0, 0], aim: null, bow: "released" };
const authorityView = (expected) => ({
  guard: expected.guard,
  movement: expected.movement,
  aim: expected.aim,
  drawing: expected.bow === "drawn",
});

beforeEach(() => {
  jest.useFakeTimers();
});

afterEach(() => {
  jest.useRealTimers();
});

test("a lost guard release is lowered at the authority within the bound", async () => {
  const fixture = await dedicatedFixture();
  fixture.runtime.setGuard("keyboard", true);
  await fixture.settle();
  expect(fixture.authority.guard).toBe(true);

  const since = fixture.mark();
  fixture.session.loseNext((command) => command.type === "setGuard" && !command.raised);
  fixture.runtime.setGuard("keyboard", false);
  await flush();
  expect(fixture.authority.guard).toBe(true);

  await fixture.run(CONVERGENCE_BOUND_MS);
  fixture.expectSingleLossNoReconnect();
  expect(fixture.authority.held()).toEqual(authorityView(NOTHING_HELD));
  fixture.expectNoStaleReplay(since, NOTHING_HELD);
  fixture.close();
});

test("a lost guard raise is raised at the authority while the key stays held", async () => {
  const fixture = await dedicatedFixture();
  const expected = { ...NOTHING_HELD, guard: true };
  const since = fixture.mark();
  fixture.session.loseNext((command) => command.type === "setGuard" && command.raised);
  fixture.runtime.setGuard("keyboard", true);
  await flush();
  expect(fixture.authority.guard).toBe(false);

  await fixture.run(CONVERGENCE_BOUND_MS);
  fixture.expectSingleLossNoReconnect();
  expect(fixture.authority.held()).toEqual(authorityView(expected));
  fixture.expectNoStaleReplay(since, expected);
  fixture.close();
});

test("a lost movement change converges to the held direction without replaying attacks", async () => {
  const fixture = await dedicatedFixture();
  fixture.runtime.setHeldMovement("forward", true);
  fixture.runtime.dispatch({ type: "primaryAttack" });
  await fixture.settle();
  expect(fixture.authority.movement).toEqual([0, -1]);

  const expected = { ...NOTHING_HELD, movement: [1, -1] };
  const since = fixture.mark();
  fixture.session.loseNext((command) => command.type === "setMovement");
  fixture.runtime.setHeldMovement("right", true);
  await flush();
  expect(fixture.authority.movement).toEqual([0, -1]);

  await fixture.run(CONVERGENCE_BOUND_MS);
  fixture.expectSingleLossNoReconnect();
  expect(fixture.authority.held()).toEqual(authorityView(expected));
  fixture.expectNoStaleReplay(since, expected);
  fixture.close();
});

test("a lost movement stop halts the character at the authority", async () => {
  const fixture = await dedicatedFixture();
  fixture.runtime.setTouchMovement(0.5, -1);
  await fixture.settle();
  expect(fixture.authority.movement).toEqual([0.5, -1]);

  const since = fixture.mark();
  fixture.session.loseNext((command) => command.type === "setMovement");
  fixture.runtime.setTouchMovement(0, 0);
  await flush();
  expect(fixture.authority.movement).toEqual([0.5, -1]);

  await fixture.run(CONVERGENCE_BOUND_MS);
  fixture.expectSingleLossNoReconnect();
  expect(fixture.authority.held()).toEqual(authorityView(NOTHING_HELD));
  fixture.expectNoStaleReplay(since, NOTHING_HELD);
  fixture.close();
});

test("a lost aim change converges to the current aim direction", async () => {
  const fixture = await dedicatedFixture();
  fixture.runtime.setAim([300, 400]);
  await fixture.settle();
  expect(fixture.authority.aim).toEqual([300, 400]);

  const expected = { ...NOTHING_HELD, aim: [-500, 0] };
  const since = fixture.mark();
  fixture.session.loseNext((command) => command.type === "setAim");
  fixture.runtime.setAim([-500, 0]);
  await flush();
  expect(fixture.authority.aim).toEqual([300, 400]);

  await fixture.run(CONVERGENCE_BOUND_MS);
  fixture.expectSingleLossNoReconnect();
  expect(fixture.authority.held()).toEqual(authorityView(expected));
  fixture.expectNoStaleReplay(since, expected);
  fixture.close();
});

test("a lost aim clear returns the authority to committed facing", async () => {
  const fixture = await dedicatedFixture();
  fixture.runtime.setAim([0, 1000]);
  await fixture.settle();
  expect(fixture.authority.aim).toEqual([0, 1000]);

  const since = fixture.mark();
  fixture.session.loseNext((command) => command.type === "setAim");
  fixture.runtime.setAim(null);
  await flush();
  expect(fixture.authority.aim).toEqual([0, 1000]);

  await fixture.run(CONVERGENCE_BOUND_MS);
  fixture.expectSingleLossNoReconnect();
  expect(fixture.authority.held()).toEqual(authorityView(NOTHING_HELD));
  fixture.expectNoStaleReplay(since, NOTHING_HELD);
  fixture.close();
});

test("a bow draw is not lost with datagrams and is drawing at the authority", async () => {
  const fixture = await dedicatedFixture({ weapon: "bow" });
  const expected = { ...NOTHING_HELD, bow: "drawn" };
  const since = fixture.mark();
  fixture.session.loseNext((command) => command.type === "drawBow");
  fixture.runtime.setBowDraw("keyboard", true);
  await flush();
  expect(fixture.authority.drawing).toBe(true);

  await fixture.run(CONVERGENCE_BOUND_MS);
  fixture.expectReliableNoLossNoReconnect("drawBow");
  expect(fixture.authority.held()).toEqual(authorityView(expected));
  fixture.expectNoStaleReplay(since, expected);
  fixture.close();
});

test("a bow cancel is not lost with datagrams, lowers the draw and never shoots", async () => {
  const fixture = await dedicatedFixture({ weapon: "bow" });
  fixture.runtime.setBowDraw("touch", true);
  await fixture.settle();
  expect(fixture.authority.drawing).toBe(true);

  const expected = { ...NOTHING_HELD, bow: "cancelled" };
  const since = fixture.mark();
  fixture.session.loseNext((command) => command.type === "cancelBow");
  fixture.runtime.setBowDraw("touch", false, { interrupted: true });
  await flush();
  expect(fixture.authority.drawing).toBe(false);

  await fixture.run(CONVERGENCE_BOUND_MS);
  fixture.expectReliableNoLossNoReconnect("cancelBow");
  expect(fixture.authority.held()).toEqual(authorityView(expected));
  fixture.expectNoStaleReplay(since, expected);
  fixture.close();
});

test("a bow release is not lost with datagrams and leaves the authority not drawing", async () => {
  const fixture = await dedicatedFixture({ weapon: "bow" });
  fixture.runtime.setBowDraw("keyboard", true);
  await fixture.settle();
  expect(fixture.authority.drawing).toBe(true);

  const since = fixture.mark();
  fixture.session.loseNext((command) => command.type === "releaseBow");
  fixture.runtime.setBowDraw("keyboard", false);
  await flush();
  expect(fixture.authority.drawing).toBe(false);

  await fixture.run(CONVERGENCE_BOUND_MS);
  fixture.expectReliableNoLossNoReconnect("releaseBow");
  expect(fixture.authority.held()).toEqual(authorityView(NOTHING_HELD));
  fixture.expectNoStaleReplay(since, NOTHING_HELD);
  fixture.close();
});

// Regression: convergence must not turn into unbounded command traffic.

test("idle with nothing held and nothing lost stays within the command budget", async () => {
  const fixture = await dedicatedFixture();
  const since = fixture.mark();
  const seconds = 5;
  await fixture.run(seconds * 1000);

  const sent = fixture.session.sent.slice(since);
  expect(sent.length).toBeLessThanOrEqual(seconds * COMMANDS_PER_SECOND_PER_INPUT * HELD_INPUTS);
  expect(fixture.session.lost).toBe(0);
  expect(fixture.authority.held()).toEqual(authorityView(NOTHING_HELD));
  fixture.expectNoStaleReplay(since, NOTHING_HELD);
  fixture.close();
});

test("a steady held set without loss stays in sync within the per-input budget", async () => {
  const fixture = await dedicatedFixture();
  fixture.runtime.setGuard("keyboard", true);
  fixture.runtime.setHeldMovement("left", true);
  fixture.runtime.setAim([0, -1000]);
  const expected = { guard: true, movement: [-1, 0], aim: [0, -1000], bow: "released" };
  await fixture.settle();
  const since = fixture.mark();
  const seconds = 5;
  await fixture.run(seconds * 1000);

  const sent = fixture.session.sent.slice(since);
  expect(sent.length).toBeLessThanOrEqual(seconds * COMMANDS_PER_SECOND_PER_INPUT * HELD_INPUTS);
  expect(fixture.authority.held()).toEqual(authorityView(expected));
  fixture.expectNoStaleReplay(since, expected);
  fixture.close();
});
