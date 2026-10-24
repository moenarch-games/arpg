// Browser game-client runtime: owns the single active gameplay source (local Rust/Wasm
// authority, peer host/guest or dedicated server), command sequencing, held input and the
// fixed-tick loop outside React. React subscribes to `getState()` for bounded shell changes
// and to the snapshot store for per-tick views; it never reaches into sources directly.
//
// Lifecycle: idle → starting → running ⇄ disconnected (dedicated grace expiry, or a peer
// guest's host link whose recovery gave up); any active state → failed; stop()
// returns to idle and dispose() is terminal.

// Core loading: the Rust/Wasm gameplay core (`loadCore`) is runtime state, unloaded → loading
// → ready, or failed with a retry through `load()`. A local start or host before the core is
// ready reports "starting" and activates once the load succeeds, but only while it is still
// the latest request: leaving, a newer start or dispose supersedes it. Every source replacement advances a generation;
// late async completions from an older generation are ignored and cannot mutate the
// replacement.
//
// Background tabs: the tick loop uses the host interval timer. Browsers throttle it in
// hidden tabs, so local simulation slows rather than catching up in a burst; simulation
// time is never fast-forwarded to wall-clock time.

import { contentRevisionMismatch, decodeSnapshot, encodeCommand } from "./wire-protocol.js";
import { DEFAULT_SCENARIO, trainingTicksForFrame } from "./training-arena.js";
import { menuPausesWorld } from "./game-menu.js";
import { t } from "./messages.ts";

export const TICK_INTERVAL_MS = 1000 / 60;

export type ClientMode = "local" | "host" | "guest" | "dedicated";
export type ClientLifecycle =
  | "idle"
  | "starting"
  | "running"
  | "disconnected"
  | "failed"
  | "disposed";

export interface TrainingState {
  paused: boolean;
  speed: number;
  // Named arpg-core workbench scenario the authority was created from.
  scenario: string;
}

export type CoreLoadState = "unloaded" | "loading" | "ready" | "failed";

export interface ClientState {
  lifecycle: ClientLifecycle;
  // Whether the gameplay core `createGame` needs is loaded.
  core: CoreLoadState;
  // The last load failure, set exactly while `core` is "failed".
  coreError: string | null;
  mode: ClientMode;
  playerId: number | null;
  lobbyCode: string;
  // Advances whenever a new authority or remote source replaces the previous one.
  sourceGeneration: number;
  training: TrainingState | null;
}

// arpg-core `WorkbenchOperation`: applied by the local training authority only (#126).
export type WorkbenchOperation =
  | { type: "spawnMonster"; definition: string; offset: [number, number] }
  | { type: "removeMonster"; monsterId: number }
  | { type: "resetArrangement" }
  | { type: "setTuning"; parameter: string; value: number };

export interface TuningValue {
  parameter: string;
  value: number;
}

export type MovementKey = "forward" | "backward" | "left" | "right";
export type GuardSource = "keyboard" | "touch";

export interface WasmGameLike {
  addPlayer(id: number): void;
  removePlayer?(id: number): void;
  applyCommand(playerId: number, sequence: number, encoded: string): void;
  advanceTick(): void;
  snapshotJson(): string;
  saveStateJson(): string;
  // Recorded workbench session as portable `Reproduction` JSON (scenario games only).
  reproductionJson?(): string;
  // Workbench-only authority operations and current tuning (scenario games only).
  applyWorkbench?(encodedOperation: string): void;
  tuningJson?(): string;
  workbenchRoomId?(): number | undefined;
  free?(): void;
}

export interface PeerSessionLike {
  hostParticipantId?: string | null;
  host(maxParticipants: number): Promise<{ displayCode: string }>;
  join(code: string): Promise<unknown>;
  sendReliable(peerId: string, data: unknown): void;
  broadcastRealtime(data: unknown): void;
  close(): void;
}

export interface DedicatedSessionLike {
  connect(callbacks: {
    onWelcome?: (welcome: { playerId: number; tickHz: number }) => void;
    onSnapshot?: (frame: { payload: Uint8Array }) => void;
    onStateChange?: (state: string) => void;
    onError?: (error: unknown) => void;
  }): Promise<unknown>;
  // Held state: a best-effort datagram, repaired by the held refresh.
  sendCommand(sequence: number, payload: Uint8Array): Promise<unknown>;
  // One-shot: the reliable command lane. Settles once the authority acknowledged it; rejects
  // for a rejected command or a session that ended first.
  sendReliableCommand(sequence: number, payload: Uint8Array): Promise<unknown>;
  close(): void;
}

export interface SnapshotSink {
  publish(snapshot: unknown): void;
}

export interface GameClientRuntimeOptions {
  snapshots: SnapshotSink;
  // Loads everything `createGame` needs (the Rust/Wasm core). Omitted: ready immediately.
  loadCore?: () => Promise<unknown>;
  // `scenario` is set exactly for training sessions, which record a reproduction.
  createGame(seed: number, scenario?: string): WasmGameLike;
  createPeerSession(apiBase: string): PeerSessionLike;
  attachPeerSession(options: {
    session: PeerSessionLike;
    role: "host" | "guest";
    isCurrent: () => boolean;
    getGame: () => WasmGameLike | null;
    onPlayer: (playerId: number) => void;
    onSnapshot: (snapshot: unknown) => void;
    onStatus: (status: string) => void;
    localContentRevision: () => string;
    // The host runs another content bundle: the guest must stop.
    onIncompatible: (reason: string) => void;
    // Guest: the host link recovered in place, keeping the assigned player. Returns false
    // when the re-assertion could not be sent yet, so the adapter retries it.
    onLinkRecovered?: () => boolean;
    // Guest: the host link closed and its recovery gave up (authoritative loss, #176).
    onHostLinkLost?: () => void;
  }): () => void;
  createDedicatedSession(endpoint: string): DedicatedSessionLike;
  supportsWebTransport(): boolean;
  freshRunSeed(): number;
  // Content revision of this build's Rust/Wasm gameplay; read once a remote snapshot arrives.
  localContentRevision(): string;
  onStatus(status: string): void;
  setInterval?: (callback: () => void, ms: number) => unknown;
  clearInterval?: (handle: unknown) => void;
}

export interface RestoredGame {
  game: WasmGameLike;
  controlledPlayerId: number;
  lastSequence: number;
  movement: [number, number];
  // Whether the saved authority still holds the guard; this client holds nothing yet.
  guardHeld?: boolean;
  // Whether the saved authority is mid-draw; this client holds no draw yet.
  drawHeld?: boolean;
  // Whether the saved authority holds aim intent; this client points nowhere yet.
  aimHeld?: boolean;
}

// A semantic aim direction for the authority (components within ±1000), or null for the
// default committed facing.
export type AimDirection = [number, number] | null;

const textEncoder = new TextEncoder();
const textDecoder = new TextDecoder();

function idleMovement() {
  return {
    forward: false,
    backward: false,
    left: false,
    right: false,
    touchX: 0,
    touchZ: 0,
    mouseX: 0,
    mouseZ: 0,
    lastX: 0,
    lastZ: 0,
  };
}

type HeldInput = "guard" | "movement" | "aim";
const HELD_INPUTS: readonly HeldInput[] = ["guard", "movement", "aim"];
// Dedicated held-input re-assertion cadence: 5 per second per input, for as long as the
// session runs. Every held datagram carries one input under the datagram lane's sequence
// watermark, so a reordered newer datagram can discard an older one; only a refresh that
// never stops guarantees that each input's current value eventually lands as the newest
// command.
const HELD_REFRESH_INTERVAL_MS = 200;
// Held commands carry an absolute value the authority can take again at any time, so they
// travel as dedicated datagrams and the refresh re-sends them. Every other command travels
// on the reliable command lane, which has its own watermark, so no datagram can overtake
// and drop it (#143): one-shots (attack, interact, weapon or target change) and the bow
// transitions. A release consumes the draw it shoots, so a lost release followed by a new
// draw would merge two shots into one; on the lane draw, release and cancel stay lossless
// and in order.
const HELD_COMMAND_TYPES = new Set(["setGuard", "setMovement", "setAim"]);
// One-shots that commit a direction carry the current aim (#156). In dedicated play the
// `setAim` datagram and the reliable one-shot are not ordered against each other, so a lost
// or late aim datagram must not steer the shot or strike. arpg-core validates the embedded
// aim and applies it as the held aim before the action. Without a current aim the field is
// left out, and the authority keeps using its held aim.
const AIMED_COMMAND_TYPES = new Set(["primaryAttack", "secondaryAttack", "releaseBow"]);

const sameAim = (a: AimDirection, b: AimDirection) =>
  a === b || (!!a && !!b && a[0] === b[0] && a[1] === b[1]);

export function createGameClientRuntime(options: GameClientRuntimeOptions) {
  const schedule = options.setInterval ?? ((callback, ms) => globalThis.setInterval(callback, ms));
  const cancel = options.clearInterval ?? ((handle) => globalThis.clearInterval(handle as never));
  const listeners = new Set<() => void>();
  let state: ClientState = {
    lifecycle: "idle",
    core: options.loadCore ? "unloaded" : "ready",
    coreError: null,
    mode: "local",
    playerId: 1,
    lobbyCode: "",
    sourceGeneration: 0,
    training: null,
  };
  let game: WasmGameLike | null = null;
  // The game menu is open (#60). Only a local authority pauses for it: a hosted, guest or
  // dedicated world is shared and keeps running.
  let menuPaused = false;
  let peer: PeerSessionLike | null = null;
  let detachPeer: (() => void) | null = null;
  let dedicated: DedicatedSessionLike | null = null;
  let sequence = 0;
  let timer: unknown = null;
  let trainingCarry = 0;
  let movement = idleMovement();
  // Every device currently holding guard; the authority sees one held flag.
  const guardSources = new Set<GuardSource>();
  // Every device currently holding the bow draw. The last deliberate release shoots; an
  // interrupted hold (pointer cancel, focus loss) only cancels.
  const drawSources = new Set<GuardSource>();
  // The aim the player is pointing now, which aimed one-shots embed, and the aim that last
  // reached the authority (#159). They differ while a session is still (re)connecting.
  let aim: AimDirection = null;
  let sentAim: AimDirection = null;
  // Whether a raised guard ever reached the current source's authority, which may then
  // still hold it after a press or release was lost.
  let guardRaiseSent = false;
  // Dedicated held commands travel as unreliable datagrams, so held input is re-asserted on a
  // bounded cadence: indefinitely while it differs from rest, and for a few periods after
  // it returns to rest so a lost release still converges (#139).
  let heldRefreshTimer: unknown = null;
  let heldRefreshRotation = 0;
  // The core load in flight, and the latest start waiting for it. A request only activates
  // while its source generation is still current.
  let coreLoad: Promise<void> | null = null;
  let pendingStart: {
    generation: number;
    activate: (() => void) | null;
  } | null = null;

  const update = (patch: Partial<ClientState>) => {
    let changed = false;
    for (const [key, value] of Object.entries(patch)) {
      if (!Object.is(state[key], value)) changed = true;
    }
    if (!changed) return;
    state = { ...state, ...patch };
    for (const listener of listeners) listener();
  };

  const disposed = () => state.lifecycle === "disposed";
  const status = (message: string) => {
    if (!disposed()) options.onStatus(message);
  };

  const stopLoop = () => {
    if (timer === null) return;
    cancel(timer);
    timer = null;
  };

  const publishFromGame = () => {
    if (!game) return;
    const encoded = game.snapshotJson();
    options.snapshots.publish(decodeSnapshot(encoded));
    if (state.mode === "host") peer?.broadcastRealtime({ kind: "snapshot", encoded });
  };

  const fail = (message: string) => {
    stopLoop();
    update({ lifecycle: "failed" });
    status(message);
  };

  const advance = (ticks: number) => {
    try {
      // Publish every tick: snapshots carry that tick's transient events (strikes,
      // interactions), so batching ticks would silently drop them. Drawing stays
      // coalesced per animation frame.
      for (let tick = 0; tick < ticks; tick += 1) {
        game.advanceTick();
        publishFromGame();
      }
    } catch (error) {
      fail(`Simulation stopped: ${error}`);
    }
  };

  const onTimer = () => {
    if (!game || state.mode === "guest" || state.mode === "dedicated") return;
    if (menuPaused && menuPausesWorld(state.mode)) return;
    let ticks = 1;
    if (state.training) {
      if (state.training.paused) return;
      const scheduled = trainingTicksForFrame(state.training.speed, trainingCarry);
      trainingCarry = scheduled.carry;
      ticks = scheduled.ticks;
      if (ticks === 0) return;
    }
    advance(ticks);
  };

  const startLoop = () => {
    stopLoop();
    timer = schedule(onTimer, TICK_INTERVAL_MS);
  };

  const stopHeldRefresh = () => {
    if (heldRefreshTimer === null) return;
    cancel(heldRefreshTimer);
    heldRefreshTimer = null;
  };

  // Releases every resource owned by the current source and invalidates its callbacks.
  const teardown = () => {
    stopLoop();
    stopHeldRefresh();
    detachPeer?.();
    detachPeer = null;
    const peerSession = peer;
    const dedicatedSession = dedicated;
    peer = null;
    dedicated = null;
    peerSession?.close();
    dedicatedSession?.close();
    game?.free?.();
    game = null;
    sequence = 0;
    trainingCarry = 0;
    movement = idleMovement();
    guardSources.clear();
    drawSources.clear();
    aim = null;
    sentAim = null;
    guardRaiseSent = false;
    heldRefreshRotation = 0;
  };

  // Replaces the current source. The returned token identifies the new generation.
  const replaceSource = (patch: Partial<ClientState>) => {
    teardown();
    const generation = state.sourceGeneration + 1;
    update({
      lobbyCode: "",
      training: null,
      ...patch,
      sourceGeneration: generation,
    });
    return generation;
  };
  const isCurrent = (generation: number) => !disposed() && state.sourceGeneration === generation;

  // Starts a core load attempt or joins the one in flight. Never rejects.
  const loadCore = (): Promise<void> => {
    if (disposed() || state.core === "ready") return Promise.resolve();
    if (coreLoad) return coreLoad;
    update({ core: "loading", coreError: null });
    let loading: Promise<unknown>;
    try {
      loading = Promise.resolve(options.loadCore!());
    } catch (error) {
      loading = Promise.reject(error);
    }
    const attempt = loading.then(
      () => {
        if (coreLoad === attempt) coreLoad = null;
        if (disposed()) return;
        update({ core: "ready", coreError: null });
        const request = pendingStart;
        pendingStart = null;
        if (request && isCurrent(request.generation)) request.activate?.();
      },
      (error) => {
        if (coreLoad === attempt) coreLoad = null;
        if (disposed()) return;
        const message = error instanceof Error ? error.message : String(error);
        update({ core: "failed", coreError: message });
        const request = pendingStart;
        pendingStart = null;
        if (request && isCurrent(request.generation)) {
          stopLoop();
          update({ lifecycle: "failed" });
        }
        status(t("status.loadError", { error: message }));
      },
    );
    coreLoad = attempt;
    return attempt;
  };

  // Replaces the current source with one that waits for the core; `activate` runs once the
  // load succeeds while this request is still the latest.
  const deferUntilCoreReady = (patch: Partial<ClientState>, activate: (() => void) | null) => {
    const generation = replaceSource({ lifecycle: "starting", ...patch });
    pendingStart = { generation, activate };
    void loadCore();
    return generation;
  };

  const installLocalAuthority = (next: WasmGameLike, patch: Partial<ClientState>) => {
    const generation = replaceSource({
      mode: "local",
      lifecycle: "running",
      ...patch,
    });
    game = next;
    publishFromGame();
    startLoop();
    return generation;
  };

  const newAuthority = (seed: number, scenario?: string) => {
    const next = options.createGame(seed, scenario);
    next.addPlayer(1);
    return next;
  };

  // Whether the command reached the authority (a connecting session drops it).
  const dispatch = (command: Record<string, unknown>, { quiet = false } = {}): boolean => {
    if (disposed() || state.lifecycle === "idle" || state.lifecycle === "failed") return false;
    const playerId = state.playerId;
    try {
      const encoded = encodeCommand(
        aim && AIMED_COMMAND_TYPES.has(command.type as string) ? { ...command, aim } : command,
      );
      if (state.mode === "guest") {
        if (!peer || !playerId || !peer.hostParticipantId) return false;
        peer.sendReliable(peer.hostParticipantId, {
          kind: "command",
          sequence: ++sequence,
          encoded,
        });
      } else if (state.mode === "dedicated") {
        const session = dedicated;
        if (!session || !playerId) return false;
        // Both lanes share one counter; each lane only needs its own sequences to increase.
        const payload = textEncoder.encode(encoded);
        if (HELD_COMMAND_TYPES.has(command.type as string)) {
          void session.sendCommand(++sequence, payload).catch((error) => {
            // A refresh only repeats held state, so its failure is not news to the player.
            if (dedicated === session && !quiet)
              status(t("status.dedicatedCommandFailed", { error: String(error) }));
          });
        } else {
          // Settles on the authority's ack; a rejection or an ended session is reported.
          void session.sendReliableCommand(++sequence, payload).catch((error) => {
            if (dedicated === session)
              status(t("status.dedicatedCommandFailed", { error: String(error) }));
          });
        }
      } else {
        if (!game || !playerId) return false;
        game.applyCommand(playerId, ++sequence, encoded);
      }
      return true;
    } catch (error) {
      status(String(error));
      return false;
    }
  };

  // Re-sends the whole current held set with fresh sequences on every period, rotating the
  // order so each input is regularly the newest datagram, so neither a lost datagram nor
  // reordering against another input (the authority drops datagram sequences below the
  // datagram watermark) can leave the dedicated authority holding a stale value for longer
  // than a few periods. One-shots and bow transitions are on the reliable lane, so a refresh
  // cannot drop them. Every repeated value is idempotent at the authority: a repeated guard,
  // movement or aim only sets the value it already has.
  const refreshHeldInput = () => {
    if (state.mode !== "dedicated" || !dedicated) {
      stopHeldRefresh();
      return;
    }
    if (state.lifecycle !== "running" || !state.playerId) return;
    const send = (command: Record<string, unknown>) => dispatch(command, { quiet: true });
    const refreshers: Record<HeldInput, () => void> = {
      guard: () => {
        const held = guardSources.size > 0;
        if (send({ type: "setGuard", raised: held }) && held) guardRaiseSent = true;
      },
      movement: () => send({ type: "setMovement", x: movement.lastX, z: movement.lastZ }),
      aim: () => {
        if (send({ type: "setAim", direction: aim })) sentAim = aim;
      },
    };
    for (let index = 0; index < HELD_INPUTS.length; index += 1) {
      refreshers[HELD_INPUTS[(heldRefreshRotation + index) % HELD_INPUTS.length]]();
    }
    heldRefreshRotation = (heldRefreshRotation + 1) % HELD_INPUTS.length;
  };

  const startHeldRefresh = () => {
    stopHeldRefresh();
    heldRefreshTimer = schedule(refreshHeldInput, HELD_REFRESH_INTERVAL_MS);
  };

  const sendGuard = (raised: boolean): boolean => {
    const sent = dispatch({ type: "setGuard", raised });
    if (sent && raised) guardRaiseSent = true;
    return sent;
  };

  // Re-asserts the guard this client actually holds once a remote authority (re-)assigns
  // its player: a press made before the assignment never reached an authority, a press or
  // release may have been lost to a link that died unnoticed before a dedicated resume, and
  // a re-joined peer player starts with its guard down. A held guard is always re-asserted
  // (the authority treats a repeated raise as a continued hold); a released one only when
  // the authority may still hold an earlier raise.
  // Returns whether the authority is now in sync (nothing to re-assert, or the send went out).
  const syncGuardOnAssignment = (): boolean => {
    const held = guardSources.size > 0;
    return held || guardRaiseSent ? sendGuard(held) : true;
  };

  const sendAim = (direction: AimDirection): boolean => {
    const sent = dispatch({ type: "setAim", direction });
    if (sent) sentAim = direction;
    return sent;
  };

  // Sends the aim the player is pointing once a remote authority (re-)assigns its player: a
  // pointer move during the connection never reached it, and a re-joined or recovered player
  // may hold an older aim or none. Nothing is sent when no aim is pointed or ever reached it.
  const syncAimOnAssignment = () => {
    if (aim || sentAim) sendAim(aim);
  };

  const syncHeldOnAssignment = () => {
    syncGuardOnAssignment();
    syncAimOnAssignment();
  };

  const flushMovement = () => {
    const keyboardX = Number(movement.right) - Number(movement.left);
    const keyboardZ = Number(movement.backward) - Number(movement.forward);
    const x = Math.max(-1, Math.min(1, keyboardX + movement.touchX + movement.mouseX));
    const z = Math.max(-1, Math.min(1, keyboardZ + movement.touchZ + movement.mouseZ));
    if (x === movement.lastX && z === movement.lastZ) return;
    movement.lastX = x;
    movement.lastZ = z;
    dispatch({ type: "setMovement", x, z });
  };

  return {
    getState: () => state,
    subscribe(listener: () => void) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    hasTickLoop: () => timer !== null,
    hasAuthority: () => game !== null,

    // Loads the gameplay core, or retries after a failure. Resolves once the attempt settled.
    load: loadCore,

    startLocal({
      seed,
      training = false,
      scenario,
    }: { seed?: number; training?: boolean; scenario?: string } = {}) {
      if (disposed()) return;
      const named = scenario ?? DEFAULT_SCENARIO;
      const runSeed = seed ?? options.freshRunSeed();
      const activate = () => {
        const next = newAuthority(runSeed, training ? named : undefined);
        installLocalAuthority(next, {
          playerId: 1,
          training: training ? { paused: false, speed: 1, scenario: named } : null,
        });
      };
      if (state.core === "ready") activate();
      else deferUntilCoreReady({ mode: "local", playerId: 1 }, activate);
    },

    // Restores a validated save. The caller loads the game first, so a rejected save leaves
    // the current source untouched.
    restore({
      game: next,
      controlledPlayerId,
      lastSequence,
      movement: [x, z],
      guardHeld = false,
      drawHeld = false,
      aimHeld = false,
    }: RestoredGame) {
      if (disposed()) {
        next.free?.();
        return;
      }
      installLocalAuthority(next, { playerId: controlledPlayerId });
      sequence = lastSequence;
      movement.lastX = x;
      movement.lastZ = z;
      if (guardHeld) dispatch({ type: "setGuard", raised: false });
      // A restored draw nobody is holding is lowered, never fired.
      if (drawHeld) dispatch({ type: "cancelBow" });
      // A restored aim nobody is pointing returns to committed facing until the next aim.
      if (aimHeld) sendAim(null);
    },

    async hostPeer(apiBase: string) {
      if (disposed()) return;
      if (state.core !== "ready") {
        const waiting = deferUntilCoreReady({ mode: "local", playerId: 1 }, null);
        await loadCore();
        if (!isCurrent(waiting) || state.core !== "ready") return;
      }
      const generation = installLocalAuthority(newAuthority(options.freshRunSeed()), {
        playerId: 1,
        lifecycle: "starting",
      });
      const session = options.createPeerSession(apiBase);
      peer = session;
      detachPeer = options.attachPeerSession({
        session,
        role: "host",
        isCurrent: () => isCurrent(generation) && peer === session,
        getGame: () => game,
        onPlayer: (playerId) => update({ playerId }),
        onSnapshot: (snapshot) => options.snapshots.publish(snapshot),
        onStatus: status,
        localContentRevision: options.localContentRevision,
        // A host never receives authority snapshots.
        onIncompatible: () => {},
      });
      try {
        const lobby = await session.host(4);
        if (!isCurrent(generation) || peer !== session) return;
        update({
          mode: "host",
          lifecycle: "running",
          lobbyCode: lobby.displayCode,
        });
        status(t("status.hosting", { code: lobby.displayCode }));
      } catch (error) {
        if (!isCurrent(generation) || peer !== session) return;
        // Hosting failed; the local authority keeps running solo.
        detachPeer?.();
        detachPeer = null;
        peer = null;
        session.close();
        update({ mode: "local", lifecycle: "running", lobbyCode: "" });
        status(t("status.hostFailed", { error: String(error) }));
      }
    },

    async joinPeer(apiBase: string, code: string) {
      if (disposed()) return;
      const generation = replaceSource({
        mode: "guest",
        lifecycle: "starting",
        playerId: null,
      });
      options.snapshots.publish(null);
      const session = options.createPeerSession(apiBase);
      peer = session;
      detachPeer = options.attachPeerSession({
        session,
        role: "guest",
        isCurrent: () => isCurrent(generation) && peer === session,
        getGame: () => null,
        onPlayer: (playerId) => {
          update({ playerId, lifecycle: "running" });
          syncHeldOnAssignment();
        },
        // The host kept this guest's player across an in-place link recovery, but any held
        // change sent during the outage was refused; re-assert it like after a re-join.
        onLinkRecovered: syncHeldOnAssignment,
        // Held input is released and nothing is sent until the player joins again.
        onHostLinkLost: () => {
          if (!isCurrent(generation) || peer !== session) return;
          teardown();
          update({ lifecycle: "disconnected", playerId: null });
          status(t("status.hostLost"));
        },
        onSnapshot: (snapshot) => options.snapshots.publish(snapshot),
        onStatus: status,
        localContentRevision: options.localContentRevision,
        onIncompatible: (reason) => {
          if (!isCurrent(generation) || peer !== session) return;
          teardown();
          options.snapshots.publish(null);
          update({ lifecycle: "failed", playerId: null });
          status(reason);
        },
      });
      try {
        await session.join(code);
        if (!isCurrent(generation) || peer !== session) return;
        status(t("status.joining", { code }));
      } catch (error) {
        if (!isCurrent(generation) || peer !== session) return;
        teardown();
        update({ lifecycle: "failed" });
        status(t("status.joinFailed", { error: String(error) }));
      }
    },

    async startDedicated(endpoint: string) {
      if (disposed()) return;
      if (!options.supportsWebTransport()) {
        status(t("status.noWebTransport"));
        return;
      }
      const generation = replaceSource({
        mode: "dedicated",
        lifecycle: "starting",
        playerId: null,
      });
      options.snapshots.publish(null);
      const session = options.createDedicatedSession(endpoint);
      dedicated = session;
      const current = () => isCurrent(generation) && dedicated === session;
      try {
        await session.connect({
          onWelcome: (welcome) => {
            if (!current()) return;
            update({ playerId: welcome.playerId, lifecycle: "running" });
            status(
              t("status.dedicatedWelcome", {
                player: String(welcome.playerId),
                hz: welcome.tickHz,
              }),
            );
            syncHeldOnAssignment();
            startHeldRefresh();
          },
          onSnapshot: (frame) => {
            if (!current()) return;
            let snapshot;
            try {
              snapshot = decodeSnapshot(textDecoder.decode(frame.payload));
            } catch (error) {
              dedicated = null;
              stopHeldRefresh();
              session.close();
              update({ lifecycle: "failed", playerId: null });
              status(t("status.dedicatedRejected", { error: String(error) }));
              return;
            }
            const incompatible = contentRevisionMismatch(
              options.localContentRevision(),
              snapshot,
              "dedicated authority",
            );
            if (incompatible) {
              dedicated = null;
              stopHeldRefresh();
              session.close();
              options.snapshots.publish(null);
              update({ lifecycle: "failed", playerId: null });
              status(incompatible);
              return;
            }
            options.snapshots.publish(snapshot);
          },
          onStateChange: (next) => {
            if (!current()) return;
            if (next === "connecting") {
              stopHeldRefresh();
              status(t("status.dedicatedConnecting"));
            }
            if (next === "disconnected") {
              stopHeldRefresh();
              update({ playerId: null, lifecycle: "disconnected" });
              status(t("status.dedicatedDisconnected"));
            }
          },
          onError: (error) => {
            if (current()) status(t("status.dedicatedTransport", { error: String(error) }));
          },
        });
      } catch (error) {
        if (!current()) return;
        dedicated = null;
        update({ lifecycle: "failed", playerId: null });
        status(t("status.dedicatedFailed", { error: String(error) }));
      }
    },

    // Leaves the current game and returns to the idle (character selection) state.
    stop() {
      if (disposed()) return;
      replaceSource({ mode: "local", lifecycle: "idle", playerId: 1 });
      options.snapshots.publish(null);
    },

    dispose() {
      if (disposed()) return;
      teardown();
      update({
        lifecycle: "disposed",
        playerId: null,
        lobbyCode: "",
        training: null,
        sourceGeneration: state.sourceGeneration + 1,
      });
      listeners.clear();
    },

    dispatch,

    setHeldMovement(key: MovementKey, held: boolean) {
      if (movement[key] === held) return;
      movement[key] = held;
      flushMovement();
    },

    setTouchMovement(x: number, z: number) {
      movement.touchX = x;
      movement.touchZ = z;
      flushMovement();
    },

    // Mouse and touch have separate held channels so releasing either leaves the other intact.
    setMouseMovement(x: number, z: number) {
      if (movement.mouseX === x && movement.mouseZ === z) return;
      movement.mouseX = x;
      movement.mouseZ = z;
      flushMovement();
    },

    // Clears every held input (focus loss, settings, context change) and stops movement.
    // Held shield input from one device; guard stays raised while any device holds it.
    setGuard(source: GuardSource, held: boolean) {
      const wasHeld = guardSources.size > 0;
      if (held) guardSources.add(source);
      else guardSources.delete(source);
      const isHeld = guardSources.size > 0;
      if (isHeld !== wasHeld) sendGuard(isHeld);
    },

    // Held bow draw per device: the first hold draws; releasing the last hold shoots
    // (the authority ignores short draws) or, when `interrupted`, cancels without shooting.
    setBowDraw(source: GuardSource, held: boolean, { interrupted = false } = {}) {
      if (held) {
        if (drawSources.size === 0) dispatch({ type: "drawBow" });
        drawSources.add(source);
        return;
      }
      if (!drawSources.delete(source) || drawSources.size > 0) return;
      dispatch({ type: interrupted ? "cancelBow" : "releaseBow" });
    },

    // Records aim intent (mouse, stick or touch) and sends it when it differs from the aim
    // the authority last received; null clears it. A session that is still connecting
    // receives it on player assignment.
    setAim(direction: AimDirection) {
      aim = direction ? [direction[0], direction[1]] : null;
      if (!sameAim(aim, sentAim)) sendAim(aim);
    },

    // Locks the nearest target or cycles the lock; the authority owns eligibility and order.
    cycleTarget() {
      dispatch({ type: "cycleTarget" });
    },

    clearTarget() {
      dispatch({ type: "clearTarget" });
    },

    // Drops every held bow draw without shooting (weapon switches, menus).
    cancelBowDraw() {
      if (drawSources.size === 0) return;
      drawSources.clear();
      dispatch({ type: "cancelBow" });
    },

    releaseInput() {
      movement.forward = false;
      movement.backward = false;
      movement.left = false;
      movement.right = false;
      movement.touchX = 0;
      movement.touchZ = 0;
      movement.mouseX = 0;
      movement.mouseZ = 0;
      flushMovement();
      if (guardSources.size > 0) {
        guardSources.clear();
        sendGuard(false);
      }
      // Losing focus or opening a menu must never fire a drawn bow.
      if (drawSources.size > 0) {
        drawSources.clear();
        dispatch({ type: "cancelBow" });
      }
      // The pointer position is unknown after focus loss: fall back to committed facing.
      aim = null;
      if (sentAim) sendAim(null);
    },

    // Opens or closes the game menu's pause. Whether the world actually stops is the
    // runtime's policy: only local play suspends its authority.
    setMenuPaused(paused: boolean) {
      menuPaused = paused;
    },

    // Whether the open game menu currently suspends the world.
    worldPausedByMenu: () => menuPaused && menuPausesWorld(state.mode),

    setTrainingPaused(paused: boolean) {
      if (!state.training) return;
      update({ training: { ...state.training, paused } });
    },

    setTrainingSpeed(speed: number) {
      if (!state.training) return;
      update({ training: { ...state.training, speed } });
    },

    stepTraining() {
      if (!state.training?.paused || !game) return;
      advance(1);
    },

    // The local training session's accepted commands and ticks as `Reproduction` JSON for
    // the native `replay_reproduction` runner. Workbench-only: null in hosted, guest,
    // dedicated and ordinary play, which never record.
    exportReproduction(): string | null {
      if (state.mode !== "local" || !state.training || !game?.reproductionJson) return null;
      return game.reproductionJson();
    },

    // Applies a workbench operation to the local training authority, which validates and
    // records it for the reproduction. Throws the authority's validation message; never
    // reaches hosted, guest, dedicated or ordinary play.
    applyWorkbench(operation: WorkbenchOperation) {
      if (state.mode !== "local" || !state.training || !game?.applyWorkbench) {
        throw new Error("workbench operations are available in local training only");
      }
      game.applyWorkbench(JSON.stringify(operation));
      // Paused sessions show the new arrangement without waiting for a tick.
      publishFromGame();
    },

    // The tuning values the local training authority runs; null elsewhere.
    trainingTuning(): TuningValue[] | null {
      if (state.mode !== "local" || !state.training || !game?.tuningJson) return null;
      return JSON.parse(game.tuningJson()) as TuningValue[];
    },

    // The room the local training authority's workbench arranges; null elsewhere.
    trainingWorkbenchRoom(): number | null {
      if (state.mode !== "local" || !state.training || !game?.workbenchRoomId) return null;
      return game.workbenchRoomId() ?? null;
    },

    // Captures the current local authority for saving; null outside local play.
    captureSaveState() {
      if (state.mode !== "local" || !game || !state.playerId) return null;
      return { saveStateJson: game.saveStateJson(), playerId: state.playerId };
    },
  };
}

export type GameClientRuntime = ReturnType<typeof createGameClientRuntime>;
