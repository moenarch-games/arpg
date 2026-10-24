// Presentation cues (#67, 67a): a bounded, typed projection of authoritative snapshots into
// short-lived feedback events (hits, blocks, whiffs, defeats, pickups, arrows, counters,
// encounter changes) for the HUD and, later, effects and audio.
//
// Cues are cosmetic. Persistent truth (health, guard, the counter window, statuses) stays in
// the snapshot and is always read from it; a dropped or deduplicated cue never hides state.
//
// - Every cue has a stable key derived from authoritative identity (strike source + strike
//   tick + target, event tick + order, entity id + transition tick), so a snapshot replayed
//   after a reconnect, or a late duplicate, never plays twice. Keys live in a bounded set.
// - Within one source, a snapshot whose tick is not newer than the last one is stale and
//   produces nothing. A new source (session replacement, restart, restore/seek), a cleared
//   store, or another run seed, scenario or content revision resets everything; the first
//   snapshot after a reset is only a baseline, so a resumed stream never replays the
//   transitions that led to it.
// - Each authoritative snapshot emits at most `frameLimit` cues (the rest are dropped and
//   counted), and at most `capacity` recent cues are kept for `lifetimeTicks` ticks.
// - Cues about a player or monster that left the snapshot are dropped with it.

const MELEE_KINDS = new Set([
  "primaryAttack",
  "secondaryAttack",
  "counter",
  "lightFollowUp",
  "lightFinisher",
  "heavyFinisher",
]);

const BOW_ARROW = "bow.arrow";

export const CUE_DEFAULTS = Object.freeze({
  frameLimit: 12,
  capacity: 32,
  keyCapacity: 512,
  lifetimeTicks: 90,
});

const partyKey = (party) => `${party.kind}:${party.id}`;
const sameParty = (left, right) => left.kind === right.kind && left.id === right.id;

function entityPosition(snapshot, party) {
  const list =
    party.kind === "player"
      ? snapshot.players
      : party.kind === "monster"
        ? snapshot.monsters
        : party.kind === "crate"
          ? snapshot.crates
          : null;
  return list?.find((entity) => entity.id === party.id)?.position ?? null;
}

function sourceIdentity(snapshot) {
  return `${snapshot.runSeed}|${snapshot.scenario}|${snapshot.contentRevision}`;
}

// The tick on which the phase an action is in now began; identical in every snapshot of
// that phase, so it identifies the phase however many of its snapshots arrive.
const phaseStartTick = (tick, action) => tick - (action.phaseTicks - action.ticksRemaining);

export function createPresentationCues(options = {}) {
  const { frameLimit, capacity, keyCapacity, lifetimeTicks } = { ...CUE_DEFAULTS, ...options };
  const listeners = new Set();
  let cues = [];
  let keys = new Set();
  let previous = null;
  let source;
  let dropped = 0;
  // Per player: whether the current melee action's strike resolved against anything.
  let struck = new Map();

  const notify = () => {
    for (const listener of listeners) listener();
  };

  const remember = (key) => {
    if (keys.has(key)) return false;
    keys.add(key);
    if (keys.size > keyCapacity) keys.delete(keys.values().next().value);
    return true;
  };

  const clear = () => {
    const had = cues.length > 0;
    cues = [];
    keys = new Set();
    previous = null;
    struck = new Map();
    return had;
  };

  // The cues one snapshot adds after `before`, in authoritative order.
  const derive = (before, snapshot) => {
    const { tick } = snapshot;
    const found = [];
    const add = (cue) => found.push({ tick, order: 1_000_000 + found.length, ...cue });

    const strikes = snapshot.strikeEvents ?? [];
    for (const event of strikes) {
      if (event.source.kind === "player") struck.set(event.source.id, true);
      found.push({
        key: `strike:${partyKey(event.source)}:${event.strikeTick}:${partyKey(event.target)}`,
        tick,
        order: event.order,
        kind: event.result.kind,
        definition: event.definition,
        source: event.source,
        target: event.target,
        damage: event.result.kind === "hit" ? event.result.damage : 0,
        defeated: event.result.kind === "hit" && event.result.defeated,
        position: entityPosition(snapshot, event.target) ?? entityPosition(before, event.target),
      });
    }
    for (const event of snapshot.interactionEvents ?? []) {
      const { result } = event;
      found.push({
        key: `interaction:${event.playerId}:${tick}:${event.order}`,
        tick,
        order: event.order,
        kind: result.kind === "refused" ? "refused" : result.kind,
        playerId: event.playerId,
        gold: result.gold ?? 0,
        reason: result.reason ?? null,
      });
    }

    const beforePlayers = new Map(before.players.map((player) => [player.id, player]));
    for (const player of snapshot.players) {
      const was = beforePlayers.get(player.id);
      const action = player.action;
      if (!action || action.phase === "windup") struck.set(player.id, false);
      // A melee swing that reached recovery without resolving against anything: a whiff,
      // distinct from a block or an obstruction, and never given a damage number.
      if (
        action &&
        MELEE_KINDS.has(action.kind) &&
        action.phase === "recovery" &&
        !action.connected &&
        !struck.get(player.id)
      )
        add({
          key: `whiff:${player.id}:${action.kind}:${phaseStartTick(tick, action)}`,
          kind: "whiff",
          playerId: player.id,
          action: action.kind,
        });
      if (was?.alive && !player.alive)
        add({ key: `defeat:player:${player.id}:${tick}`, kind: "defeat", target: party(player) });

      const counter = player.counter;
      const earlier = was?.counter;
      if (counter && counter.blockedAtTick !== earlier?.blockedAtTick)
        add({
          key: `counter:${player.id}:${counter.blockedAtTick}:open`,
          kind: "counterOpen",
          playerId: player.id,
          expiresAtTick: counter.expiresAtTick,
        });
      if (earlier && counter?.blockedAtTick !== earlier.blockedAtTick && !counter)
        add({
          key: `counter:${player.id}:${earlier.blockedAtTick}:end`,
          kind:
            action?.kind === "counter"
              ? "counterUsed"
              : tick >= earlier.expiresAtTick
                ? "counterExpired"
                : "counterLost",
          playerId: player.id,
        });
    }

    const beforeMonsters = new Map(before.monsters.map((monster) => [monster.id, monster]));
    for (const monster of snapshot.monsters) {
      if (beforeMonsters.get(monster.id)?.alive && !monster.alive)
        add({
          key: `defeat:monster:${monster.id}:${tick}`,
          kind: "defeat",
          target: { kind: "monster", id: monster.id },
          definition: monster.definition,
        });
    }

    const beforeArrows = new Map((before.arrows ?? []).map((arrow) => [arrow.id, arrow]));
    const arrows = new Set();
    for (const arrow of snapshot.arrows ?? []) {
      arrows.add(arrow.id);
      if (!beforeArrows.has(arrow.id) && arrow.source.kind === "player")
        add({
          key: `arrow:${arrow.id}:release`,
          kind: "arrowRelease",
          playerId: arrow.source.id,
          arrowId: arrow.id,
        });
    }
    for (const [id, arrow] of beforeArrows) {
      if (arrows.has(id) || arrow.source.kind !== "player") continue;
      // An arrow that left without striking anything this tick hit the world.
      const struckSomething = strikes.some(
        (event) => event.definition === BOW_ARROW && sameParty(event.source, arrow.source),
      );
      if (!struckSomething)
        add({
          key: `arrow:${id}:landed`,
          kind: "arrowLanded",
          playerId: arrow.source.id,
          arrowId: id,
          position: arrow.position,
        });
    }

    const beforeRooms = new Map(before.rooms.map((room) => [room.id, room]));
    for (const room of snapshot.rooms) {
      const was = beforeRooms.get(room.id);
      if (was && was.encounterState !== room.encounterState)
        add({
          key: `encounter:${room.id}:${room.encounterState}:${tick}`,
          kind: "encounter",
          roomId: room.id,
          state: room.encounterState,
        });
      // A phased encounter moving on (#66); a wipe's return to the first phase is the reset.
      if (was && (room.phase ?? 0) > (was.phase ?? 0))
        add({
          key: `phase:${room.id}:${room.phase}:${tick}`,
          kind: "phase",
          roomId: room.id,
          phase: room.phase,
          phaseCount: room.phaseCount ?? 0,
        });
    }

    return found.sort((left, right) => left.order - right.order);
  };

  return {
    // Feeds one published snapshot (or null for a cleared store). `sourceId` identifies the
    // authority source that produced it; a different one resets the projection.
    observe(snapshot, sourceId) {
      if (!snapshot) {
        source = undefined;
        if (clear()) notify();
        return [];
      }
      const identity = `${sourceId ?? ""}|${sourceIdentity(snapshot)}`;
      if (identity !== source) {
        source = identity;
        const had = clear();
        previous = snapshot;
        if (had) notify();
        return [];
      }
      if (snapshot.tick <= previous.tick) return [];

      const derived = derive(previous, snapshot);
      previous = snapshot;
      const fresh = derived.filter((cue) => remember(cue.key));
      const emitted = fresh.slice(0, frameLimit);
      dropped += fresh.length - emitted.length;

      const players = new Set(snapshot.players.map((player) => player.id));
      const monsters = new Set(snapshot.monsters.map((monster) => monster.id));
      const present = (party) =>
        !party ||
        ((party.kind === "player" ? players.has(party.id) : true) &&
          (party.kind === "monster" ? monsters.has(party.id) : true));
      const kept = [...cues, ...emitted].filter(
        (cue) =>
          cue.tick > snapshot.tick - lifetimeTicks &&
          present(cue.source) &&
          present(cue.target) &&
          (cue.playerId == null || players.has(cue.playerId)),
      );
      const next = kept.slice(Math.max(0, kept.length - capacity));
      if (next.length !== cues.length || next.some((cue, index) => cue !== cues[index])) {
        cues = next;
        notify();
      }
      return emitted;
    },

    reset() {
      source = undefined;
      if (clear()) notify();
    },

    // The recent cues, oldest first. A new array whenever they change.
    getCues: () => cues,
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
    stats: () => ({ dropped, keys: keys.size, cues: cues.length }),
  };
}

function party(player) {
  return { kind: "player", id: player.id };
}

// Feeds every snapshot the store publishes, with the store's source generation, into `cues`.
export function attachPresentationCues(store, cues) {
  const feed = () => cues.observe(store.getSnapshot(), store.getSource?.());
  feed();
  return store.subscribe(feed);
}
