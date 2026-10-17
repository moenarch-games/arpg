// In-world humanoid characters (#55, #188): one instance of the shared character asset per
// player in the snapshot, placed at the player's authoritative position, turned to the
// direction the frame shows and animated from authoritative motion. Presentation only: it
// reads snapshots and never feeds anything back into gameplay.
//
// Animation time follows the simulation timeline (snapshot ticks), so pausing, stepping and
// the render frame rate behave like the game. Standing plays the idle clip (the guard clip
// while guarding, the sword-drawn idle while a monster is engaged with the character), moving
// picks a gait by speed with playback scaled to the stride, death plays once and holds, and
// every change crossfades. Attack, hurt and bow clips belong to #56. Each character carries
// the equipment of its authoritative weapon (#183).

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
  action?: { facing: number[]; aim?: number[] | null } | null;
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
  tick: number;
  position: number[];
  speed: number;
  // Seconds of the death clip already played, while dead.
  dying: number | null;
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

export function createCharacterLayer(options: {
  getAsset: () => CharacterAssetLike | null;
  animation: CharacterAnimation;
  tickHz: number;
}) {
  const { animation, tickHz } = options;
  const animated = new Map<number, Animated>();

  const advance = (state: Animated, seconds: number, asset: CharacterAssetLike) => {
    if (seconds <= 0) return;
    if (state.dying !== null) {
      // Play the death clip once and hold its last pose.
      const duration = asset.clips.find((clip) => clip.name === animation.death)?.duration ?? 0;
      const remaining = Math.max(0, duration - 1 / tickHz - state.dying);
      const step = Math.min(seconds, remaining);
      if (step > 0) state.instance.advance(step);
      state.dying += step;
      return;
    }
    const gait = animation.gaits.find((candidate) => candidate.clip === state.clip);
    const rate = gait
      ? Math.min(animation.rate.max, Math.max(animation.rate.min, state.speed / gait.speed))
      : 1;
    state.instance.advance(seconds * rate);
  };

  const sync = (snapshot: SnapshotLike) => {
    const asset = options.getAsset();
    const scale = snapshot.worldUnitsPerMeter;
    let changed = false;
    const live = new Set<number>();
    const engaged = new Set(
      (snapshot.monsters ?? [])
        .filter((monster) => monster.alive !== false && monster.targetPlayerId != null)
        .map((monster) => monster.targetPlayerId),
    );
    if (asset) {
      for (const player of snapshot.players) {
        live.add(player.id);
        const alive = player.alive !== false;
        const clipState = (speed: number): ClipState => ({
          alive,
          speed,
          guarding: player.guard != null,
          engaged: engaged.has(player.id),
        });
        let state = animated.get(player.id);
        if (!state) {
          const clip = chooseClip(animation, clipState(0));
          const instance = asset.createInstance();
          instance.seek(clip, 0);
          state = {
            instance,
            clip,
            tick: snapshot.tick,
            position: player.position,
            speed: 0,
            dying: alive ? null : 0,
            weapon: undefined,
            unequip: () => {},
          };
          animated.set(player.id, state);
          changed = true;
        } else {
          const ticks = snapshot.tick - state.tick;
          // A tick going backwards is a new session or a reset: start over from rest.
          const seconds = ticks > 0 ? ticks / tickHz : 0;
          if (ticks < 0) state.speed = 0;
          if (seconds > 0) {
            const [x0, , z0] = state.position;
            const [x1, , z1] = player.position;
            const measured = Math.hypot(x1 - x0, z1 - z0) / scale / seconds;
            const blend = Math.min(1, seconds / SPEED_SMOOTHING_SECONDS);
            state.speed += (measured - state.speed) * blend;
          }
          state.tick = snapshot.tick;
          state.position = player.position;
          const clip = chooseClip(animation, clipState(state.speed));
          if (clip !== state.clip) {
            state.instance.transition(clip, animation.crossfadeSeconds);
            state.clip = clip;
            state.dying = alive ? null : 0;
          }
          advance(state, seconds, asset);
        }
        if (player.weapon !== state.weapon) {
          state.unequip();
          state.weapon = player.weapon;
          state.unequip =
            player.weapon && asset.equip ? asset.equip(state.instance, player.weapon) : () => {};
        }
        const [x, y, z] = player.position;
        state.instance.root.position.set(x / scale, y / scale, z / scale);
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
    return { roots: [...animated.values()].map((state) => state.instance.root), changed };
  };

  return {
    sync,
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
