// In-world humanoid characters (#55, #188, #56): one instance of the shared character asset
// per player in the snapshot, standing at the player's authoritative position (its origin is
// at its feet, so it stands at the body's published feet height and rises and falls with a
// jump, #59), turned to the direction the frame shows and animated from authoritative
// state. Presentation only: it reads snapshots and never feeds anything back into gameplay;
// no clip event triggers damage, attacks or rewards, and root motion never moves the body.
//
// Animation time follows the simulation timeline (snapshot ticks), so pausing, stepping and
// the render frame rate behave like the game. Each snapshot picks one pose, highest first:
// death (plays once and holds), the current action (each phase plays its authored stretch
// of a clip, placed by the phase's remaining ticks, so the swing's contact lands as the
// active phase opens), a reaction (hurt, blocked, guard broken: plays once from its start),
// the bow's draw, airborne takeoff/fall, landing, then standing (the guard clip while
// guarding, the sword-drawn idle while a monster is engaged with the character) or a gait
// picked by speed with playback scaled to the stride. Every change crossfades; a repeated
// or late snapshot never restarts a clip, it only moves it to where the timeline says.
// A mapped clip the asset lacks falls back to the next pose and is reported by
// `missingClips()`. Each character carries the equipment of its authoritative weapon (#183).

export interface CharacterInstanceLike {
  root: {
    position: { set(x: number, y: number, z: number): void };
    rotation: { set(x: number, y: number, z: number): void };
  };
  seek(clip: string, seconds: number): void;
  transition(clip: string, seconds: number): void;
  advance(seconds: number): void;
  dispose(): void;
}

export interface CharacterAssetLike {
  clips: readonly { name: string; duration: number }[];
  createInstance(): CharacterInstanceLike;
  // Attaches a weapon's equipment to an instance and returns a function that removes it.
  equip?(instance: CharacterInstanceLike, weapon: string): () => void;
}

export interface ClipSegment {
  clip: string;
  from: number;
  to: number;
}

export type ActionPhase = "windup" | "active" | "recovery";

export interface CharacterAnimation {
  idle: string;
  engagedIdle?: string;
  guard?: string;
  death: string;
  // Speeds (m/s) below this count as standing.
  standingBelow: number;
  // Ordered by `upTo`; `speed` is the ground speed the clip's stride matches.
  gaits: readonly { clip: string; speed: number; upTo: number }[];
  crossfadeSeconds: number;
  rate: { min: number; max: number };
  actions?: Readonly<Record<string, Readonly<Record<ActionPhase, ClipSegment>>>>;
  actionCrossfadeSeconds?: number;
  reactions?: Readonly<Record<string, string>>;
  bow?: { draw: string; hold: string };
  air?: {
    takeoff: string;
    fall: string;
    land: string;
    fallAfterSeconds: number;
    landSeconds: number;
  };
}

interface ActionLike {
  kind?: string;
  phase?: string;
  ticksRemaining?: number;
  phaseTicks?: number;
  facing: number[];
  aim?: number[] | null;
}

interface PlayerLike {
  id: number;
  alive?: boolean;
  weapon?: string;
  guard?: unknown;
  position: number[];
  facing?: number[] | null;
  aim?: number[] | null;
  lockedMonsterId?: number | null;
  action?: ActionLike | null;
  reaction?: { kind: string; ticksRemaining: number } | null;
  drawTicks?: number | null;
  // Authoritative height and support (#59).
  motion?: {
    grounded: boolean;
    verticalVelocity?: number;
    feetY: number;
    groundY?: number | null;
  } | null;
}

interface SnapshotLike {
  tick: number;
  worldUnitsPerMeter: number;
  players: PlayerLike[];
  monsters?: { alive?: boolean; targetPlayerId?: number | null }[];
}

// The exact committed or intended direction, as the frame shows it: an action's aim or
// facing, the authoritative facing under a target lock, otherwise aim or facing.
export function presentedFacing(player: PlayerLike): number[] {
  if (player.action) return player.action.aim ?? player.action.facing;
  if (player.lockedMonsterId != null) return player.facing ?? [1, 0];
  return player.aim ?? player.facing ?? [1, 0];
}

// Speed reacts within about this many seconds, so a single uneven tick does not flicker gaits.
const SPEED_SMOOTHING_SECONDS = 0.1;

interface Animated {
  instance: CharacterInstanceLike;
  clip: string;
  // Clip seconds the instance has played of `clip` since it started it.
  time: number;
  tick: number;
  position: number[];
  speed: number;
  // Tick a reaction started (restarted when a new one begins), the tick the body left its
  // support and the tick it touched down, or null.
  reaction: { kind: string; ticksRemaining: number; since: number } | null;
  airborneSince: number | null;
  landedAt: number | null;
  weapon: string | undefined;
  unequip: () => void;
}

export interface ClipState {
  alive: boolean;
  speed: number;
  guarding?: boolean;
  engaged?: boolean;
}

export function chooseClip(animation: CharacterAnimation, state: ClipState) {
  if (!state.alive) return animation.death;
  if (state.speed < animation.standingBelow) {
    if (state.guarding && animation.guard) return animation.guard;
    if (state.engaged && animation.engagedIdle) return animation.engagedIdle;
    return animation.idle;
  }
  return (animation.gaits.find((gait) => state.speed <= gait.upTo) ?? animation.gaits.at(-1)!).clip;
}

// Where an authoritative action phase is in its clip: `from`..`to` by the phase's elapsed
// ticks. The first snapshot of a phase is its start; the last is one tick short of its end.
export function actionPose(
  animation: CharacterAnimation,
  action: ActionLike | null | undefined,
): { clip: string; time: number } | null {
  if (!action?.kind || !action.phase) return null;
  const segment = animation.actions?.[action.kind]?.[action.phase as ActionPhase];
  if (!segment) return null;
  const total = action.phaseTicks ?? 0;
  const progress =
    total > 0 ? Math.min(1, Math.max(0, (total - (action.ticksRemaining ?? total)) / total)) : 0;
  return {
    clip: segment.clip,
    time: segment.from + (segment.to - segment.from) * progress,
  };
}

interface Pose {
  clip: string;
  // Clip seconds the pose shows, or `advance` seconds to play on from the current time.
  time?: number;
  advance?: number;
  crossfade?: number;
}

export function createCharacterLayer(options: {
  getAsset: () => CharacterAssetLike | null;
  animation: CharacterAnimation;
  tickHz: number;
}) {
  const { animation, tickHz } = options;
  const animated = new Map<number, Animated>();
  const missing = new Set<string>();

  const pose = (
    state: Animated,
    player: PlayerLike,
    seconds: number,
    asset: CharacterAssetLike,
    engaged: boolean,
  ): Pose => {
    const duration = (clip: string) => {
      const found = asset.clips.find((candidate) => candidate.name === clip);
      if (!found) missing.add(clip);
      return found?.duration;
    };
    // Plays a one-shot clip on from where it is, holding just short of its last frame.
    const once = (clip: string, elapsed: number): Pose | null => {
      const length = duration(clip);
      if (length === undefined) return null;
      return {
        clip,
        time: Math.min(elapsed, Math.max(0, length - 1 / tickHz)),
      };
    };
    const at = (ticks: number) => (state.tick - ticks) / tickHz;
    const alive = player.alive !== false;
    if (!alive) {
      const elapsed = (state.clip === animation.death ? state.time : 0) + seconds;
      return once(animation.death, elapsed) ?? { clip: animation.death, time: 0 };
    }
    const action = actionPose(animation, player.action);
    if (action && duration(action.clip) !== undefined) {
      return { ...action, crossfade: animation.actionCrossfadeSeconds };
    }
    const reactionClip = state.reaction && animation.reactions?.[state.reaction.kind];
    if (state.reaction && reactionClip) {
      const reaction = once(reactionClip, at(state.reaction.since));
      if (reaction) return reaction;
    }
    if (player.drawTicks != null && animation.bow) {
      const drawn = player.drawTicks / tickHz;
      const draw = duration(animation.bow.draw);
      if (draw !== undefined && drawn < draw) return { clip: animation.bow.draw, time: drawn };
      if (draw !== undefined && duration(animation.bow.hold) !== undefined) {
        return { clip: animation.bow.hold, time: drawn - draw };
      }
    }
    const { air } = animation;
    if (air && state.airborneSince !== null) {
      const flying = at(state.airborneSince);
      const rising = (player.motion?.verticalVelocity ?? 0) > 0;
      if (rising || flying >= air.fallAfterSeconds || state.clip === air.takeoff) {
        const takeoff = duration(air.takeoff);
        if (takeoff !== undefined && flying < takeoff) return { clip: air.takeoff, time: flying };
        if (takeoff !== undefined && duration(air.fall) !== undefined) {
          return { clip: air.fall, time: flying - takeoff };
        }
      }
    }
    const clip = chooseClip(animation, {
      alive,
      speed: state.speed,
      guarding: player.guard != null,
      engaged,
    });
    if (air && state.landedAt !== null && state.speed < animation.standingBelow) {
      const landed = at(state.landedAt);
      if (landed < air.landSeconds) {
        const landing = once(air.land, landed);
        if (landing) return landing;
      }
    }
    const gait = animation.gaits.find((candidate) => candidate.clip === clip);
    const rate = gait
      ? Math.min(animation.rate.max, Math.max(animation.rate.min, state.speed / gait.speed))
      : 1;
    return { clip, advance: seconds * rate };
  };

  // Moves the instance to the pose: a new clip (or an earlier time in the same clip, a new
  // action or reaction) crossfades from the start, then plays on to the pose's time.
  const apply = (state: Animated, next: Pose) => {
    const continuing = next.clip === state.clip;
    const base = continuing ? state.time : 0;
    const time = next.time ?? base + (next.advance ?? 0);
    if (!continuing || time < state.time - 1e-9) {
      state.instance.transition(next.clip, next.crossfade ?? animation.crossfadeSeconds);
      state.clip = next.clip;
      state.time = 0;
    }
    const step = time - state.time;
    if (step > 0) state.instance.advance(step);
    state.time = Math.max(state.time, time);
  };

  const sync = (snapshot: SnapshotLike) => {
    const asset = options.getAsset();
    const scale = snapshot.worldUnitsPerMeter;
    let changed = false;
    const live = new Set<number>();
    const engagedPlayers = new Set(
      (snapshot.monsters ?? [])
        .filter((monster) => monster.alive !== false && monster.targetPlayerId != null)
        .map((monster) => monster.targetPlayerId),
    );
    if (asset) {
      for (const player of snapshot.players) {
        live.add(player.id);
        const engaged = engagedPlayers.has(player.id);
        let state = animated.get(player.id);
        let seconds = 0;
        if (!state) {
          const clip = chooseClip(animation, {
            alive: player.alive !== false,
            speed: 0,
            guarding: player.guard != null,
            engaged,
          });
          const instance = asset.createInstance();
          instance.seek(clip, 0);
          state = {
            instance,
            clip,
            time: 0,
            tick: snapshot.tick,
            position: player.position,
            speed: 0,
            reaction: null,
            airborneSince: null,
            landedAt: null,
            weapon: undefined,
            unequip: () => {},
          };
          animated.set(player.id, state);
          changed = true;
        } else {
          const ticks = snapshot.tick - state.tick;
          // A tick going backwards is a new session or a reset: start over from rest.
          seconds = ticks > 0 ? ticks / tickHz : 0;
          if (ticks < 0) {
            state.speed = 0;
            state.reaction = null;
            state.airborneSince = null;
            state.landedAt = null;
          }
          if (seconds > 0) {
            const [x0, , z0] = state.position;
            const [x1, , z1] = player.position;
            const measured = Math.hypot(x1 - x0, z1 - z0) / scale / seconds;
            const blend = Math.min(1, seconds / SPEED_SMOOTHING_SECONDS);
            state.speed += (measured - state.speed) * blend;
          }
          state.tick = snapshot.tick;
          state.position = player.position;
        }
        // A reaction starts when one appears, changes kind or is renewed by a new hit.
        const reaction = player.reaction;
        if (!reaction) state.reaction = null;
        else if (
          !state.reaction ||
          state.reaction.kind !== reaction.kind ||
          reaction.ticksRemaining > state.reaction.ticksRemaining
        ) {
          state.reaction = { ...reaction, since: snapshot.tick };
        } else state.reaction.ticksRemaining = reaction.ticksRemaining;
        // Support comes from the body's authoritative motion; no motion means supported.
        const grounded = player.motion?.grounded ?? true;
        if (!grounded && state.airborneSince === null) {
          state.airborneSince = snapshot.tick;
          state.landedAt = null;
        } else if (grounded && state.airborneSince !== null) {
          // Only a shown jump or fall lands; a brief step off a ledge does not.
          const flew = state.clip === animation.air?.takeoff || state.clip === animation.air?.fall;
          state.landedAt = flew ? snapshot.tick : null;
          state.airborneSince = null;
        }
        apply(state, pose(state, player, seconds, asset, engaged));
        if (player.weapon !== state.weapon) {
          state.unequip();
          state.weapon = player.weapon;
          state.unequip =
            player.weapon && asset.equip ? asset.equip(state.instance, player.weapon) : () => {};
        }
        const [x, y, z] = player.position;
        // The character's origin is its feet. Gait speed above stays horizontal: a jump's
        // vertical motion never reads as walking.
        state.instance.root.position.set(x / scale, (player.motion?.feetY ?? y) / scale, z / scale);
        const [facingX, facingZ] = presentedFacing(player);
        // The character faces +Z at rest.
        state.instance.root.rotation.set(0, Math.atan2(facingX, facingZ), 0);
      }
    }
    for (const [id, state] of animated) {
      if (live.has(id)) continue;
      state.unequip();
      state.instance.dispose();
      animated.delete(id);
      changed = true;
    }
    return {
      roots: [...animated.values()].map((state) => state.instance.root),
      changed,
    };
  };

  return {
    sync,
    // Mapped clips the loaded asset does not have; those poses fall back (#56).
    missingClips: () => [...missing].sort(),
    dispose() {
      for (const state of animated.values()) {
        state.unequip();
        state.instance.dispose();
      }
      animated.clear();
    },
  };
}

export type CharacterLayer = ReturnType<typeof createCharacterLayer>;
