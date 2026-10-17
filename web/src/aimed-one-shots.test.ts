// Aimed one-shots carry their aim (#156). In dedicated play `setAim` is a best-effort
// datagram (#139), while `releaseBow`, `primaryAttack` and `secondaryAttack` travel on the
// reliable command lane (#143). The lanes have separate watermarks and are not ordered
// against each other. When the aim datagram just before a release is lost or delayed, the
// authority used to shoot along the previous aim. The aim the shot commits now travels
// inside the shot.
//
// Contract (web/src/game-client-runtime.ts, arpg-protocol v20):
//
// - `dispatch` fills `aim` on the aimed one-shots `primaryAttack`, `secondaryAttack` and
//   `releaseBow` from the runtime's current aim intent, the direction last passed to
//   `setAim`. This holds in every mode: dedicated, local and guest. That includes the
//   release that `setBowDraw(source, false)` sends.
// - With no current aim (none set yet, `setAim(null)`, or after `releaseInput()`), the
//   field is absent or `null`. Both decode to "no embedded aim", and the authority then
//   uses its held aim as today.
// - The browser adds no gameplay rule. arpg-core validates the embedded aim like `SetAim`
//   and applies it as the held aim before it commits the action, in the same tick. The
//   model authority below copies that rule only so the test can observe the outcome.
// - Other commands are unchanged. `drawBow`, `cancelBow`, `interact`, `setAim` and the held
//   refresh carry no `aim` field.
// - `PROTOCOL_VERSION` is 20. arpg-protocol's `tests/aimed_one_shots.rs` keeps the Rust
//   side in sync with it.
import { afterEach, beforeEach, expect, jest, test } from "bun:test";
import { createGameClientRuntime, TICK_INTERVAL_MS } from "./game-client-runtime.ts";
import { createSnapshotStore } from "./snapshot-store.js";
import { PROTOCOL_VERSION } from "./wire-protocol.js";

const LOCAL_REVISION = "0123456789abcdef";
const PLAYER_ID = 1;
const AIMED_TYPES = new Set(["primaryAttack", "secondaryAttack", "releaseBow"]);
const AIM_A = [300, 400];
const AIM_B = [-500, 0];

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
// The embedded aim as the authority reads it: absent and null both mean none.
const embeddedAim = (command) => command.aim ?? null;

async function flush(rounds = 25) {
  for (let round = 0; round < rounds; round += 1) await Promise.resolve();
}

// `MatchRuntime` + `arpg-core` for this player: one watermark per lane. An aimed one-shot
// with an embedded aim first makes it the held aim, then commits the held aim.
class LaneAuthority {
  watermarks = { datagram: 0, reliable: 0 };
  aim = null;
  drawing = false;
  // Every committed direction, in order: { type, aim }.
  committed = [];

  receive(lane, sequence, command) {
    if (sequence <= this.watermarks[lane]) return "duplicate";
    this.watermarks[lane] = sequence;
    if (command.type === "setAim") this.aim = command.direction;
    if (AIMED_TYPES.has(command.type)) {
      if (embeddedAim(command) !== null) this.aim = command.aim;
      this.committed.push({ type: command.type, aim: this.aim });
    }
    if (command.type === "drawBow") this.drawing = true;
    if (command.type === "releaseBow" || command.type === "cancelBow") this.drawing = false;
    return "applied";
  }
}

class ModelDedicatedSession {
  callbacks = null;
  sent = [];
  // The next datagram matching `lose` is lost; the next matching `delay` is held back
  // until `releaseDelayed()` (reordered behind later reliable traffic).
  lose = null;
  delay = null;
  delayed = [];
  closed = false;

  constructor(authority, weapon) {
    this.authority = authority;
    this.weapon = weapon;
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
    if (this.lose?.(command)) {
      this.lose = null;
      return;
    }
    if (this.delay?.(command)) {
      this.delay = null;
      this.delayed.push({ sequence, command });
      return;
    }
    this.authority.receive("datagram", sequence, command);
  }

  async sendReliableCommand(sequence, payload) {
    if (this.closed) throw new Error("Dedicated session is closed");
    const command = decodeCommand(payload);
    this.sent.push({ lane: "reliable", sequence, command });
    return this.authority.receive("reliable", sequence, command);
  }

  releaseDelayed() {
    for (const { sequence, command } of this.delayed.splice(0)) {
      this.authority.receive("datagram", sequence, command);
    }
  }

  emitSnapshot(tick) {
    this.callbacks?.onSnapshot?.({
      payload: new TextEncoder().encode(snapshotJson(tick, this.weapon)),
    });
  }

  close() {
    this.closed = true;
  }
}

async function dedicatedFixture({ weapon = "swordAndShield" } = {}) {
  const authority = new LaneAuthority();
  const sessions = [];
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
      return session;
    },
    supportsWebTransport: () => true,
    freshRunSeed: () => 1,
    localContentRevision: () => LOCAL_REVISION,
    onStatus: () => {},
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
    async run(ms) {
      const ticks = Math.ceil(ms / TICK_INTERVAL_MS);
      for (let index = 0; index < ticks; index += 1) {
        jest.advanceTimersByTime(TICK_INTERVAL_MS);
        tick += 1;
        session.emitSnapshot(tick);
        await flush();
      }
    },
    async settle() {
      await this.run(TICK_INTERVAL_MS);
    },
    sentOf(type) {
      return session.sent.filter((entry) => entry.command.type === type);
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

test("the protocol version is bumped for aimed one-shots", () => {
  // Version 20 introduced aimed one-shots; later versions (21: jump, #59) keep them.
  expect(PROTOCOL_VERSION).toBeGreaterThanOrEqual(20);
});

// The defect from the issue: aim A settled, the turn to B is lost, the release fires.
test("a release right after a lost aim datagram still shoots along the new aim", async () => {
  const fixture = await dedicatedFixture({ weapon: "bow" });
  fixture.runtime.setAim(AIM_A);
  fixture.runtime.setBowDraw("keyboard", true);
  await fixture.settle();
  expect(fixture.authority.aim).toEqual(AIM_A);
  expect(fixture.authority.drawing).toBe(true);

  fixture.session.lose = (command) => command.type === "setAim";
  fixture.runtime.setAim(AIM_B);
  fixture.runtime.setBowDraw("keyboard", false);
  await flush();

  expect(fixture.session.lose).toBeNull();
  const [release] = fixture.sentOf("releaseBow");
  expect(release.lane).toBe("reliable");
  expect(release.command.aim).toEqual(AIM_B);
  // Resolved at once, before any held refresh could repair the aim.
  expect(fixture.authority.committed).toEqual([{ type: "releaseBow", aim: AIM_B }]);
  expect(fixture.authority.aim).toEqual(AIM_B);
  fixture.close();
});

test("a release overtaking its delayed aim datagram still shoots along the new aim", async () => {
  const fixture = await dedicatedFixture({ weapon: "bow" });
  fixture.runtime.setAim(AIM_A);
  fixture.runtime.setBowDraw("touch", true);
  await fixture.settle();

  fixture.session.delay = (command) => command.type === "setAim";
  fixture.runtime.setAim(AIM_B);
  fixture.runtime.setBowDraw("touch", false);
  await flush();
  expect(fixture.authority.committed).toEqual([{ type: "releaseBow", aim: AIM_B }]);

  // The late datagram only repeats the aim the release already set.
  fixture.session.releaseDelayed();
  expect(fixture.authority.aim).toEqual(AIM_B);
  fixture.close();
});

for (const type of ["primaryAttack", "secondaryAttack"]) {
  test(`${type} right after a lost aim datagram strikes along the new aim`, async () => {
    const fixture = await dedicatedFixture();
    fixture.runtime.setAim(AIM_A);
    await fixture.settle();
    expect(fixture.authority.aim).toEqual(AIM_A);

    fixture.session.lose = (command) => command.type === "setAim";
    fixture.runtime.setAim(AIM_B);
    fixture.runtime.dispatch({ type });
    await flush();

    const [attack] = fixture.sentOf(type);
    expect(attack.lane).toBe("reliable");
    expect(attack.command.aim).toEqual(AIM_B);
    expect(fixture.authority.committed).toEqual([{ type, aim: AIM_B }]);
    fixture.close();
  });
}

test("aimed one-shots without a current aim carry no embedded aim", async () => {
  const fixture = await dedicatedFixture({ weapon: "bow" });
  fixture.runtime.dispatch({ type: "primaryAttack" });
  fixture.runtime.setAim(AIM_A);
  fixture.runtime.setAim(null);
  fixture.runtime.dispatch({ type: "secondaryAttack" });
  fixture.runtime.setAim(AIM_B);
  fixture.runtime.releaseInput();
  fixture.runtime.setBowDraw("keyboard", true);
  fixture.runtime.setBowDraw("keyboard", false);
  await flush();

  const aimed = fixture.session.sent.filter((entry) => AIMED_TYPES.has(entry.command.type));
  expect(aimed.map((entry) => entry.command.type)).toEqual([
    "primaryAttack",
    "secondaryAttack",
    "releaseBow",
  ]);
  for (const entry of aimed) expect(embeddedAim(entry.command)).toBeNull();
  fixture.close();
});

test("only the aimed one-shots carry an aim field", async () => {
  const fixture = await dedicatedFixture({ weapon: "bow" });
  fixture.runtime.setAim(AIM_B);
  fixture.runtime.dispatch({ type: "interact" });
  fixture.runtime.setBowDraw("keyboard", true);
  fixture.runtime.cancelBowDraw();
  await fixture.run(1000);

  for (const { command } of fixture.session.sent) {
    if (!AIMED_TYPES.has(command.type)) expect(command).not.toHaveProperty("aim");
  }
  expect(fixture.sentOf("interact")).toHaveLength(1);
  expect(fixture.sentOf("drawBow")).toHaveLength(1);
  expect(fixture.sentOf("cancelBow")).toHaveLength(1);
  fixture.close();
});

// Local (direct) play has one ordered channel, but the command still carries the aim, so
// every mode sends the same command shape.
class FakeGame {
  tick = 0;
  players = [];
  commands = [];
  addPlayer(id) {
    this.players.push(id);
  }
  applyCommand(playerId, sequence, encoded) {
    this.commands.push({ playerId, sequence, payload: JSON.parse(encoded).payload });
  }
  advanceTick() {
    this.tick += 1;
  }
  snapshotJson() {
    return snapshotJson(this.tick, "bow");
  }
  saveStateJson() {
    return JSON.stringify({ tick: this.tick });
  }
}

function localRuntime() {
  const games = [];
  const runtime = createGameClientRuntime({
    snapshots: createSnapshotStore(),
    createGame: () => {
      const game = new FakeGame();
      games.push(game);
      return game;
    },
    createPeerSession: () => {
      throw new Error("local play creates no peer session");
    },
    attachPeerSession: () => () => {},
    createDedicatedSession: () => {
      throw new Error("local play creates no dedicated session");
    },
    supportsWebTransport: () => false,
    freshRunSeed: () => 1,
    localContentRevision: () => LOCAL_REVISION,
    onStatus: () => {},
    setInterval: () => 0,
    clearInterval: () => {},
  });
  runtime.startLocal({ seed: 1 });
  expect(games).toHaveLength(1);
  return { runtime, game: games[0] };
}

test("local play sends the current aim with every aimed one-shot", () => {
  const { runtime, game } = localRuntime();
  runtime.dispatch({ type: "primaryAttack" });
  runtime.setAim(AIM_A);
  runtime.dispatch({ type: "primaryAttack" });
  runtime.setAim(AIM_B);
  runtime.dispatch({ type: "secondaryAttack" });
  runtime.setBowDraw("keyboard", true);
  runtime.setBowDraw("keyboard", false);
  runtime.setAim(null);
  runtime.dispatch({ type: "primaryAttack" });

  const aimed = game.commands
    .map((entry) => entry.payload)
    .filter((command) => AIMED_TYPES.has(command.type))
    .map((command) => ({ type: command.type, aim: embeddedAim(command) }));
  expect(aimed).toEqual([
    { type: "primaryAttack", aim: null },
    { type: "primaryAttack", aim: AIM_A },
    { type: "secondaryAttack", aim: AIM_B },
    { type: "releaseBow", aim: AIM_B },
    { type: "primaryAttack", aim: null },
  ]);
  runtime.dispose();
});
