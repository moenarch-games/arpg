// In-world humanoid characters (#55): one instance of the shared character asset per
// player in the snapshot, placed at the player's authoritative position and turned to the
// direction the frame shows. Presentation only: it reads snapshots and never feeds
// anything back into gameplay. Locomotion and combat clips belong to #56; until then every
// character plays its idle loop.

export interface CharacterInstanceLike {
  root: {
    position: { set(x: number, y: number, z: number): void };
    rotation: { set(x: number, y: number, z: number): void };
  };
  seek(clip: string, seconds: number): void;
  advance(seconds: number): void;
  dispose(): void;
}

export interface CharacterAssetLike {
  createInstance(): CharacterInstanceLike;
}

interface PlayerLike {
  id: number;
  position: number[];
  facing?: number[] | null;
  aim?: number[] | null;
  lockedMonsterId?: number | null;
  action?: { facing: number[]; aim?: number[] | null } | null;
}

interface SnapshotLike {
  worldUnitsPerMeter: number;
  players: PlayerLike[];
}

// The exact committed or intended direction, as the frame shows it: an action's aim or
// facing, the authoritative facing under a target lock, otherwise aim or facing.
export function presentedFacing(player: PlayerLike): number[] {
  if (player.action) return player.action.aim ?? player.action.facing;
  if (player.lockedMonsterId != null) return player.facing ?? [1, 0];
  return player.aim ?? player.facing ?? [1, 0];
}

export function createCharacterLayer(options: {
  getAsset: () => CharacterAssetLike | null;
  idleClip: string;
  now: () => number;
}) {
  const instances = new Map<number, CharacterInstanceLike>();
  let lastTime: number | null = null;

  const sync = (snapshot: SnapshotLike) => {
    const asset = options.getAsset();
    const time = options.now();
    const delta = lastTime === null ? 0 : Math.max(0, (time - lastTime) / 1000);
    lastTime = time;
    let changed = false;
    const live = new Set<number>();
    if (asset) {
      for (const player of snapshot.players) {
        live.add(player.id);
        let instance = instances.get(player.id);
        if (!instance) {
          instance = asset.createInstance();
          instance.seek(options.idleClip, 0);
          instances.set(player.id, instance);
          changed = true;
        } else if (delta > 0) {
          instance.advance(delta);
        }
        const scale = snapshot.worldUnitsPerMeter;
        const [x, y, z] = player.position;
        instance.root.position.set(x / scale, y / scale, z / scale);
        const [facingX, facingZ] = presentedFacing(player);
        // The character faces +Z at rest.
        instance.root.rotation.set(0, Math.atan2(facingX, facingZ), 0);
      }
    }
    for (const [id, instance] of instances) {
      if (live.has(id)) continue;
      instance.dispose();
      instances.delete(id);
      changed = true;
    }
    return { roots: [...instances.values()].map((instance) => instance.root), changed };
  };

  return {
    sync,
    dispose() {
      for (const instance of instances.values()) instance.dispose();
      instances.clear();
      lastTime = null;
    },
  };
}

export type CharacterLayer = ReturnType<typeof createCharacterLayer>;
