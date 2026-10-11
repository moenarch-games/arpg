// Acceptance contract for Rust/Wasm core loading as game-client runtime state (#88).
//
// Loading the gameplay core (today `initWasm()`, later also the minimum encounter assets for
// the #145 preparation screen) moves from a React effect into `createGameClientRuntime`.
// The runtime reports it as state, so a shell can show honest loading / ready / failed
// status with retry, and a load that completes after leave, unmount or dispose can never
// activate an authority or tick loop.
//
// API additions to `web/src/game-client-runtime.ts`:
//
//   GameClientRuntimeOptions.loadCore?: () => Promise<unknown>
//     Loads everything `createGame` needs. Omitted: the core counts as ready immediately,
//     so existing compositions and tests keep their synchronous behaviour.
//
//   ClientState.core: "unloaded" | "loading" | "ready" | "failed"
//   ClientState.coreError: string | null   // set exactly while core === "failed"
//
//   runtime.load(): Promise<void>
//     Starts a load attempt, or joins the one in flight. Never rejects: a failed attempt
//     resolves after the state became core: "failed". Calling it again after a failure is
//     the retry. A no-op once ready or after dispose.
//
// Behaviour:
// - `startLocal(...)` (normal play and the training route) before the core is ready begins
//   or joins a load, reports lifecycle "starting" for that request and creates no game and
//   no tick loop yet. When the load succeeds and the request is still the current one, the
//   authority starts exactly as a synchronous `startLocal` would.
// - `stop()`, `dispose()` or a newer start before the load resolves supersede the pending
//   request: a late success never calls `createGame` for it, never schedules a tick and
//   never publishes a snapshot. Only the latest request is activated.
// - A failed load sets core "failed" with the error message, puts a pending start into
//   lifecycle "failed", reports a status, and never throws or rejects into the caller.
//   `startLocal` or `load()` after a failure starts a new attempt.
// - After dispose a late success or failure does not change state or notify listeners.
import { expect, test } from "bun:test";
import { createGameClientRuntime } from "./game-client-runtime.ts";
import { PROTOCOL_VERSION } from "./wire-protocol.js";
import { createSnapshotStore } from "./snapshot-store.js";

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

function deferred() {
  let resolve;
  let reject;
  const promise = new Promise((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

// Lets the runtime's promise continuations run.
const settle = async () => {
  for (let round = 0; round < 5; round += 1) await Promise.resolve();
};

class FakeGame {
  tick = 0;
  freed = false;
  players = [];
  constructor(seed, scenario) {
    this.seed = seed;
    this.scenario = scenario;
  }
  addPlayer(id) {
    this.players.push(id);
  }
  applyCommand() {
    if (this.freed) throw new Error("used after free");
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
  free() {
    this.freed = true;
  }
}

function harness() {
  const games = [];
  const loads = [];
  const timers = new Map();
  const statuses = [];
  const published = [];
  let nextTimer = 1;
  let nextSeed = 100;
  const store = createSnapshotStore();
  const snapshots = {
    getSnapshot: () => store.getSnapshot(),
    subscribe: (listener) => store.subscribe(listener),
    publish(next) {
      published.push(next);
      store.publish(next);
    },
  };
  const unused = (name) => () => {
    throw new Error(`${name} is not part of this contract`);
  };
  const runtime = createGameClientRuntime({
    snapshots,
    loadCore: () => {
      const load = deferred();
      loads.push(load);
      return load.promise;
    },
    createGame: (seed, scenario) => {
      const game = new FakeGame(seed, scenario);
      games.push(game);
      return game;
    },
    createPeerSession: unused("createPeerSession"),
    attachPeerSession: unused("attachPeerSession"),
    createDedicatedSession: unused("createDedicatedSession"),
    supportsWebTransport: () => true,
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
  let notifications = 0;
  runtime.subscribe(() => {
    notifications += 1;
  });
  return {
    runtime,
    games,
    loads,
    timers,
    statuses,
    published,
    snapshots,
    notifications: () => notifications,
  };
}

test("the core starts unloaded and load reports loading then ready", async () => {
  const { runtime, loads } = harness();
  expect(runtime.getState()).toMatchObject({ core: "unloaded", coreError: null });

  const loading = runtime.load();
  expect(runtime.getState().core).toBe("loading");
  expect(loads).toHaveLength(1);

  loads[0].resolve();
  await loading;
  expect(runtime.getState()).toMatchObject({ core: "ready", coreError: null });
  expect(runtime.getState().lifecycle).toBe("idle");
});

test("concurrent and repeated loads share one attempt", async () => {
  const { runtime, loads } = harness();
  const first = runtime.load();
  const second = runtime.load();
  expect(loads).toHaveLength(1);

  loads[0].resolve();
  await Promise.all([first, second]);
  await runtime.load();
  expect(loads).toHaveLength(1);
});

test("normal play waits for the core and then starts one authority", async () => {
  const { runtime, games, loads, timers, snapshots } = harness();
  runtime.startLocal({ seed: 9 });

  expect(loads).toHaveLength(1);
  expect(runtime.getState()).toMatchObject({ lifecycle: "starting", mode: "local" });
  expect(games).toHaveLength(0);
  expect(timers.size).toBe(0);
  expect(runtime.hasAuthority()).toBe(false);

  loads[0].resolve();
  await settle();

  expect(games.map((game) => game.seed)).toEqual([9]);
  expect(timers.size).toBe(1);
  expect(runtime.hasTickLoop()).toBe(true);
  expect(runtime.getState()).toMatchObject({ lifecycle: "running", core: "ready", playerId: 1 });
  expect(snapshots.getSnapshot()).not.toBeNull();
});

test("the training route waits for the core and starts its named scenario", async () => {
  const { runtime, games, loads } = harness();
  runtime.startLocal({ seed: 42, training: true, scenario: "dummy" });
  expect(runtime.getState().lifecycle).toBe("starting");

  loads[0].resolve();
  await settle();

  expect(games).toHaveLength(1);
  expect(games[0]).toMatchObject({ seed: 42, scenario: "dummy" });
  expect(runtime.getState()).toMatchObject({
    lifecycle: "running",
    training: { paused: false, speed: 1, scenario: "dummy" },
  });
});

test("leaving before a slow load resolves never activates the authority", async () => {
  const { runtime, games, loads, timers, published } = harness();
  runtime.startLocal({ seed: 1 });
  runtime.stop();
  expect(runtime.getState().lifecycle).toBe("idle");

  loads[0].resolve();
  await settle();

  expect(games).toHaveLength(0);
  expect(timers.size).toBe(0);
  expect(runtime.hasAuthority()).toBe(false);
  expect(published.filter((snapshot) => snapshot !== null)).toEqual([]);
  expect(runtime.getState()).toMatchObject({ lifecycle: "idle", core: "ready" });
});

test("disposing before a slow load resolves never activates a simulation", async () => {
  const { runtime, games, loads, timers, published, notifications } = harness();
  runtime.startLocal({ seed: 1, training: true });
  runtime.dispose();
  const notified = notifications();

  loads[0].resolve();
  await settle();

  expect(games).toHaveLength(0);
  expect(timers.size).toBe(0);
  expect(published.filter((snapshot) => snapshot !== null)).toEqual([]);
  expect(runtime.getState().lifecycle).toBe("disposed");
  expect(notifications()).toBe(notified);

  runtime.startLocal({ seed: 2 });
  await runtime.load();
  await settle();
  expect(loads).toHaveLength(1);
  expect(games).toHaveLength(0);
});

test("re-entering during a load activates only the latest request", async () => {
  const { runtime, games, loads, timers } = harness();
  // StrictMode-style unmount/remount and a double activation of Start.
  runtime.startLocal({ seed: 1 });
  runtime.stop();
  runtime.startLocal({ seed: 2 });
  runtime.startLocal({ seed: 3, training: true, scenario: "ranged" });

  expect(loads).toHaveLength(1);
  loads[0].resolve();
  await settle();

  expect(games.map((game) => game.seed)).toEqual([3]);
  expect(timers.size).toBe(1);
  expect(runtime.getState()).toMatchObject({
    lifecycle: "running",
    training: { scenario: "ranged" },
  });
});

test("a failed load is a usable failed state, not a rejection", async () => {
  const { runtime, games, loads, timers, statuses } = harness();
  runtime.startLocal({ seed: 1 });
  const loading = runtime.load();

  loads[0].reject(new Error("wasm fetch 404"));
  await expect(loading).resolves.toBeUndefined();
  await settle();

  expect(runtime.getState()).toMatchObject({ lifecycle: "failed", core: "failed" });
  expect(runtime.getState().coreError).toContain("wasm fetch 404");
  expect(statuses.some((status) => status.includes("wasm fetch 404"))).toBe(true);
  expect(games).toHaveLength(0);
  expect(timers.size).toBe(0);
});

test("retrying after a failure loads again and can then start normally", async () => {
  const { runtime, games, loads } = harness();
  const failed = runtime.load();
  loads[0].reject(new Error("offline"));
  await failed;
  expect(runtime.getState().core).toBe("failed");

  const retry = runtime.load();
  expect(loads).toHaveLength(2);
  expect(runtime.getState()).toMatchObject({ core: "loading", coreError: null });
  loads[1].resolve();
  await retry;
  expect(runtime.getState().core).toBe("ready");

  runtime.startLocal({ seed: 5 });
  expect(games.map((game) => game.seed)).toEqual([5]);
  expect(runtime.getState().lifecycle).toBe("running");
});

test("starting after a failure begins a new load attempt", async () => {
  const { runtime, games, loads } = harness();
  runtime.startLocal({ seed: 1 });
  loads[0].reject(new Error("offline"));
  await settle();
  expect(runtime.getState().lifecycle).toBe("failed");

  runtime.startLocal({ seed: 2 });
  expect(loads).toHaveLength(2);
  expect(runtime.getState()).toMatchObject({ lifecycle: "starting", core: "loading" });
  loads[1].resolve();
  await settle();

  expect(games.map((game) => game.seed)).toEqual([2]);
  expect(runtime.getState().lifecycle).toBe("running");
});

test("a late failure after dispose changes nothing", async () => {
  const { runtime, loads, statuses, notifications } = harness();
  runtime.startLocal({ seed: 1 });
  runtime.dispose();
  const notified = notifications();
  const reported = statuses.length;

  loads[0].reject(new Error("late"));
  await settle();

  expect(runtime.getState().lifecycle).toBe("disposed");
  expect(notifications()).toBe(notified);
  expect(statuses).toHaveLength(reported);
});

test("once ready, starts stay synchronous", async () => {
  const { runtime, games, loads } = harness();
  const loading = runtime.load();
  loads[0].resolve();
  await loading;

  for (let round = 0; round < 3; round += 1) {
    runtime.startLocal({ seed: round });
    expect(runtime.getState().lifecycle).toBe("running");
    runtime.stop();
  }
  expect(loads).toHaveLength(1);
  expect(games).toHaveLength(3);
  expect(games.every((game) => game.freed)).toBe(true);
});

test("without loadCore the core is ready immediately", () => {
  const runtime = createGameClientRuntime({
    snapshots: createSnapshotStore(),
    createGame: (seed) => new FakeGame(seed, undefined),
    createPeerSession: () => {
      throw new Error("unused");
    },
    attachPeerSession: () => () => {},
    createDedicatedSession: () => {
      throw new Error("unused");
    },
    supportsWebTransport: () => true,
    freshRunSeed: () => 1,
    localContentRevision: () => LOCAL_REVISION,
    onStatus: () => {},
    setInterval: () => 1,
    clearInterval: () => {},
  });
  expect(runtime.getState()).toMatchObject({ core: "ready", coreError: null });
  runtime.startLocal({ seed: 1 });
  expect(runtime.getState().lifecycle).toBe("running");
  runtime.dispose();
});
