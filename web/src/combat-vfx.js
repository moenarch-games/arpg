// Combat VFX (#67, 67b): impact sparks, block sparks, arrow and obstruction dust, weapon
// trails and a restrained camera impulse, all driven by presentation cues
// (presentation-cues.js) and drawn by 3d-lab's cosmetic flipbook effects
// (`frame.effects`, docs/contracts/browser-renderer.md in 3d-lab).
//
// The flipbooks are asset-tooling's baked `combat-effects` release (asset-tooling#362,
// `docs/combat-effects.md`): the setup step pins and verifies its tar into
// `public/effects/combat-effects/`, and `combat-effects/descriptor.json` is the pinned copy
// of its descriptor (atlas layouts and burst presets).
//
// Cosmetic only: effects read cues and never feed anything back. Their clock is the display
// clock (seconds), so frame rate, pauses and drops change only what is drawn. Particle
// seeds are hashes of cue keys, never game randomness. Turning effects off (the combat
// effects preference or reduced motion) submits `enabled: false`, which also releases the
// atlas textures, and stops the camera impulse. Until the atlases load, and for good if
// they fail to, the game plays with no combat effects at all.

import descriptor from "./combat-effects/descriptor.json";
import { decodePngRgba } from "./png-rgba.js";

const EFFECTS = Object.freeze(Object.fromEntries(descriptor.effects.map((e) => [e.key, e])));
const SPARK = "spark.flipbook";
const BLOCK_SPARK = "block-spark.flipbook";
const DUST = "impact-dust.flipbook";
const TRAIL = "trail.flipbook";

// Cue kind → effect and preset, by intensity. `subtle` is the lighter setting (reduced
// flashes). A guard break is the block spark's `heavy` preset, drawn about 1.2× larger.
const BURSTS = Object.freeze({
  hit: { effect: SPARK, standard: "standard", subtle: "subtle" },
  blocked: { effect: BLOCK_SPARK, standard: "standard", subtle: "subtle" },
  guardBroken: { effect: BLOCK_SPARK, standard: "heavy", subtle: "standard", scale: 1.2 },
  obstructed: { effect: DUST, standard: "standard", subtle: "subtle" },
  arrowLanded: { effect: DUST, standard: "standard", subtle: "subtle" },
});

// The swing trail follows the arc of the strike around the striker.
const TRAIL_RADIUS = 0.9;
const TRAIL_ARC = 1.6;
const TRAIL_HEIGHT = 1.0;
const IMPULSE_DURATION = 0.18;
const IMPULSE_DEALT = 0.04;
const IMPULSE_TAKEN = 0.09;
const IMPULSE_LIMIT = 0.12;
// Longest effect lifetime: past it a cue's start time is forgotten.
const LONGEST = Math.max(
  IMPULSE_DURATION,
  ...descriptor.effects.flatMap((effect) =>
    Object.entries(effect.presets).map(([name, preset]) => {
      const instances = effect.instances?.[name];
      return preset.lifetime + (instances ? (instances.segments - 1) * instances.stagger : 0);
    }),
  ),
);

export const DEFAULT_MAX_EFFECTS = 48;

// The renderer atlas (3d-lab `RendererEffectAtlas`) for one descriptor effect.
export function combatAtlas(effect, pixels) {
  const { width, height, columns, rows, frameCount, filter } = effect.atlas;
  return {
    resourceKey: `combat-effects:${effect.png.sha256}`,
    width,
    height,
    columns,
    rows,
    frameCount,
    filter,
    pixels,
  };
}

// Fetches and decodes every atlas of the pinned release from `baseUrl` (the directory
// holding `assets/<sha256>.png`). Resolves to atlases by effect key; rejects on any failure.
export async function loadCombatAtlases({ baseUrl, fetchImpl = fetch, effects = descriptor }) {
  const entries = await Promise.all(
    effects.effects.map(async (effect) => {
      const url = `${baseUrl}assets/${effect.png.sha256}.png`;
      const response = await fetchImpl(url);
      if (!response.ok) throw new Error(`Loading ${url} failed: ${response.status}`);
      const bytes = new Uint8Array(await response.arrayBuffer());
      if (bytes.byteLength !== effect.png.byteLength) throw new Error(`${url} has the wrong size`);
      const image = await decodePngRgba(bytes);
      if (image.width !== effect.atlas.width || image.height !== effect.atlas.height)
        throw new Error(`${url} does not match its declared atlas`);
      return [effect.key, combatAtlas(effect, image.pixels)];
    }),
  );
  return Object.fromEntries(entries);
}

// Starts `load` once. `current()` is null until it resolves, and stays null if it fails:
// the game then plays without combat effects.
export function createAtlasSource(load, warn = console.warn) {
  let atlases = null;
  const ready = Promise.resolve()
    .then(load)
    .then(
      (value) => {
        atlases = value;
        return true;
      },
      (error) => {
        warn("Combat effects are unavailable; playing without them.", error);
        return false;
      },
    );
  return { current: () => atlases, ready };
}

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

const OFF = (time) => ({
  effects: { time, enabled: false, atlases: [], instances: [] },
  active: false,
  impulse: null,
});

// `atlases()` returns the loaded atlases by effect key, or null while they are unavailable.
export function createCombatVfx({ atlases, maxEffects = DEFAULT_MAX_EFFECTS }) {
  // Display-clock start of every cue drawn so far, by cue key.
  const started = new Map();

  return {
    // The renderer `effects` for one drawn frame, whether any effect is still playing (the
    // caller keeps drawing while it is), and the camera impulse offset in metres. `subtle`
    // selects the lighter presets.
    frame({ cues, snapshot, playerId, time, enabled, subtle = false }) {
      const loaded = enabled && snapshot ? atlases() : null;
      if (!loaded) {
        started.clear();
        return OFF(time);
      }
      const intensity = subtle ? "subtle" : "standard";
      const scale = snapshot.worldUnitsPerMeter;
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
          const effect = EFFECTS[burst.effect];
          bursts.push({
            id: cue.key,
            atlas: loaded[burst.effect].resourceKey,
            space: "world",
            origin: impactPoint(cue.position, scale),
            startTime: start,
            seed,
            scale: effect.scale * (burst.scale ?? 1),
            ...effect.presets[burst[intensity]],
          });
        }
        if (cue.kind === "swing" && cue.position && cue.direction) {
          const effect = EFFECTS[TRAIL];
          const { segments, stagger } = effect.instances[intensity];
          const duration = effect.presets[intensity].lifetime;
          const [x, z] = cue.direction;
          const facing = Math.atan2(z, x);
          const [cx, , cz] = impactPoint(cue.position, scale);
          for (let segment = 0; segment < segments; segment += 1) {
            const along = segments > 1 ? segment / (segments - 1) : 0.5;
            const angle = facing - TRAIL_ARC / 2 + TRAIL_ARC * along;
            instances.push({
              id: `${cue.key}:${segment}`,
              atlas: loaded[TRAIL].resourceKey,
              space: "world",
              origin: [
                cx + Math.cos(angle) * TRAIL_RADIUS,
                TRAIL_HEIGHT,
                cz + Math.sin(angle) * TRAIL_RADIUS,
              ],
              startTime: start + segment * stagger,
              duration,
              scale: effect.scale,
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
          atlases: Object.values(loaded),
          instances,
          bursts,
        },
        active,
        impulse: length > 0 ? limited : null,
      };
    },
  };
}
