import { expect, test } from "bun:test";
import { createGameClientRuntime } from "./game-client-runtime.ts";
import { PROTOCOL_VERSION } from "./wire-protocol.js";
import { createSnapshotStore } from "./snapshot-store.js";

const LOCAL_REVISION = "0123456789abcdef";

const snapshotJson = (tick, players = [1], contentRevision = LOCAL_REVISION) =>
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
      contentRevision,
      strikeEvents: [],
      chests: [],
      interactionEvents: [],
    },
  });

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

class FakeGame {
  tick = 0;
  freed = false;
  players = [];
  commands = [];
  constructor(seed) {
    this.seed = seed;
  }
  addPlayer(id) {
    this.players.push(id);
  }
  removePlayer(id) {
    this.players = this.players.filter((player) => player !== id);
  }
  applyCommand(playerId, sequence, encoded) {
    if (this.freed) throw new Error("used after free");
    this.commands.push({
      playerId,
      sequence,
      payload: JSON.parse(encoded).payload,
    });
  }
  advanceTick() {
    if (this.freed) throw new Error("used after free");
    this.tick += 1;
  }
  snapshotJson() {
    return snapshotJson(this.tick, this.players);
  }
  saveStateJson() {
    return JSON.stringify({ tick: this.tick });
  }
  reproductionJson() {
    return JSON.stringify({
      seed: this.seed,
      ticks: this.tick,
      commands: this.commands.length,
    });
  }
  operations = [];
  applyWorkbench(encoded) {
    const operation = JSON.parse(encoded);
    if (operation.type === "setTuning" && operation.value < 1) {
      throw new Error(`${operation.parameter}: must be positive`);
    }
    this.operations.push(operation);
  }
  workbenchRoomId() {
    return 2;
  }
  tuningJson() {
    return JSON.stringify([{ parameter: "guard.maxPoints", value: 100 }]);
  }
  free() {
    this.freed = true;
  }
}

class FakePeerSession {
  hostParticipantId = "host";
  closed = false;
  sent = [];
  broadcasts = [];
  host = deferred();
  join = deferred();
  constructor(apiBase) {
    this.apiBase = apiBase;
  }
  hostLobby() {
    return this.host.promise;
  }
  sendReliable(peerId, data) {
    this.sent.push({ peerId, data });
  }
  broadcastRealtime(data) {
    this.broadcasts.push(data);
  }
  close() {
    this.closed = true;
  }
}

class FakeDedicatedSession {
  closed = false;
  // Datagram commands (`sendCommand`) and reliable one-shots (`sendReliableCommand`, #143).
  commands = [];
  reliable = [];
  ready = deferred();
  callbacks = null;
  constructor(endpoint) {
    this.endpoint = endpoint;
  }
  connect(callbacks) {
    this.callbacks = callbacks;
    callbacks.onStateChange?.("connecting");
    return this.ready.promise;
  }
  async sendCommand(sequence, payload) {
    this.commands.push({
      sequence,
      payload: new TextDecoder().decode(payload),
    });
  }
  async sendReliableCommand(sequence, payload) {
    this.reliable.push({
      sequence,
      payload: new TextDecoder().decode(payload),
    });
    return "applied";
  }
  close() {
    this.closed = true;
  }
}

function harness({ webTransport = true } = {}) {
  const games = [];
  const peers = [];
  const dedicatedSessions = [];
  const timers = new Map();
  const statuses = [];
  const attachments = [];
  let nextTimer = 1;
  let nextSeed = 100;
  const snapshots = createSnapshotStore();
  const runtime = createGameClientRuntime({
    snapshots,
    createGame: (seed) => {
      const game = new FakeGame(seed);
      games.push(game);
      return game;
    },
    createPeerSession: (apiBase) => {
      const session = new FakePeerSession(apiBase);
      peers.push(session);
      return {
        get hostParticipantId() {
          return session.hostParticipantId;
        },
        host: () => session.hostLobby(),
        join: () => session.join.promise,
        sendReliable: (peerId, data) => session.sendReliable(peerId, data),
        broadcastRealtime: (data) => session.broadcastRealtime(data),
        close: () => session.close(),
      };
    },
    attachPeerSession: (options) => {
      const attachment = { options, detached: false };
      attachments.push(attachment);
      return () => {
        attachment.detached = true;
      };
    },
    createDedicatedSession: (endpoint) => {
      const session = new FakeDedicatedSession(endpoint);
      dedicatedSessions.push(session);
      return session;
    },
    supportsWebTransport: () => webTransport,
    freshRunSeed: () => nextSeed++,
    localContentRevision: () => LOCAL_REVISION,
    onStatus: (status) => statuses.push(status),
    setInterval: (callback) => {
      const id = nextTimer++;
      timers.set(id, callback);
      return id;
    },
    clearInterval: (id) => timers.delete(id),
  });
  const tick = (frames = 1) => {
    for (let frame = 0; frame < frames; frame += 1) {
      for (const callback of [...timers.values()]) callback();
    }
  };
  return {
    runtime,
    snapshots,
    games,
    peers,
    dedicatedSessions,
    timers,
    statuses,
    attachments,
    tick,
  };
}

test("starting a local game replaces the previous authority and keeps one tick loop", () => {
  const { runtime, games, timers, tick, snapshots } = harness();
  runtime.startLocal({ seed: 1 });
  tick(3);
  runtime.startLocal({ seed: 2 });
  runtime.startLocal({ seed: 3 });

  expect(games.map((game) => game.freed)).toEqual([true, true, false]);
  expect(timers.size).toBe(1);
  tick(2);
  expect(games[2].tick).toBe(2);
  expect(games[0].tick).toBe(3);
  expect(snapshots.getSnapshot().tick).toBe(2);
  expect(runtime.getState()).toMatchObject({
    lifecycle: "running",
    mode: "local",
    playerId: 1,
    sourceGeneration: 3,
  });
});

test("repeated enter and leave releases every authority and loop", () => {
  const { runtime, games, timers, snapshots } = harness();
  for (let round = 0; round < 5; round += 1) {
    runtime.startLocal();
    expect(timers.size).toBe(1);
    runtime.stop();
    expect(timers.size).toBe(0);
    expect(snapshots.getSnapshot()).toBeNull();
    expect(runtime.getState().lifecycle).toBe("idle");
  }
  expect(games).toHaveLength(5);
  expect(games.every((game) => game.freed)).toBe(true);
});

test("commands carry increasing sequences to the active local authority", () => {
  const { runtime, games } = harness();
  runtime.startLocal({ seed: 1 });
  runtime.dispatch({ type: "primaryAttack" });
  runtime.setHeldMovement("forward", true);
  runtime.setHeldMovement("forward", true);
  runtime.setHeldMovement("right", true);
  runtime.releaseInput();

  expect(games[0].commands).toEqual([
    { playerId: 1, sequence: 1, payload: { type: "primaryAttack" } },
    { playerId: 1, sequence: 2, payload: { type: "setMovement", x: 0, z: -1 } },
    { playerId: 1, sequence: 3, payload: { type: "setMovement", x: 1, z: -1 } },
    { playerId: 1, sequence: 4, payload: { type: "setMovement", x: 0, z: 0 } },
  ]);

  runtime.startLocal({ seed: 2 });
  runtime.dispatch({ type: "interact" });
  expect(games[1].commands).toEqual([{ playerId: 1, sequence: 1, payload: { type: "interact" } }]);
});

test("held input is cleared when the source is replaced", () => {
  const { runtime, games } = harness();
  runtime.startLocal({ seed: 1 });
  runtime.setHeldMovement("left", true);
  runtime.startLocal({ seed: 2 });
  runtime.setTouchMovement(0, 0);
  runtime.setHeldMovement("left", false);

  expect(games[1].commands).toEqual([]);
});

test("local → host → guest → dedicated changes close every previous source", async () => {
  const { runtime, games, peers, attachments, dedicatedSessions, timers } = harness();
  runtime.startLocal({ seed: 1 });

  const hosting = runtime.hostPeer("http://setup");
  expect(games[0].freed).toBe(true);
  expect(runtime.getState()).toMatchObject({
    mode: "local",
    lifecycle: "starting",
  });
  peers[0].host.resolve({ displayCode: "ABCD" });
  await hosting;
  expect(runtime.getState()).toMatchObject({
    mode: "host",
    lifecycle: "running",
    lobbyCode: "ABCD",
  });
  expect(timers.size).toBe(1);

  const joining = runtime.joinPeer("http://setup", "WXYZ");
  expect(peers[0].closed).toBe(true);
  expect(attachments[0].detached).toBe(true);
  expect(games[1].freed).toBe(true);
  expect(timers.size).toBe(0);
  expect(runtime.getState()).toMatchObject({
    mode: "guest",
    lifecycle: "starting",
    playerId: null,
    lobbyCode: "",
  });
  peers[1].join.resolve({});
  await joining;
  attachments[1].options.onPlayer(3);
  expect(runtime.getState()).toMatchObject({
    playerId: 3,
    lifecycle: "running",
  });
  runtime.dispatch({ type: "interact" });
  expect(peers[1].sent).toEqual([
    {
      peerId: "host",
      data: { kind: "command", sequence: 1, encoded: expect.any(String) },
    },
  ]);

  const connecting = runtime.startDedicated("https://server/arpg");
  expect(peers[1].closed).toBe(true);
  expect(attachments[1].detached).toBe(true);
  dedicatedSessions[0].callbacks.onWelcome({ playerId: 2, tickHz: 20 });
  dedicatedSessions[0].ready.resolve();
  await connecting;
  runtime.dispatch({ type: "primaryAttack" });
  await Promise.resolve();
  // A one-shot travels on the reliable command lane, never as a datagram (#143).
  expect(dedicatedSessions[0].reliable).toEqual([
    { sequence: 1, payload: expect.stringContaining("primaryAttack") },
  ]);
  expect(dedicatedSessions[0].commands).toEqual([]);
  expect(runtime.getState()).toMatchObject({
    mode: "dedicated",
    lifecycle: "running",
    playerId: 2,
  });

  runtime.startLocal();
  expect(dedicatedSessions[0].closed).toBe(true);
  expect(timers.size).toBe(1);
});

test("the host broadcasts each authoritative snapshot it publishes", async () => {
  const { runtime, peers, tick } = harness();
  const hosting = runtime.hostPeer("http://setup");
  peers[0].host.resolve({ displayCode: "ABCD" });
  await hosting;
  tick(2);
  expect(peers[0].broadcasts.map((message) => JSON.parse(message.encoded).payload.tick)).toEqual([
    1, 2,
  ]);
});

test("a late host completion cannot activate a replaced session", async () => {
  const { runtime, peers, statuses } = harness();
  const hosting = runtime.hostPeer("http://setup");
  runtime.startLocal({ seed: 9 });
  peers[0].host.resolve({ displayCode: "LATE" });
  await hosting;

  expect(peers[0].closed).toBe(true);
  expect(runtime.getState()).toMatchObject({ mode: "local", lobbyCode: "" });
  expect(statuses).not.toContain("Hosting lobby LATE");
});

test("a failed host keeps the local authority running solo", async () => {
  const { runtime, peers, games, timers, statuses } = harness();
  const hosting = runtime.hostPeer("http://setup");
  peers[0].host.reject(new Error("offline"));
  await hosting;

  expect(peers[0].closed).toBe(true);
  expect(games[0].freed).toBe(false);
  expect(timers.size).toBe(1);
  expect(runtime.getState()).toMatchObject({
    mode: "local",
    lifecycle: "running",
  });
  expect(statuses.at(-1)).toBe("Could not host: Error: offline");
});

test("a failed join reports a usable failed state", async () => {
  const { runtime, peers, statuses } = harness();
  const joining = runtime.joinPeer("http://setup", "NOPE");
  peers[0].join.reject(new Error("unknown lobby"));
  await joining;

  expect(peers[0].closed).toBe(true);
  expect(runtime.getState()).toMatchObject({
    mode: "guest",
    lifecycle: "failed",
  });
  expect(statuses.at(-1)).toBe("Could not join: Error: unknown lobby");
});

test("late dedicated callbacks from a replaced session are ignored", async () => {
  const { runtime, dedicatedSessions, snapshots, statuses } = harness();
  const connecting = runtime.startDedicated("https://server/arpg");
  const stale = dedicatedSessions[0];
  runtime.startLocal({ seed: 4 });
  const localSnapshot = snapshots.getSnapshot();

  stale.callbacks.onWelcome({ playerId: 3, tickHz: 20 });
  stale.callbacks.onSnapshot({
    payload: new TextEncoder().encode(snapshotJson(99)),
  });
  stale.callbacks.onStateChange("disconnected");
  stale.ready.reject(new Error("closed"));
  await connecting;

  expect(stale.closed).toBe(true);
  expect(snapshots.getSnapshot()).toBe(localSnapshot);
  expect(runtime.getState()).toMatchObject({
    mode: "local",
    playerId: 1,
    lifecycle: "running",
  });
  expect(statuses.some((status) => status.includes("Dedicated authority"))).toBe(false);
});

test("dedicated reconnect callbacks move between disconnected and running", async () => {
  const { runtime, dedicatedSessions } = harness();
  const connecting = runtime.startDedicated("https://server/arpg");
  const session = dedicatedSessions[0];
  session.callbacks.onWelcome({ playerId: 2, tickHz: 20 });
  session.ready.resolve();
  await connecting;

  session.callbacks.onStateChange("disconnected");
  expect(runtime.getState()).toMatchObject({
    lifecycle: "disconnected",
    playerId: null,
  });
  runtime.dispatch({ type: "interact" });
  session.callbacks.onWelcome({ playerId: 2, tickHz: 20 });
  expect(runtime.getState()).toMatchObject({
    lifecycle: "running",
    playerId: 2,
  });
  expect(session.commands).toEqual([]);
  expect(session.reliable).toEqual([]);
});

test("startup failures and rejected payloads become failed states, not black screens", async () => {
  const { runtime, dedicatedSessions, statuses } = harness();
  const failing = runtime.startDedicated("https://server/arpg");
  dedicatedSessions[0].ready.reject(new Error("handshake timed out"));
  await failing;
  expect(runtime.getState()).toMatchObject({
    lifecycle: "failed",
    mode: "dedicated",
  });
  expect(statuses.at(-1)).toBe("Could not connect dedicated server: Error: handshake timed out");

  const connecting = runtime.startDedicated("https://server/arpg");
  dedicatedSessions[1].callbacks.onSnapshot({
    payload: new TextEncoder().encode("{}"),
  });
  expect(dedicatedSessions[1].closed).toBe(true);
  expect(runtime.getState().lifecycle).toBe("failed");
  expect(statuses.at(-1)).toStartWith("Rejected dedicated snapshot");
  dedicatedSessions[1].ready.resolve();
  await connecting;
});

test("a dedicated authority running other content is refused with a clear reason", async () => {
  const { runtime, dedicatedSessions, snapshots, statuses } = harness();
  const connecting = runtime.startDedicated("https://server/arpg");
  dedicatedSessions[0].callbacks.onWelcome({ playerId: 2, tickHz: 60 });
  dedicatedSessions[0].callbacks.onSnapshot({
    payload: new TextEncoder().encode(snapshotJson(5, [2], "fedcba9876543210")),
  });
  expect(dedicatedSessions[0].closed).toBe(true);
  expect(snapshots.getSnapshot()).toBeNull();
  expect(runtime.getState()).toMatchObject({
    lifecycle: "failed",
    playerId: null,
  });
  expect(statuses.at(-1)).toBe(
    "Incompatible game content: the dedicated authority runs content revision " +
      "fedcba9876543210, this client runs 0123456789abcdef. Both sides need the same game build.",
  );
  dedicatedSessions[0].ready.resolve();
  await connecting;
});

test("a guest whose host runs other content stops and releases the session", async () => {
  const { runtime, peers, attachments, statuses } = harness();
  const joining = runtime.joinPeer("https://lobby", "ABCD");
  peers[0].join.resolve({});
  await joining;
  expect(attachments[0].options.localContentRevision()).toBe(LOCAL_REVISION);
  attachments[0].options.onIncompatible("Incompatible game content: host differs");
  expect(peers[0].closed).toBe(true);
  expect(attachments[0].detached).toBe(true);
  expect(runtime.getState()).toMatchObject({
    lifecycle: "failed",
    playerId: null,
  });
  expect(statuses.at(-1)).toBe("Incompatible game content: host differs");
});

test("dedicated play without WebTransport leaves the current source untouched", async () => {
  const { runtime, games, statuses } = harness({ webTransport: false });
  runtime.startLocal({ seed: 1 });
  await runtime.startDedicated("https://server/arpg");
  expect(games[0].freed).toBe(false);
  expect(runtime.getState().mode).toBe("local");
  expect(statuses.at(-1)).toBe("Dedicated online requires browser WebTransport support");
});

test("a simulation error stops the loop and fails closed", () => {
  const { runtime, games, timers, statuses } = harness();
  runtime.startLocal({ seed: 1 });
  games[0].advanceTick = () => {
    throw new Error("broken");
  };
  for (const callback of [...timers.values()]) callback();
  expect(timers.size).toBe(0);
  expect(runtime.getState().lifecycle).toBe("failed");
  expect(statuses.at(-1)).toBe("Simulation stopped: Error: broken");
  runtime.dispatch({ type: "interact" });
  expect(games[0].commands).toEqual([]);
});

test("training pause, step and speed are owned by the runtime loop", () => {
  const { runtime, games, tick } = harness();
  runtime.startLocal({ seed: 42, training: true });
  expect(runtime.getState().training).toEqual({
    paused: false,
    speed: 1,
    scenario: "dungeon",
  });
  tick(2);
  expect(games[0].tick).toBe(2);

  runtime.setTrainingPaused(true);
  tick(5);
  expect(games[0].tick).toBe(2);
  runtime.stepTraining();
  expect(games[0].tick).toBe(3);

  runtime.setTrainingPaused(false);
  runtime.setTrainingSpeed(2);
  tick(1);
  expect(games[0].tick).toBe(5);

  runtime.startLocal({ seed: 1 });
  expect(runtime.getState().training).toBeNull();
  runtime.setTrainingPaused(true);
  expect(runtime.getState().training).toBeNull();
});

test("restoring a save resumes its sequence fence and movement", () => {
  const { runtime, games, timers } = harness();
  runtime.startLocal({ seed: 1 });
  const restored = new FakeGame(5);
  restored.addPlayer(2);
  runtime.restore({
    game: restored,
    controlledPlayerId: 2,
    lastSequence: 40,
    movement: [1, 0],
  });

  expect(games[0].freed).toBe(true);
  expect(timers.size).toBe(1);
  runtime.setHeldMovement("right", true);
  runtime.dispatch({ type: "interact" });
  expect(restored.commands).toEqual([{ playerId: 2, sequence: 41, payload: { type: "interact" } }]);
  expect(runtime.captureSaveState()).toEqual({
    saveStateJson: '{"tick":0}',
    playerId: 2,
  });
});

test("disposal releases everything and makes the runtime inert", async () => {
  const { runtime, games, peers, attachments, timers, statuses } = harness();
  const hosting = runtime.hostPeer("http://setup");
  const changes = [];
  runtime.subscribe(() => changes.push(runtime.getState().lifecycle));
  runtime.dispose();
  peers[0].host.resolve({ displayCode: "LATE" });
  await hosting;

  expect(games[0].freed).toBe(true);
  expect(peers[0].closed).toBe(true);
  expect(attachments[0].detached).toBe(true);
  expect(timers.size).toBe(0);
  expect(changes).toEqual(["disposed"]);
  const statusCount = statuses.length;

  runtime.startLocal();
  await runtime.startDedicated("https://server/arpg");
  runtime.dispatch({ type: "interact" });
  expect(games).toHaveLength(1);
  expect(timers.size).toBe(0);
  expect(statuses).toHaveLength(statusCount);
  expect(runtime.getState().lifecycle).toBe("disposed");
});

test("guard is held while any device holds it and released on focus loss", () => {
  const { runtime, games } = harness();
  runtime.startLocal({ seed: 1 });
  runtime.setGuard("keyboard", true);
  runtime.setGuard("touch", true);
  runtime.setGuard("keyboard", false);
  runtime.setGuard("keyboard", true);
  runtime.releaseInput();
  runtime.releaseInput();
  runtime.setGuard("touch", false);
  expect(games[0].commands.map((command) => command.payload)).toEqual([
    { type: "setGuard", raised: true },
    { type: "setGuard", raised: false },
  ]);

  runtime.setGuard("keyboard", true);
  runtime.startLocal({ seed: 2 });
  runtime.setGuard("keyboard", false);
  expect(games[1].commands).toEqual([]);
});

test("restoring a save that held guard releases it for this client", () => {
  const { runtime } = harness();
  const restored = new FakeGame(5);
  restored.addPlayer(1);
  runtime.restore({
    game: restored,
    controlledPlayerId: 1,
    lastSequence: 3,
    movement: [0, 0],
    guardHeld: true,
  });
  expect(restored.commands).toEqual([
    { playerId: 1, sequence: 4, payload: { type: "setGuard", raised: false } },
  ]);
});

test("aim that could not be dispatched yet is sent once the session can play", () => {
  const { runtime, games } = harness();
  // No session yet: the command is dropped, so the direction must not count as sent.
  runtime.setAim([700, 700]);
  runtime.startLocal({ seed: 1 });
  runtime.setAim([700, 700]);
  expect(games[0].commands.map((command) => command.payload)).toEqual([
    { type: "setAim", direction: [700, 700] },
  ]);
});

test("aim intent is sent when it changes and cleared on focus loss and restore", () => {
  const { runtime, games } = harness();
  runtime.startLocal({ seed: 1 });
  runtime.setAim([1000, -400]);
  runtime.setAim([1000, -400]);
  runtime.setAim([0, 1000]);
  runtime.cycleTarget();
  runtime.releaseInput();
  runtime.releaseInput();
  runtime.setAim(null);
  runtime.clearTarget();
  expect(games[0].commands.map((command) => command.payload)).toEqual([
    { type: "setAim", direction: [1000, -400] },
    { type: "setAim", direction: [0, 1000] },
    { type: "cycleTarget" },
    { type: "setAim", direction: null },
    { type: "clearTarget" },
  ]);

  runtime.setAim([1, 1]);
  runtime.startLocal({ seed: 2 });
  runtime.setAim(null);
  expect(games[1].commands).toEqual([]);

  const restored = new FakeGame(5);
  restored.addPlayer(1);
  runtime.restore({
    game: restored,
    controlledPlayerId: 1,
    lastSequence: 3,
    movement: [0, 0],
    aimHeld: true,
  });
  expect(restored.commands).toEqual([
    { playerId: 1, sequence: 4, payload: { type: "setAim", direction: null } },
  ]);
});

test("a held bow draw shoots on release and is cancelled, never fired, on focus loss", () => {
  const { runtime, games } = harness();
  runtime.startLocal({ seed: 1 });
  runtime.setBowDraw("keyboard", true);
  runtime.setBowDraw("keyboard", true);
  runtime.setBowDraw("keyboard", false);
  runtime.setBowDraw("keyboard", false);
  runtime.setBowDraw("keyboard", true);
  runtime.releaseInput();
  runtime.setBowDraw("keyboard", false);
  expect(games[0].commands.map((command) => command.payload.type)).toEqual([
    "drawBow",
    "releaseBow",
    "drawBow",
    "cancelBow",
  ]);
});

test("bow holds are tracked per device and interrupted holds cancel", () => {
  const { runtime, games } = harness();
  runtime.startLocal({ seed: 1 });
  runtime.setBowDraw("keyboard", true);
  runtime.setBowDraw("touch", true);
  runtime.setBowDraw("touch", false);
  runtime.setBowDraw("keyboard", false);
  runtime.setBowDraw("touch", true);
  runtime.setBowDraw("touch", false, { interrupted: true });
  runtime.setBowDraw("touch", false);
  expect(games[0].commands.map((command) => command.payload.type)).toEqual([
    "drawBow",
    "releaseBow",
    "drawBow",
    "cancelBow",
  ]);
});

test("cancelling a held draw clears every device so the next press draws again", () => {
  const { runtime, games } = harness();
  runtime.startLocal({ seed: 1 });
  runtime.setBowDraw("keyboard", true);
  runtime.cancelBowDraw();
  runtime.cancelBowDraw();
  runtime.setBowDraw("keyboard", false);
  runtime.setBowDraw("keyboard", true);
  expect(games[0].commands.map((command) => command.payload.type)).toEqual([
    "drawBow",
    "cancelBow",
    "drawBow",
  ]);
});

test("restoring a save mid-draw cancels the draw for this client", () => {
  const { runtime } = harness();
  const restored = new FakeGame(5);
  restored.addPlayer(1);
  runtime.restore({
    game: restored,
    controlledPlayerId: 1,
    lastSequence: 3,
    movement: [0, 0],
    drawHeld: true,
  });
  expect(restored.commands).toEqual([{ playerId: 1, sequence: 4, payload: { type: "cancelBow" } }]);
});

test("training authorities are created from the named scenario", () => {
  const scenarios = [];
  const local = createGameClientRuntime({
    snapshots: createSnapshotStore(),
    createGame: (seed, scenario) => {
      scenarios.push([seed, scenario]);
      return new FakeGame(seed);
    },
    createPeerSession: () => {
      throw new Error("unused");
    },
    attachPeerSession: () => () => {},
    createDedicatedSession: () => {
      throw new Error("unused");
    },
    supportsWebTransport: () => false,
    freshRunSeed: () => 1,
    onStatus: () => {},
    setInterval: () => 1,
    clearInterval: () => {},
  });
  local.startLocal({ seed: 9, training: true, scenario: "archery" });
  // Training without a named fixture uses the default scenario, so it still records.
  local.startLocal({ seed: 9, training: true });
  local.startLocal({ seed: 9, scenario: "archery" });
  expect(scenarios).toEqual([
    [9, "archery"],
    [9, "dungeon"],
    [9, undefined],
  ]);
  expect(local.getState().training).toBeNull();
});

test("accelerated training publishes every simulated tick", () => {
  const { runtime, snapshots, tick } = harness();
  runtime.startLocal({ seed: 1, training: true });
  runtime.setTrainingSpeed(2);
  const published = [];
  snapshots.subscribe(() => published.push(snapshots.getSnapshot().tick));
  tick(2);
  expect(published).toEqual([1, 2, 3, 4]);
});

test("only local training sessions export a reproduction", () => {
  const { runtime, tick } = harness();
  runtime.startLocal({ seed: 1 });
  expect(runtime.exportReproduction()).toBeNull();

  runtime.startLocal({ seed: 42, training: true, scenario: "dummy" });
  runtime.dispatch({ type: "primaryAttack" });
  tick(3);
  expect(JSON.parse(runtime.exportReproduction())).toEqual({
    seed: 42,
    ticks: 3,
    commands: 1,
  });

  void runtime.hostPeer("http://setup");
  expect(runtime.exportReproduction()).toBeNull();
  runtime.stop();
  expect(runtime.exportReproduction()).toBeNull();
});

test("workbench operations reach only the local training authority", () => {
  const { runtime, tick, games, snapshots } = harness();
  runtime.startLocal({ seed: 1 });
  expect(() => runtime.applyWorkbench({ type: "resetArrangement" })).toThrow("local training only");
  expect(runtime.trainingTuning()).toBeNull();

  runtime.startLocal({ seed: 42, training: true, scenario: "dummy" });
  runtime.setTrainingPaused(true);
  tick(1);
  let published = 0;
  snapshots.subscribe(() => published++);
  const spawn = {
    type: "spawnMonster",
    definition: "monster.brute",
    offset: [100, -40],
  };
  runtime.applyWorkbench(spawn);
  expect(games.at(-1).operations).toEqual([spawn]);
  // A paused session republishes so the new arrangement shows immediately.
  expect(published).toBe(1);
  expect(() =>
    runtime.applyWorkbench({
      type: "setTuning",
      parameter: "guard.maxPoints",
      value: 0,
    }),
  ).toThrow("guard.maxPoints: must be positive");
  expect(games.at(-1).operations).toEqual([spawn]);
  expect(runtime.trainingTuning()).toEqual([{ parameter: "guard.maxPoints", value: 100 }]);
  expect(runtime.trainingWorkbenchRoom()).toBe(2);

  void runtime.hostPeer("http://setup");
  expect(() => runtime.applyWorkbench({ type: "resetArrangement" })).toThrow();
  expect(runtime.trainingTuning()).toBeNull();
  expect(runtime.trainingWorkbenchRoom()).toBeNull();
});
