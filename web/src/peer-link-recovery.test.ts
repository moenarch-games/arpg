// Peer link recovery without a re-join (#140): when the lobby client recovers a guest's
// link to the host in place (an ICE restart or a transient `disconnected`; no
// `participant-disconnected`, no new `peer-ready`), the host keeps the same player. Held
// input the guest tried to send during the outage was lost, so after the recovery the
// guest re-asserts the guard it actually holds, as after a re-join (#92 / #138): a key
// released during the outage leaves the host's guard lowered, a key held stays raised.
//
// The recovery is driven through the event surface of the pinned upstream lobby client
// (`web/resilient-lobby-session.ts` at the setup-service revision named in
// docs/ARCHITECTURE.md): every `RTCPeerConnection` state change emits
// `peer-statechange` `{ peerId, state }`, ICE-restart recovery emits `peer-recovery`, and
// `peer-ready` is emitted only once per link, so an in-place recovery never re-emits it.
// `sendReliable` throws "Peer … is not ready" while the connection state is not
// `connected`. Only the lobby transport and the host authority are modelled; the client
// runtime and the peer admission boundary are real.
import { expect, test } from "bun:test";
import { createGameClientRuntime } from "./game-client-runtime.ts";
import { attachPeerGameSession } from "./peer-session.js";
import { createSnapshotStore } from "./snapshot-store.js";
import { PROTOCOL_VERSION } from "./wire-protocol.js";

const LOCAL_REVISION = "0123456789abcdef";

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

async function settle(rounds = 20) {
  for (let round = 0; round < rounds; round += 1) {
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

// One participant's view of the lobby, modelled on the pinned resilient lobby session:
// a link per remote peer whose `RTCPeerConnection` state drives `peer-statechange`, and
// reliable sends that throw unless that link is `connected`.
class ModelLobby extends EventTarget {
  hostParticipantId;
  linkState = new Map();
  // Reliable messages that actually left this participant.
  sent = [];
  // Reliable sends refused because the link was down.
  refused = 0;
  closed = false;

  constructor(hostParticipantId) {
    super();
    this.hostParticipantId = hostParticipantId;
  }
  async host() {
    return { displayCode: "HOST" };
  }
  async join() {
    return {};
  }
  sendReliable(peerId, data) {
    if (this.linkState.get(peerId) !== "connected") {
      this.refused += 1;
      throw new Error(`Peer ${peerId} is not ready`);
    }
    this.sent.push({ peerId, data });
  }
  broadcastRealtime() {}
  close() {
    this.closed = true;
  }
  emit(name, detail) {
    this.dispatchEvent(new CustomEvent(name, { detail }));
  }
  // `connectionstatechange`: the state is already current when the event is emitted.
  setPeerState(peerId, state) {
    this.linkState.set(peerId, state);
    this.emit("peer-statechange", { peerId, state });
  }
  commandsTo(peerId) {
    return this.sent
      .filter((message) => message.peerId === peerId && message.data?.kind === "command")
      .map((message) => JSON.parse(message.data.encoded).payload);
  }
}

// The host keeps the guest's player across an in-place recovery, so its guard is simply
// the last `setGuard` the guest delivered since admission.
function hostGuard(commands) {
  let held = false;
  for (const command of commands) if (command.type === "setGuard") held = command.raised;
  return held;
}

function runtimeWith(options) {
  const statuses = [];
  const runtime = createGameClientRuntime({
    snapshots: createSnapshotStore(),
    createGame: () => {
      throw new Error("remote sessions create no local authority");
    },
    createPeerSession: () => {
      throw new Error("no peer session expected");
    },
    attachPeerSession: attachPeerGameSession,
    createDedicatedSession: () => {
      throw new Error("no dedicated session expected");
    },
    supportsWebTransport: () => true,
    freshRunSeed: () => 1,
    localContentRevision: () => LOCAL_REVISION,
    onStatus: (status) => statuses.push(status),
    setInterval: () => 1,
    clearInterval: () => {},
    ...options,
  });
  return { runtime, statuses };
}

async function guestFixture() {
  const lobby = new ModelLobby("host");
  const { runtime } = runtimeWith({ createPeerSession: () => lobby });
  await runtime.joinPeer("http://setup.test", "CODE");
  // Initial connection as the upstream client reports it: the connection comes up, the
  // channels open (`peer-ready`, once per link), then the host welcomes player 2.
  lobby.setPeerState("host", "connecting");
  lobby.setPeerState("host", "connected");
  lobby.emit("peer-ready", { peerId: "host" });
  lobby.emit("reliable", {
    peerId: "host",
    data: { kind: "welcome", playerId: 2, encodedSnapshot: snapshotJson(1, [1, 2]) },
  });
  await settle();
  expect(runtime.getState()).toMatchObject({ mode: "guest", lifecycle: "running", playerId: 2 });
  const guard = () => hostGuard(lobby.commandsTo("host"));
  return {
    runtime,
    lobby,
    guard,
    // The link to the host drops. The host keeps the guest's player (no
    // `participant-disconnected`); reliable sends to the host now throw.
    loseLink(state = "disconnected") {
      lobby.setPeerState("host", state);
    },
    // The link recovers in place: ICE restart (when it had failed) and back to
    // `connected`, with no `peer-ready` and no new welcome.
    async recover({ viaIceRestart = false } = {}) {
      if (viaIceRestart) {
        lobby.emit("peer-recovery", {
          peerId: "host",
          attempt: 1,
          action: "request",
          requested: false,
        });
        lobby.setPeerState("host", "connecting");
      }
      lobby.setPeerState("host", "connected");
      await settle();
      expect(runtime.getState()).toMatchObject({ lifecycle: "running", playerId: 2 });
    },
  };
}

test("peer guest: a guard released while the link was down is lowered on the host after an in-place recovery", async () => {
  const fixture = await guestFixture();
  fixture.runtime.setGuard("keyboard", true);
  expect(fixture.guard()).toBe(true);

  fixture.loseLink("disconnected");
  const refusedBefore = fixture.lobby.refused;
  fixture.runtime.setGuard("keyboard", false);
  // The release could not be delivered: the host still guards.
  expect(fixture.lobby.refused).toBeGreaterThan(refusedBefore);
  expect(fixture.guard()).toBe(true);

  await fixture.recover();

  // Nobody holds the key: the host must not keep a raised guard.
  expect(fixture.guard()).toBe(false);
  // A later press raises the guard again from the released state.
  fixture.runtime.setGuard("keyboard", true);
  expect(fixture.guard()).toBe(true);
  fixture.runtime.dispose();
});

test("peer guest: a guard released while the link had failed is lowered after ICE-restart recovery", async () => {
  const fixture = await guestFixture();
  fixture.runtime.setGuard("keyboard", true);
  expect(fixture.guard()).toBe(true);

  fixture.loseLink("disconnected");
  fixture.loseLink("failed");
  fixture.runtime.setGuard("keyboard", false);
  expect(fixture.guard()).toBe(true);

  await fixture.recover({ viaIceRestart: true });

  expect(fixture.guard()).toBe(false);
  fixture.runtime.dispose();
});

test("peer guest: a guard held through an in-place recovery stays raised on the host", async () => {
  const fixture = await guestFixture();
  fixture.runtime.setGuard("keyboard", true);
  expect(fixture.guard()).toBe(true);
  const beforeOutage = fixture.lobby.commandsTo("host").length;

  fixture.loseLink("disconnected");
  await fixture.recover();

  // No phantom drop: the host still guards and was never told to lower.
  expect(fixture.guard()).toBe(true);
  expect(
    fixture.lobby
      .commandsTo("host")
      .slice(beforeOutage)
      .some((command) => command.type === "setGuard" && !command.raised),
  ).toBe(false);
  fixture.runtime.dispose();
});

test("peer guest: a guard pressed while the link was down is raised on the host after an in-place recovery", async () => {
  const fixture = await guestFixture();
  expect(fixture.guard()).toBe(false);

  fixture.loseLink("disconnected");
  fixture.runtime.setGuard("keyboard", true);
  expect(fixture.guard()).toBe(false);

  await fixture.recover();

  // The key is still held: the host must guard.
  expect(fixture.guard()).toBe(true);
  fixture.runtime.dispose();
});

test("peer guest: a link that never dropped causes no re-assertion traffic", async () => {
  const fixture = await guestFixture();
  // The initial connection (`connecting` → `connected` → `peer-ready` → welcome) with
  // nothing held sends nothing.
  expect(fixture.lobby.commandsTo("host")).toEqual([]);

  fixture.runtime.setGuard("keyboard", true);
  fixture.runtime.setGuard("keyboard", false);
  fixture.runtime.setGuard("keyboard", true);
  await settle();

  // Exactly the commands the input produced, delivered in order; nothing refused.
  expect(fixture.lobby.commandsTo("host")).toEqual([
    { type: "setGuard", raised: true },
    { type: "setGuard", raised: false },
    { type: "setGuard", raised: true },
  ]);
  expect(fixture.lobby.refused).toBe(0);
  expect(fixture.guard()).toBe(true);
  fixture.runtime.dispose();
});

// Host-side authority that records which player each command was applied to.
class HostGame {
  players = [];
  commands = [];
  addPlayer(id) {
    this.players.push(id);
  }
  removePlayer(id) {
    this.players = this.players.filter((player) => player !== id);
  }
  applyCommand(playerId, sequence, encoded) {
    this.commands.push({ playerId, sequence, payload: JSON.parse(encoded).payload });
  }
  advanceTick() {}
  snapshotJson() {
    return snapshotJson(1, this.players);
  }
  saveStateJson() {
    return "{}";
  }
  free() {}
}

test("peer host: recovering a guest's link sends no guest commands and changes no player's input", async () => {
  const lobby = new ModelLobby("self");
  const games = [];
  const { runtime } = runtimeWith({
    createGame: () => {
      const game = new HostGame();
      games.push(game);
      return game;
    },
    createPeerSession: () => lobby,
  });
  await runtime.hostPeer("http://setup.test");
  expect(runtime.getState()).toMatchObject({ mode: "host", lifecycle: "running", playerId: 1 });
  const game = games[0];

  // A guest joins and is admitted as player 2; the host's own player guards.
  lobby.setPeerState("guest-a", "connecting");
  lobby.setPeerState("guest-a", "connected");
  lobby.emit("peer-ready", { peerId: "guest-a" });
  expect(game.players).toContain(2);
  runtime.setGuard("keyboard", true);
  const commandsBefore = game.commands.length;

  // The host's link to that guest drops and recovers in place.
  lobby.setPeerState("guest-a", "disconnected");
  lobby.setPeerState("guest-a", "failed");
  lobby.emit("peer-recovery", {
    peerId: "guest-a",
    attempt: 1,
    action: "restart",
    usingTurn: false,
    requested: true,
  });
  lobby.setPeerState("guest-a", "connecting");
  lobby.setPeerState("guest-a", "connected");
  await settle();

  // The host never sends guest commands to anyone ...
  expect(lobby.sent.filter((message) => message.data?.kind === "command")).toEqual([]);
  // ... and no player's input changes at the host authority: neither the guest's player
  // nor the host's own still-held guard.
  expect(game.commands.slice(commandsBefore)).toEqual([]);
  expect(game.players).toContain(2);
  expect(runtime.getState()).toMatchObject({ mode: "host", lifecycle: "running", playerId: 1 });
  runtime.dispose();
});

// Owner decision (#176, Atlas 4.1.2): a guest's held inputs are released as soon as its own
// link to the host closes and recovery gives up.
test("peer guest: a host link whose recovery gave up releases held input and stops sending", async () => {
  const fixture = await guestFixture();
  fixture.runtime.setGuard("keyboard", true);
  fixture.runtime.setHeldMovement("forward", true);
  const sentBefore = fixture.lobby.sent.length;

  fixture.loseLink("failed");
  fixture.lobby.emit("peer-recovery-exhausted", { peerId: "host", attempts: 3 });
  await settle();

  expect(fixture.runtime.getState()).toMatchObject({
    mode: "guest",
    lifecycle: "disconnected",
    playerId: null,
  });
  expect(fixture.lobby.closed).toBe(true);

  // Keys still held or pressed afterwards reach nobody; a refused send would count here.
  const refusedBefore = fixture.lobby.refused;
  fixture.runtime.setHeldMovement("left", true);
  fixture.runtime.setGuard("touch", true);
  fixture.runtime.dispatch({ type: "primaryAttack" });
  await settle();
  expect(fixture.lobby.sent.length).toBe(sentBefore);
  expect(fixture.lobby.refused).toBe(refusedBefore);
  fixture.runtime.dispose();
});

test("peer guest: exhausted recovery of another peer's link is not a loss", async () => {
  const fixture = await guestFixture();
  fixture.lobby.emit("peer-recovery-exhausted", { peerId: "someone-else", attempts: 3 });
  await settle();
  expect(fixture.runtime.getState()).toMatchObject({ lifecycle: "running", playerId: 2 });
  expect(fixture.lobby.closed).toBe(false);
  fixture.runtime.dispose();
});

test("peer guest: a re-join after the loss starts with nothing held", async () => {
  const lobbies = [new ModelLobby("host"), new ModelLobby("host")];
  const sessions = [...lobbies];
  const { runtime } = runtimeWith({ createPeerSession: () => sessions.shift() });
  const join = async (lobby) => {
    await runtime.joinPeer("http://setup.test", "CODE");
    lobby.setPeerState("host", "connected");
    lobby.emit("peer-ready", { peerId: "host" });
    lobby.emit("reliable", {
      peerId: "host",
      data: { kind: "welcome", playerId: 2, encodedSnapshot: snapshotJson(1, [1, 2]) },
    });
    await settle();
  };
  await join(lobbies[0]);
  runtime.setGuard("keyboard", true);
  runtime.setHeldMovement("forward", true);
  expect(hostGuard(lobbies[0].commandsTo("host"))).toBe(true);
  lobbies[0].emit("peer-recovery-exhausted", { peerId: "host", attempts: 3 });
  await settle();

  // The keys are still physically down when the player joins again.
  await join(lobbies[1]);
  expect(runtime.getState()).toMatchObject({ lifecycle: "running", playerId: 2 });
  const commands = lobbies[1].commandsTo("host");
  expect(hostGuard(commands)).toBe(false);
  expect(commands.some((c) => c.type === "setMovement" && (c.x || c.z))).toBe(false);
  // A fresh press applies again.
  runtime.setGuard("keyboard", true);
  expect(hostGuard(lobbies[1].commandsTo("host"))).toBe(true);
  runtime.dispose();
});
