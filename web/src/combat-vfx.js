// Combat VFX (#67, 67b): impact sparks, block sparks, arrow and obstruction dust, weapon
// trails and a restrained camera impulse, all driven by presentation cues
// (presentation-cues.js) and drawn by 3d-lab's cosmetic flipbook effects
// (`frame.effects`, docs/contracts/browser-renderer.md in 3d-lab).
//
// Cosmetic only: effects read cues and never feed anything back. Their clock is the display
// clock (seconds), so frame rate, pauses and drops change only what is drawn. Particle
// seeds are hashes of cue keys, never game randomness. Turning effects off (the combat
// effects preference or reduced motion) submits `enabled: false`, which also releases the
// atlas textures, and stops the camera impulse.
//
// The atlases are a minimal procedural placeholder until asset-tooling#362 releases the
// baked ARPG combat flipbooks; that issue replaces them and deletes this generator.

import { compileBurst } from "@moritzbrantner/three-d-renderer/flipbook";

const CELL = 32;
const COLUMNS = 2;
const ROWS = 2;
const FRAMES = COLUMNS * ROWS;
const SIZE = CELL * COLUMNS;

// Draws `FRAMES` cells; `shape(frame, x, y)` returns alpha 0..1 for a cell-local point
// centred on the cell (cell units -1..1).
function placeholderAtlas(key, rgb, shape) {
  const pixels = new Uint8Array(SIZE * SIZE * 4);
  for (let frame = 0; frame < FRAMES; frame += 1) {
    const cellX = (frame % COLUMNS) * CELL;
    const cellY = Math.floor(frame / COLUMNS) * CELL;
    for (let y = 0; y < CELL; y += 1)
      for (let x = 0; x < CELL; x += 1) {
        const u = ((x + 0.5) / CELL) * 2 - 1;
        const v = ((y + 0.5) / CELL) * 2 - 1;
        // Two texels of transparent border so the renderer's half-texel inset never bleeds.
        const border = x < 2 || y < 2 || x >= CELL - 2 || y >= CELL - 2;
        const alpha = border ? 0 : Math.max(0, Math.min(1, shape(frame / (FRAMES - 1), u, v)));
        const index = ((cellY + y) * SIZE + cellX + x) * 4;
        pixels[index] = rgb[0];
        pixels[index + 1] = rgb[1];
        pixels[index + 2] = rgb[2];
        pixels[index + 3] = Math.round(alpha * 255);
      }
  }
  return {
    resourceKey: `arpg-placeholder-vfx:${key}:v1`,
    width: SIZE,
    height: SIZE,
    columns: COLUMNS,
    rows: ROWS,
    frameCount: FRAMES,
    filter: "linear",
    pixels,
  };
}

const soft = (distance, radius) => 1 - Math.min(1, distance / radius) ** 2;

let atlasCache = null;
export function combatAtlases() {
  atlasCache ??= {
    // A warm round spark that shrinks.
    spark: placeholderAtlas("spark", [255, 190, 104], (t, u, v) =>
      soft(Math.hypot(u, v), 0.7 - 0.45 * t),
    ),
    // A cool four-pointed glint, told apart from the spark by its shape as well as its hue.
    blockSpark: placeholderAtlas("block-spark", [170, 212, 255], (t, u, v) => {
      const reach = 0.85 - 0.5 * t;
      return Math.max(
        soft(Math.abs(u) + Math.abs(v) * 4, reach),
        soft(Math.abs(v) + Math.abs(u) * 4, reach),
      );
    }),
    // A grey-brown puff that swells and thins.
    dust: placeholderAtlas(
      "dust",
      [154, 138, 116],
      (t, u, v) => soft(Math.hypot(u, v), 0.45 + 0.4 * t) * (1 - 0.8 * t),
    ),
    // An elongated streak that fades.
    trail: placeholderAtlas(
      "trail",
      [236, 228, 210],
      (t, u, v) => soft(Math.hypot(u, v * 4), 0.9) * (0.8 - 0.7 * t),
    ),
  };
  return atlasCache;
}

// Burst presets (3d-lab `BurstPreset`), validated once by the renderer's own compiler.
export const BURST_PRESETS = Object.freeze({
  hit: {
    count: 10,
    lifetime: 0.28,
    speed: [1.6, 3],
    spread: 1.1,
    gravity: [0, -6, 0],
    endScale: 0.4,
  },
  blocked: {
    count: 8,
    lifetime: 0.3,
    speed: [1.2, 2.4],
    spread: 0.9,
    gravity: [0, -4, 0],
    endScale: 0.5,
  },
  guardBroken: {
    count: 18,
    lifetime: 0.45,
    speed: [1.6, 3.4],
    spread: 1.4,
    gravity: [0, -5, 0],
    endScale: 0.5,
  },
  dust: {
    count: 6,
    lifetime: 0.5,
    speed: [0.3, 0.8],
    spread: 1.2,
    gravity: [0, -0.4, 0],
    endScale: 1.8,
  },
});
const BURSTS = {
  hit: { atlas: "spark", preset: "hit", scale: 0.16 },
  blocked: { atlas: "blockSpark", preset: "blocked", scale: 0.22 },
  guardBroken: { atlas: "blockSpark", preset: "guardBroken", scale: 0.26 },
  obstructed: { atlas: "dust", preset: "dust", scale: 0.3 },
  arrowLanded: { atlas: "dust", preset: "dust", scale: 0.3 },
};
for (const preset of Object.values(BURST_PRESETS))
  compileBurst({
    id: "check",
    atlas: "spark",
    space: "world",
    origin: [0, 0, 0],
    startTime: 0,
    seed: 1,
    scale: 1,
    ...preset,
  });

const TRAIL_SEGMENTS = 5;
const TRAIL_DURATION = 0.22;
const TRAIL_STAGGER = 0.025;
const TRAIL_RADIUS = 0.9;
const TRAIL_ARC = 1.6;
const IMPULSE_DURATION = 0.18;
const IMPULSE_DEALT = 0.04;
const IMPULSE_TAKEN = 0.09;
const IMPULSE_LIMIT = 0.12;
// Longest effect lifetime: past it a cue's start time is forgotten.
const LONGEST = 0.6;

export const DEFAULT_MAX_EFFECTS = 48;

function hash(text) {
  let value = 2166136261;
  for (let index = 0; index < text.length; index += 1)
    value = Math.imul(value ^ text.charCodeAt(index), 16777619);
  return value >>> 0;
}

const isPlayer = (party, playerId) => party?.kind === "player" && party.id === playerId;

// The world point (metres) an impact at `position` (world units) is drawn at: chest height.
function impactPoint(position, scale) {
  return [position[0] / scale, Math.max(position[1] / scale, 0.5) + 0.4, position[2] / scale];
}

export function createCombatVfx({ maxEffects = DEFAULT_MAX_EFFECTS } = {}) {
  // Display-clock start of every cue drawn so far, by cue key.
  const started = new Map();

  return {
    // The renderer `effects` for one drawn frame, whether any effect is still playing (the
    // caller keeps drawing while it is), and the camera impulse offset in metres.
    frame({ cues, snapshot, playerId, time, enabled }) {
      if (!enabled || !snapshot) {
        started.clear();
        return {
          effects: { time, enabled: false, atlases: [], instances: [] },
          active: false,
          impulse: null,
        };
      }
      const scale = snapshot.worldUnitsPerMeter;
      const atlases = combatAtlases();
      const bursts = [];
      const instances = [];
      const impulse = [0, 0, 0];
      let active = false;
      const live = new Set();
      for (const cue of cues) {
        live.add(cue.key);
        if (!started.has(cue.key)) started.set(cue.key, time);
        const start = started.get(cue.key);
        const age = Math.max(0, time - start);
        if (age >= LONGEST) continue;
        active = true;
        const seed = hash(cue.key);
        const burst = BURSTS[cue.kind];
        if (burst && cue.position) {
          bursts.push({
            id: cue.key,
            atlas: atlases[burst.atlas].resourceKey,
            space: "world",
            origin: impactPoint(cue.position, scale),
            startTime: start,
            seed,
            scale: burst.scale,
            ...BURST_PRESETS[burst.preset],
          });
        }
        if (cue.kind === "swing" && cue.position && cue.direction) {
          const [x, z] = cue.direction;
          const facing = Math.atan2(z, x);
          const [cx, , cz] = impactPoint(cue.position, scale);
          for (let segment = 0; segment < TRAIL_SEGMENTS; segment += 1) {
            const angle = facing - TRAIL_ARC / 2 + (TRAIL_ARC * segment) / (TRAIL_SEGMENTS - 1);
            instances.push({
              id: `${cue.key}:${segment}`,
              atlas: atlases.trail.resourceKey,
              space: "world",
              origin: [
                cx + Math.cos(angle) * TRAIL_RADIUS,
                1.0,
                cz + Math.sin(angle) * TRAIL_RADIUS,
              ],
              startTime: start + segment * TRAIL_STAGGER,
              duration: TRAIL_DURATION,
              scale: [0.5, 0.18],
            });
          }
        }
        // A short damped kick for a hit the local player dealt or took.
        if (cue.kind === "hit" && age < IMPULSE_DURATION) {
          const amplitude = isPlayer(cue.target, playerId)
            ? IMPULSE_TAKEN
            : isPlayer(cue.source, playerId)
              ? IMPULSE_DEALT
              : 0;
          if (amplitude > 0) {
            const fade = (1 - age / IMPULSE_DURATION) ** 2;
            const angle = ((seed % 360) * Math.PI) / 180;
            impulse[0] += Math.cos(angle) * amplitude * fade;
            impulse[1] += amplitude * 0.5 * fade;
            impulse[2] += Math.sin(angle) * amplitude * fade;
          }
        }
      }
      for (const key of started.keys()) if (!live.has(key)) started.delete(key);
      const length = Math.hypot(...impulse);
      const limited =
        length > IMPULSE_LIMIT ? impulse.map((value) => (value * IMPULSE_LIMIT) / length) : impulse;
      return {
        effects: {
          time,
          enabled: true,
          maxInstances: maxEffects,
          atlases: Object.values(atlases),
          instances,
          bursts,
        },
        active,
        impulse: length > 0 ? limited : null,
      };
    },
  };
}
