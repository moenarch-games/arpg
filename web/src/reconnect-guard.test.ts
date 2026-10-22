// Reconnect mid-guard (#92): after a dedicated resume or a peer guest re-join, the
// authority's guard must match the guard input this client actually holds. A key still
// held is (re-)asserted; a key released during the outage leaves the guard lowered. Never a
// guard stuck raised with no input, never a phantom drop while the key is held.
//
// These tests drive the real client runtime with the real dedicated session and peer
// admission boundary; only the transports and authorities are modelled. The authority
// models follow what `crates/arpg-game-server/tests/reconnect_guard.rs` pins natively:
// `MatchRuntime` keeps the player, its guard and its command watermark across an in-grace
// resume, while a peer host re-adds a re-joined guest as a fresh player whose guard is down.
import { expect, test } from "bun:test";
import { createGameClientRuntime } from "./game-client-runtime.ts";
import { DedicatedGameSession } from "./dedicated-session.js";
import { attachPeerGameSession } from "./peer-session.js";
import { createSnapshotStore } from "./snapshot-store.js";
import { PROTOCOL_VERSION } from "./wire-protocol.js";

const LOCAL_REVISION = "0123456789abcdef";
const COMMAND_HEADER_BYTES = 8;

const snapshotJson = (tick, players) =>
  JSON.stringify({
    protocolVersion: PROTOCOL_VERSION,
    payload: {
      tick,
      runSeed: 7,
      worldUnitsPerMeter: 100,
      players: players.map((id) => ({
        id,
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
        weapon: "swordAndShield",
        interaction: { kind: "unavailable", reason: "nothingInRange" },
      })),
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

async function settle(condition = () => false, rounds = 50) {
  for (let round = 0; round < rounds && !condition(); round += 1) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

function runtimeWith({ createDedicatedSession = null, createPeerSession = null }) {
  const statuses = [];
  const runtime = createGameClientRuntime({
    snapshots: createSnapshotStore(),
    createGame: () => {
      throw new Error("remote sessions create no local authority");
    },
    createPeerSession,
    attachPeerSession: attachPeerGameSession,
    createDedicatedSession,
    supportsWebTransport: () => true,
    freshRunSeed: () => 1,
    localContentRevision: () => LOCAL_REVISION,
    onStatus: (status) => statuses.push(status),
    setInterval: () => 1,
    clearInterval: () => {},
  });
  return { runtime, statuses };
}

// ---------------------------------------------------------------------------------------
// Dedicated: real `DedicatedGameSession` over modelled WebTransport connections.

function welcomeFrame({ playerId, epoch, tokenByte }) {
  const frame = new Uint8Array(46);
  const view = new DataView(frame.buffer);
  frame[0] = 3;
  frame[1] = 3;
  view.setUint32(2, playerId, false);
  view.setUint16(6, 60, false);
  view.setUint16(8, 4, false);
  view.setBigUint64(10, 99n, false);
  view.setUint32(18, epoch, false);
  frame.fill(tokenByte, 22, 38);
  view.setBigUint64(38, 600n, false);
  return frame;
}

// The dedicated authority as `MatchRuntime` keeps one player across an in-grace resume:
// its guard input and its command watermark survive; stale sequences are ignored.
class DedicatedAuthority {
  watermark = 0;
  guardHeld = false;
  guardCommands = [];

  receive(frame) {
    const view = new DataView(frame.buffer, frame.byteOffset, frame.byteLength);
    const sequence = view.getUint32(2, false);
    if (sequence <= this.watermark) return;
    this.watermark = sequence;
    const json = new TextDecoder().decode(frame.subarray(COMMAND_HEADER_BYTES));
    const command = JSON.parse(json).payload;
    if (command.type === "setGuard") {
      this.guardHeld = command.raised;
      this.guardCommands.push(command.raised);
    }
  }
}

class ModelTransport {
  // While `silent`, the link has died without the client noticing yet: datagram writes
  // still resolve, but nothing reaches the authority (WebTransport datagrams are
  // unreliable and a write only queues the datagram locally).
  silent = false;
  dropped = false;

  constructor(authority, welcome, { ready = true } = {}) {
    let releaseReady;
    this.ready = new Promise((resolve) => {
      releaseReady = resolve;
    });
    this.releaseReady = releaseReady;
    if (ready) releaseReady();
    let signalClosed;
    this.closed = new Promise((resolve) => {
      signalClosed = resolve;
    });
    this.signalClosed = signalClosed;
    this.incomingUnidirectionalStreams = new ReadableStream({
      start(controller) {
        controller.enqueue(
          new ReadableStream({
            start(stream) {
              stream.enqueue(welcome);
              stream.close();
            },
          }),
        );
        controller.close();
      },
    });
    let datagrams;
    const readable = new ReadableStream({
      start(controller) {
        datagrams = controller;
      },
    });
    this.datagramController = datagrams;
    const transport = this;
    this.datagrams = {
      readable,
      writable: new WritableStream({
        write(frame) {
          if (!transport.silent) authority.receive(new Uint8Array(frame));
        },
      }),
    };
  }

  drop() {
    if (this.dropped) return;
    this.dropped = true;
    this.datagramController.close();
    this.signalClosed();
  }

  close() {
    this.drop();
  }
}

async function dedicatedFixture() {
  const authority = new DedicatedAuthority();
  const first = new ModelTransport(
    authority,
    welcomeFrame({ playerId: 1, epoch: 1, tokenByte: 1 }),
  );
  const second = new ModelTransport(
    authority,
    welcomeFrame({ playerId: 1, epoch: 2, tokenByte: 2 }),
    { ready: false },
  );
  const transports = [first, second];
  const sessions = [];
  const { runtime, statuses } = runtimeWith({
    createDedicatedSession: (endpoint) => {
      const session = new DedicatedGameSession({
        endpoint,
        transportFactory: () => {
          const next = transports.shift();
          if (!next) throw new Error("unexpected transport attempt");
          return next;
        },
        reconnectDelayMs: 1,
        sleep: async () => {},
      });
      sessions.push(session);
      return session;
    },
  });
  await runtime.startDedicated("https://server.test/arpg");
  expect(runtime.getState()).toMatchObject({ lifecycle: "running", playerId: 1 });
  const welcomes = () =>
    statuses.filter((status) => status.startsWith("Dedicated authority · player 1")).length;

  return {
    runtime,
    authority,
    first,
    second,
    statuses,
    // The live transport fails; the session starts resuming within the grace period.
    async loseConnection() {
      first.drop();
      await settle(() => statuses.at(-1) === "Connecting to dedicated authority…");
      expect(statuses.at(-1)).toBe("Connecting to dedicated authority…");
    },
    // The resume handshake completes against the same authoritative player.
    async resume() {
      const before = welcomes();
      second.releaseReady();
      await settle(() => welcomes() > before);
      expect(welcomes()).toBe(before + 1);
      // Let any follow-up commands of the resumed client reach the authority.
      await settle();
      expect(runtime.getState()).toMatchObject({ lifecycle: "running", playerId: 1 });
    },
    close() {
      runtime.dispose();
      for (const session of sessions) session.close();
    },
  };
}

test("dedicated: a guard key held through a resume stays raised at the authority", async () => {
  const fixture = await dedicatedFixture();
  fixture.runtime.setGuard("keyboard", true);
  await settle(() => fixture.authority.guardHeld);
  expect(fixture.authority.guardHeld).toBe(true);
  const beforeOutage = fixture.authority.guardCommands.length;

  await fixture.loseConnection();
  await fixture.resume();

  // No phantom drop: the resumed authority still guards and was never told to lower.
  expect(fixture.authority.guardHeld).toBe(true);
  expect(fixture.authority.guardCommands.slice(beforeOutage)).not.toContain(false);
  fixture.close();
});

test("dedicated: a guard key released while reconnecting is lowered after the resume", async () => {
  const fixture = await dedicatedFixture();
  fixture.runtime.setGuard("keyboard", true);
  await settle(() => fixture.authority.guardHeld);

  await fixture.loseConnection();
  fixture.runtime.setGuard("keyboard", false);
  expect(fixture.authority.guardHeld).toBe(true);
  await fixture.resume();

  expect(fixture.authority.guardHeld).toBe(false);
  fixture.close();
});

test("dedicated: a release lost to a link that died unnoticed is reconciled on resume", async () => {
  const fixture = await dedicatedFixture();
  fixture.runtime.setGuard("keyboard", true);
  await settle(() => fixture.authority.guardHeld);

  // The link is already dead when the key comes up; the client learns of it only later.
  fixture.first.silent = true;
  fixture.runtime.setGuard("keyboard", false);
  await settle();
  expect(fixture.authority.guardHeld).toBe(true);
  await fixture.loseConnection();
  await fixture.resume();

  // Nobody holds the key: the resumed authority must not keep a raised guard.
  expect(fixture.authority.guardHeld).toBe(false);
  fixture.close();
});

test("dedicated: a press lost to a link that died unnoticed is re-asserted on resume", async () => {
  const fixture = await dedicatedFixture();
  fixture.first.silent = true;
  fixture.runtime.setGuard("keyboard", true);
  await settle();
  expect(fixture.authority.guardHeld).toBe(false);
  await fixture.loseConnection();
  await fixture.resume();

  // The key is still held: the resumed authority must guard.
  expect(fixture.authority.guardHeld).toBe(true);
  fixture.close();
});

// ---------------------------------------------------------------------------------------
// Peer guest: real `attachPeerGameSession` over a modelled lobby link to the host.

// The guest's view of the host link. While the link is down the lobby client refuses to
// send (`sendReliable` throws "not ready"), as the pinned resilient lobby session does.
class ModelLobby extends EventTarget {
  hostParticipantId = "host";
  linkUp = true;
  commands = [];
  closed = false;

  host() {
    throw new Error("guest lobby");
  }
  async join() {
    return {};
  }
  sendReliable(peerId, data) {
    if (!this.linkUp) throw new Error(`Peer ${peerId} is not ready`);
    if (data?.kind === "command") {
      this.commands.push({ sequence: data.sequence, payload: JSON.parse(data.encoded).payload });
    }
  }
  broadcastRealtime() {}
  close() {
    this.closed = true;
  }
  emit(name, detail) {
    this.dispatchEvent(new CustomEvent(name, { detail }));
  }
  // The host (re-)admits the guest: `peer-ready` → `addPlayer` → reliable welcome.
  welcome(playerId) {
    this.emit("peer-ready", { peerId: "host" });
    this.emit("reliable", {
      peerId: "host",
      data: { kind: "welcome", playerId, encodedSnapshot: snapshotJson(1, [1, playerId]) },
    });
  }
}

// Host-side guard of the guest's player: a re-admitted guest is a fresh player (the host
// removed it on `participant-disconnected`), so its guard starts lowered.
function hostGuardSince(lobby, admittedAtCommand) {
  let held = false;
  for (const command of lobby.commands.slice(admittedAtCommand)) {
    if (command.payload.type === "setGuard") held = command.payload.raised;
  }
  return held;
}

async function guestFixture() {
  const lobby = new ModelLobby();
  const { runtime, statuses } = runtimeWith({ createPeerSession: () => lobby });
  await runtime.joinPeer("http://setup.test", "CODE");
  lobby.welcome(2);
  expect(runtime.getState()).toMatchObject({ mode: "guest", lifecycle: "running", playerId: 2 });
  return {
    runtime,
    lobby,
    statuses,
    // The link drops; the host removes the guest's player.
    loseLink() {
      lobby.linkUp = false;
    },
    // The link comes back and the host admits the guest again as a fresh player. Returns
    // the index of the first command the fresh player receives.
    rejoin() {
      lobby.linkUp = true;
      const admittedAt = lobby.commands.length;
      lobby.welcome(2);
      expect(runtime.getState()).toMatchObject({ lifecycle: "running", playerId: 2 });
      return admittedAt;
    },
  };
}

test("peer guest: a guard key held through a re-join is re-asserted to the fresh player", async () => {
  const fixture = await guestFixture();
  fixture.runtime.setGuard("keyboard", true);
  expect(hostGuardSince(fixture.lobby, 0)).toBe(true);

  fixture.loseLink();
  const admittedAt = fixture.rejoin();

  // The key is still held: the re-joined player must guard, not silently drop it.
  expect(hostGuardSince(fixture.lobby, admittedAt)).toBe(true);
  fixture.runtime.dispose();
});

test("peer guest: a guard key released during the outage stays lowered after a re-join", async () => {
  const fixture = await guestFixture();
  fixture.runtime.setGuard("keyboard", true);

  fixture.loseLink();
  // The release cannot be sent while the link is down.
  fixture.runtime.setGuard("keyboard", false);
  const admittedAt = fixture.rejoin();

  expect(hostGuardSince(fixture.lobby, admittedAt)).toBe(false);
  expect(
    fixture.lobby.commands
      .slice(admittedAt)
      .some((command) => command.payload.type === "setGuard" && command.payload.raised),
  ).toBe(false);
  // A later press raises the guard again from the released state.
  fixture.runtime.setGuard("keyboard", true);
  expect(hostGuardSince(fixture.lobby, admittedAt)).toBe(true);
  fixture.runtime.dispose();
});
