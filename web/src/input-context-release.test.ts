// Acceptance contract for #176: input contexts, capture priority and safe release of held
// inputs. Written independently of the implementation, against the baseline at ea15df2.
//
// Specification: GitHub issue #176 (successor (i) of #57, Atlas 4.1.1) and the #57
// integration decision of 2026-10-11 "release held inputs on authoritative disconnect"
// (#71 decision A: stop acting, remain vulnerable).
//
// Seams and assumptions (documented so an implementer knows what is pinned here):
//
// - Contexts. The existing `createInputAttachment(...).setContext(name)` seam selects the
//   active input context. Besides the existing "gameplay" and "menu", this contract assumes
//   the names "text-entry" and "rebinding-capture" for the two contexts the issue adds.
//   Renaming them is a seam change that needs independent re-verification, not a semantic
//   change. Who decides the context (focus tracking with `isTextEntryTarget`, the rebinding
//   recorder, the settings dialog) is not prescribed here; the browser acceptance in
//   `tests/browser/input-contexts.spec.js` pins the user-observable result.
// - Capture priority, highest first: rebinding-capture, text-entry, menu, gameplay. Only
//   gameplay forwards gameplay actions. Menu forwards only the always-active actions (the
//   settings toggle). Text entry and rebinding capture own the keyboard completely, so not
//   even the settings toggle may act while the player types or records a binding.
// - "Released exactly once" is checked at two levels: the attachment's `onRelease` fires once
//   when gameplay is left (not again for moves between non-gameplay contexts), and the
//   runtime emits each held-intent release command once however often release is requested.
// - Authoritative remote-session loss is the game-client runtime moving to lifecycle
//   "disconnected", which the dedicated session reports once its reconnect grace expired.
//   An in-grace resume ("connecting") is *not* an authoritative loss: the #92 contract in
//   `reconnect-guard.test.ts` and `peer-link-recovery.test.ts` (a key held through a resume
//   stays asserted) remains in force and is not contradicted here.
// - Peer-guest authoritative loss is not covered: the baseline runtime has no peer-guest
//   "disconnected" lifecycle, and choosing which lobby event is authoritative is not settled
//   by the issue. See the handoff comment on #176.
import { expect, test } from "bun:test";
import { createInputAttachment } from "./input-attachment.ts";
import { createGameClientRuntime } from "./game-client-runtime.ts";
import { createSnapshotStore } from "./snapshot-store.js";
import { PROTOCOL_VERSION } from "./wire-protocol.js";

// ---------------------------------------------------------------------------------------
// Context and capture-priority matrix (input attachment).

class Target {
  listeners = new Map();
  visibilityState = "visible";

  addEventListener(type, listener) {
    if (!this.listeners.has(type)) this.listeners.set(type, new Set());
    this.listeners.get(type).add(listener);
  }

  removeEventListener(type, listener) {
    this.listeners.get(type)?.delete(listener);
  }

  dispatch(type) {
    for (const listener of [...(this.listeners.get(type) ?? [])]) listener({ type });
  }
}

function attachmentHarness() {
  const focusTarget = new Target();
  const visibilityTarget = new Target();
  const dispatched = [];
  let controller = null;
  let releases = 0;
  const input = createInputAttachment({
    profile: undefined,
    createController(options) {
      controller = {
        updateProfile() {},
        activeContexts: () => [...options.getActiveContexts()],
        emit: (dispatch) => options.onDispatch(dispatch),
      };
      return controller;
    },
    attachKeyboard: () => () => {},
    onDispatch: (dispatch) => dispatched.push(dispatch),
    onRelease: () => {
      releases += 1;
    },
    focusTarget,
    visibilityTarget,
    alwaysActions: ["game.settings"],
  });
  input.attach();
  return {
    input,
    focusTarget,
    visibilityTarget,
    dispatched,
    controller: () => controller,
    releases: () => releases,
  };
}

const press = (action) => ({ action, phase: "press" });
const release = (action) => ({ action, phase: "release" });

const GAMEPLAY_ACTIONS = [
  "game.moveForward",
  "game.moveRight",
  "game.primaryAttack",
  "game.secondaryAttack",
  "game.guard",
  "game.interact",
  "game.switchWeapon",
  "game.cycleTarget",
];

const NON_GAMEPLAY = ["menu", "text-entry", "rebinding-capture"];

for (const context of NON_GAMEPLAY) {
  test(`${context}: entering it from gameplay releases held input exactly once`, () => {
    const h = attachmentHarness();
    const before = h.releases();

    h.input.setContext(context);
    h.input.setContext(context);

    expect(h.input.getContext()).toBe(context);
    expect(h.releases() - before).toBe(1);
  });

  test(`${context}: gameplay keys pressed or released there dispatch no gameplay command`, () => {
    const h = attachmentHarness();
    h.input.setContext(context);

    expect(h.controller().activeContexts()).not.toContain("gameplay");
    for (const action of GAMEPLAY_ACTIONS) {
      h.controller().emit(press(action));
      h.controller().emit(release(action));
    }

    expect(h.dispatched).toEqual([]);
  });

  test(`${context}: returning to gameplay resumes dispatch without another release`, () => {
    const h = attachmentHarness();
    h.input.setContext(context);
    const afterLeaving = h.releases();

    h.input.setContext("gameplay");

    expect(h.releases()).toBe(afterLeaving);
    expect(h.controller().activeContexts()).toContain("gameplay");
    h.controller().emit(press("game.moveForward"));
    expect(h.dispatched).toEqual([press("game.moveForward")]);
  });
}

test("moving between non-gameplay contexts never releases a second time", () => {
  const h = attachmentHarness();
  const before = h.releases();

  // Gameplay → menu (settings) → a text field in it → rebinding capture → back to the menu.
  h.input.setContext("menu");
  h.input.setContext("text-entry");
  h.input.setContext("rebinding-capture");
  h.input.setContext("menu");

  expect(h.releases() - before).toBe(1);
  expect(h.dispatched).toEqual([]);
});

test("gameplay forwards gameplay actions and the settings toggle", () => {
  const h = attachmentHarness();
  h.controller().emit(press("game.primaryAttack"));
  h.controller().emit(press("game.settings"));
  expect(h.dispatched).toEqual([press("game.primaryAttack"), press("game.settings")]);
});

test("menu forwards only the always-active settings toggle", () => {
  const h = attachmentHarness();
  h.input.setContext("menu");
  h.controller().emit(press("game.primaryAttack"));
  h.controller().emit(press("game.settings"));
  expect(h.dispatched).toEqual([press("game.settings")]);
});

for (const context of ["text-entry", "rebinding-capture"]) {
  test(`${context} has priority over the menu: not even the settings toggle acts`, () => {
    const h = attachmentHarness();
    h.input.setContext("menu");
    h.input.setContext(context);

    h.controller().emit(press("game.settings"));
    h.controller().emit(release("game.settings"));
    h.controller().emit(press("game.primaryAttack"));

    expect(h.dispatched).toEqual([]);
  });
}

test("blur and a hidden page while in a non-gameplay context dispatch no gameplay command", () => {
  const h = attachmentHarness();
  h.input.setContext("text-entry");

  h.focusTarget.dispatch("blur");
  h.visibilityTarget.visibilityState = "hidden";
  h.visibilityTarget.dispatch("visibilitychange");
  h.controller().emit(press("game.moveForward"));

  expect(h.dispatched).toEqual([]);
  expect(h.input.getContext()).toBe("text-entry");
});

// ---------------------------------------------------------------------------------------
// Held-intent release in the game-client runtime.

const LOCAL_REVISION = "0123456789abcdef";
const PLAYER_ID = 1;

const snapshotJson = (tick = 0) =>
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

const decode = (encoded) => JSON.parse(encoded).payload;
const decodeBytes = (payload) => decode(new TextDecoder().decode(payload));

async function settle(rounds = 25) {
  for (let round = 0; round < rounds; round += 1) await Promise.resolve();
}

const HELD_RELEASE_TYPES = new Set(["setMovement", "setGuard", "setAim", "cancelBow"]);

function localFixture() {
  const commands = [];
  const runtime = createGameClientRuntime({
    snapshots: createSnapshotStore(),
    createGame: () => ({
      addPlayer() {},
      applyCommand(_playerId, _sequence, encoded) {
        commands.push(decode(encoded));
      },
      advanceTick() {},
      snapshotJson: () => snapshotJson(),
      saveStateJson: () => "{}",
    }),
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
    setInterval: () => 1,
    clearInterval: () => {},
  });
  runtime.startLocal({ seed: 1 });
  return { runtime, commands };
}

// Holds every kind of continuous intent: keyboard and touch movement, keyboard and touch
// guard, a bow draw and a pointer aim.
function holdEverything(runtime) {
  runtime.setHeldMovement("forward", true);
  runtime.setHeldMovement("right", true);
  runtime.setTouchMovement(0.5, 0);
  runtime.setGuard("keyboard", true);
  runtime.setGuard("touch", true);
  runtime.setBowDraw("touch", true);
  runtime.setAim([300, 400]);
}

test("a release request clears every held intent exactly once, however often it repeats", () => {
  const { runtime, commands } = localFixture();
  holdEverything(runtime);
  const before = commands.length;

  // Blur, then a hidden page, then leaving gameplay: three triggers for one hold.
  runtime.releaseInput();
  runtime.releaseInput();
  runtime.releaseInput();

  const released = commands.slice(before);
  expect(released.filter((command) => command.type === "setMovement")).toEqual([
    { type: "setMovement", x: 0, z: 0 },
  ]);
  expect(released.filter((command) => command.type === "setGuard")).toEqual([
    { type: "setGuard", raised: false },
  ]);
  expect(released.filter((command) => command.type === "setAim")).toEqual([
    { type: "setAim", direction: null },
  ]);
  // An interrupted draw is lowered, never shot.
  expect(released.filter((command) => command.type === "cancelBow")).toHaveLength(1);
  expect(released.some((command) => command.type === "releaseBow")).toBe(false);
  expect(released.every((command) => HELD_RELEASE_TYPES.has(command.type))).toBe(true);
});

test("device releases that arrive after a release request send nothing more", () => {
  const { runtime, commands } = localFixture();
  holdEverything(runtime);
  runtime.releaseInput();
  const before = commands.length;

  // The keys, fingers and pointer come up later (lost capture, keyup after refocus).
  runtime.setHeldMovement("forward", false);
  runtime.setHeldMovement("right", false);
  runtime.setTouchMovement(0, 0);
  runtime.setGuard("keyboard", false);
  runtime.setGuard("touch", false);
  runtime.setBowDraw("touch", false);
  runtime.setAim(null);

  expect(commands.slice(before)).toEqual([]);
});

test("an interrupted touch draw (pointercancel or lost capture) cancels and never shoots", () => {
  const { runtime, commands } = localFixture();
  runtime.setBowDraw("touch", true);
  const before = commands.length;

  runtime.setBowDraw("touch", false, { interrupted: true });

  expect(commands.slice(before)).toEqual([{ type: "cancelBow" }]);
});

test("a committed one-shot is not erased or repeated by a release", () => {
  const { runtime, commands } = localFixture();
  runtime.setHeldMovement("forward", true);
  runtime.dispatch({ type: "primaryAttack" });
  const afterAttack = commands.length;

  runtime.releaseInput();

  expect(commands.filter((command) => command.type === "primaryAttack")).toHaveLength(1);
  expect(commands.slice(afterAttack).every((command) => HELD_RELEASE_TYPES.has(command.type))).toBe(
    true,
  );
});

// ---------------------------------------------------------------------------------------
// Authoritative remote-session loss (dedicated): the runtime moves to "disconnected".

// A dedicated session as the runtime sees it. `expire()` is what the real
// `DedicatedGameSession` does once its reconnect grace expired: it rejects the reliable
// commands it still buffered and reports "disconnected"; afterwards every send is refused
// with "Dedicated session is not connected".
class ModelDedicatedSession {
  callbacks = null;
  // Every send attempt, accepted or refused.
  attempts = [];
  // Reliable commands waiting for the authority's acknowledgement.
  pending = [];
  connected = true;
  closed = false;

  async connect(callbacks) {
    this.callbacks = callbacks;
    callbacks.onStateChange?.("connecting");
    await Promise.resolve();
    callbacks.onStateChange?.("connected");
    callbacks.onWelcome?.({ playerId: PLAYER_ID, tickHz: 60 });
    return {};
  }

  async sendCommand(sequence, payload) {
    this.attempts.push({ lane: "datagram", sequence, command: decodeBytes(payload) });
    if (!this.connected) throw new Error("Dedicated session is not connected");
  }

  sendReliableCommand(sequence, payload) {
    this.attempts.push({ lane: "reliable", sequence, command: decodeBytes(payload) });
    if (!this.connected) return Promise.reject(new Error("Dedicated session is not connected"));
    return new Promise((resolve, reject) => {
      this.pending.push({ sequence, resolve, reject });
    });
  }

  acknowledgeAll() {
    for (const entry of this.pending.splice(0)) entry.resolve("applied");
  }

  // The in-grace resume loop started: not an authoritative loss.
  interrupt() {
    this.callbacks.onStateChange?.("connecting");
  }

  expire() {
    this.connected = false;
    for (const entry of this.pending.splice(0)) {
      entry.reject(
        new Error("Dedicated reconnect grace expired before the command was acknowledged"),
      );
    }
    this.callbacks.onStateChange?.("disconnected");
  }

  close() {
    this.closed = true;
    this.connected = false;
  }
}

async function dedicatedFixture() {
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
      const session = new ModelDedicatedSession();
      sessions.push(session);
      return session;
    },
    supportsWebTransport: () => true,
    freshRunSeed: () => 1,
    localContentRevision: () => LOCAL_REVISION,
    onStatus: (status) => statuses.push(status),
    // The held re-assertion timer never fires on its own here.
    setInterval: () => 1,
    clearInterval: () => {},
  });
  const connect = async () => {
    await runtime.startDedicated("https://server.test/arpg");
    await settle();
    expect(runtime.getState()).toMatchObject({ lifecycle: "running", playerId: PLAYER_ID });
    return sessions.at(-1);
  };
  const session = await connect();
  return { runtime, sessions, statuses, session, connect };
}

const failures = (statuses) => statuses.filter((status) => status.includes("command failed"));

test("dedicated: after an authoritative loss mid-hold, no input reaches the lost session", async () => {
  const { runtime, session, statuses } = await dedicatedFixture();
  holdEverything(runtime);
  await settle();
  const before = session.attempts.length;

  session.expire();
  await settle();
  expect(runtime.getState().lifecycle).toBe("disconnected");
  // The session itself reports the reliable commands its expired grace dropped; anything
  // reported after this point comes from input applied to the lost session.
  const failuresBefore = failures(statuses).length;

  // Releasing what was held, pressing again and trying one-shots while disconnected.
  runtime.setHeldMovement("forward", false);
  runtime.setHeldMovement("right", false);
  runtime.setTouchMovement(0, 0);
  runtime.setGuard("keyboard", false);
  runtime.setGuard("touch", false);
  runtime.setBowDraw("touch", false);
  runtime.setAim(null);
  runtime.setHeldMovement("left", true);
  runtime.setGuard("keyboard", true);
  runtime.setAim([0, 1000]);
  runtime.releaseInput();
  await settle();

  // Input does not apply again until a reconnect: nothing is even attempted, so the player
  // sees the disconnect, not a stream of failed commands.
  expect(session.attempts.slice(before)).toEqual([]);
  expect(failures(statuses).slice(failuresBefore)).toEqual([]);
});

test("dedicated: a reconnect after a loss mid-hold starts with nothing held and accepts fresh input", async () => {
  const { runtime, session, connect } = await dedicatedFixture();
  holdEverything(runtime);
  await settle();

  session.expire();
  await settle();
  // The player reconnects while the keys, fingers and pointer are still down.
  const resumed = await connect();
  await settle();

  const raised = (attempt) =>
    (attempt.command.type === "setGuard" && attempt.command.raised) ||
    (attempt.command.type === "setMovement" && (attempt.command.x || attempt.command.z)) ||
    (attempt.command.type === "setAim" && attempt.command.direction) ||
    attempt.command.type === "drawBow" ||
    attempt.command.type === "releaseBow";
  expect(resumed.attempts.filter(raised)).toEqual([]);

  // Devices from before the loss come up after the reconnect: no stuck or phantom intent.
  runtime.setHeldMovement("forward", false);
  runtime.setHeldMovement("right", false);
  runtime.setTouchMovement(0, 0);
  runtime.setGuard("keyboard", false);
  runtime.setGuard("touch", false);
  runtime.setBowDraw("touch", false);
  await settle();
  expect(resumed.attempts.filter(raised)).toEqual([]);
  expect(resumed.attempts.some((attempt) => attempt.command.type === "releaseBow")).toBe(false);

  // A fresh press after the reconnect applies again.
  runtime.setGuard("keyboard", true);
  runtime.setHeldMovement("forward", true);
  await settle();
  expect(resumed.attempts.map((attempt) => attempt.command)).toEqual(
    expect.arrayContaining([
      { type: "setGuard", raised: true },
      { type: "setMovement", x: 0, z: -1 },
    ]),
  );
  runtime.dispose();
});

test("dedicated: releasing held input never drops an unacknowledged reliable one-shot", async () => {
  const { runtime, session, statuses } = await dedicatedFixture();
  runtime.setHeldMovement("forward", true);
  runtime.dispatch({ type: "primaryAttack" });
  await settle();
  expect(session.pending).toHaveLength(1);

  // Blur, a menu and an in-grace interruption all release or pause held input only.
  runtime.releaseInput();
  session.interrupt();
  await settle();

  expect(session.closed).toBe(false);
  expect(session.pending).toHaveLength(1);
  session.acknowledgeAll();
  await settle();
  expect(failures(statuses)).toEqual([]);
  expect(
    session.attempts.filter((attempt) => attempt.command.type === "primaryAttack"),
  ).toHaveLength(1);
  runtime.dispose();
});
