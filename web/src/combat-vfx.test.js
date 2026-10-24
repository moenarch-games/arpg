import { describe, expect, test } from "bun:test";
import { compileBurst, compileFlipbookLayout } from "@moritzbrantner/three-d-renderer/flipbook";
import descriptor from "./combat-effects/descriptor.json";
import {
  combatAtlas,
  createAtlasSource,
  createCombatVfx,
  DEFAULT_MAX_EFFECTS,
  loadCombatAtlases,
} from "./combat-vfx.js";
import { createFramePresenter } from "./frame-presenter.ts";
import { encodePng } from "./png-fixture.js";
import { resolvePresentationEffects } from "./presentation-effects.js";
import { createSnapshotStore } from "./snapshot-store.js";

const effect = (key) => descriptor.effects.find((entry) => entry.key === key);
const SPARK = effect("spark.flipbook");
const BLOCK = effect("block-spark.flipbook");
const DUST = effect("impact-dust.flipbook");
const TRAIL = effect("trail.flipbook");
const keyOf = (entry) => `combat-effects:${entry.png.sha256}`;
// Loaded atlases stand-in: the declared layouts with blank pixels.
const ATLASES = Object.fromEntries(
  descriptor.effects.map((entry) => [
    entry.key,
    combatAtlas(entry, new Uint8Array(entry.atlas.width * entry.atlas.height * 4)),
  ]),
);
const loaded = () => ATLASES;
const vfxOf = () => createCombatVfx({ atlases: loaded });

const snapshot = { tick: 10, worldUnitsPerMeter: 100, players: [{ id: 1 }], monsters: [] };
const hit = (patch = {}) => ({
  key: "strike:player:1:5:monster:7",
  tick: 5,
  kind: "hit",
  source: { kind: "player", id: 1 },
  target: { kind: "monster", id: 7 },
  damage: 25,
  position: [200, 0, 100],
  ...patch,
});
const swing = {
  key: "swing:1:primaryAttack:4",
  tick: 4,
  kind: "swing",
  playerId: 1,
  position: [0, 0, 0],
  direction: [1, 0],
};
const frameOf = (vfx, cues, time, enabled = true, subtle = false) =>
  vfx.frame({ cues, snapshot, playerId: 1, time, enabled, subtle });

describe("combat vfx", () => {
  test("the pinned release's atlases and presets satisfy the renderer's contracts", () => {
    for (const entry of descriptor.effects) {
      const atlas = ATLASES[entry.key];
      compileFlipbookLayout(atlas);
      expect(atlas.resourceKey).toBe(keyOf(entry));
      expect(atlas.width % atlas.columns).toBe(0);
      for (const preset of Object.values(entry.presets))
        compileBurst({
          id: "check",
          atlas: "a",
          space: "world",
          origin: [0, 0, 0],
          startTime: 0,
          seed: 1,
          scale: 1,
          ...preset,
        });
    }
  });

  test("a hit is one seeded spark burst at the target that keeps its start time", () => {
    const vfx = vfxOf();
    const first = frameOf(vfx, [hit()], 1);
    const [burst] = first.effects.bursts;
    expect(burst.atlas).toBe(keyOf(SPARK));
    expect(burst.origin).toEqual([2, 0.9, 1]);
    expect(burst.startTime).toBe(1);
    expect(burst.scale).toBe(SPARK.scale);
    expect(burst).toMatchObject(SPARK.presets.standard);
    expect(() => compileBurst(burst)).not.toThrow();
    expect(first.effects.maxInstances).toBe(DEFAULT_MAX_EFFECTS);
    expect(first.effects.atlases.map((atlas) => atlas.resourceKey).sort()).toEqual(
      descriptor.effects.map(keyOf).sort(),
    );
    expect(first.active).toBe(true);
    const later = frameOf(vfx, [hit()], 1.2);
    expect(later.effects.bursts[0]).toEqual({ ...burst });
  });

  test("each cue kind draws its release effect and preset", () => {
    const { effects } = frameOf(
      vfxOf(),
      [
        hit({ key: "b", kind: "blocked", damage: 0 }),
        hit({ key: "g", kind: "guardBroken", damage: 0 }),
        hit({ key: "o", kind: "obstructed", damage: 0 }),
        hit({ key: "a", kind: "arrowLanded", damage: 0 }),
        swing,
        { key: "w", tick: 6, kind: "whiff", playerId: 1 },
      ],
      0,
    );
    const [blocked, broken, obstructed, landed] = effects.bursts;
    expect(blocked).toMatchObject({ atlas: keyOf(BLOCK), scale: BLOCK.scale });
    expect(blocked).toMatchObject(BLOCK.presets.standard);
    expect(broken).toMatchObject({ atlas: keyOf(BLOCK) });
    expect(broken).toMatchObject(BLOCK.presets.heavy);
    expect(broken.scale).toBeCloseTo(0.264);
    for (const dust of [obstructed, landed]) {
      expect(dust).toMatchObject({ atlas: keyOf(DUST), scale: DUST.scale });
      expect(dust).toMatchObject(DUST.presets.standard);
    }
    expect(effects.bursts).toHaveLength(4);
    // The swing trail: `segments` instances, each one preset lifetime long, `stagger` apart.
    const { segments, stagger } = TRAIL.instances.standard;
    expect(effects.instances).toHaveLength(segments);
    expect(new Set(effects.instances.map((instance) => instance.id)).size).toBe(segments);
    effects.instances.forEach((instance, index) => {
      expect(instance.atlas).toBe(keyOf(TRAIL));
      expect(instance.duration).toBe(TRAIL.presets.standard.lifetime);
      expect(instance.startTime).toBeCloseTo(index * stagger);
      expect(instance.scale).toBe(0.5);
    });
  });

  test("the lighter setting draws the subtle presets", () => {
    const { effects } = frameOf(vfxOf(), [hit(), swing], 0, true, true);
    expect(effects.bursts[0]).toMatchObject(SPARK.presets.subtle);
    expect(effects.instances).toHaveLength(TRAIL.instances.subtle.segments);
    expect(effects.instances[1].startTime).toBeCloseTo(TRAIL.instances.subtle.stagger);
    expect(effects.instances[0].duration).toBe(TRAIL.presets.subtle.lifetime);
  });

  test("the camera impulse is restrained, stronger when hurt, and decays to nothing", () => {
    const vfx = vfxOf();
    const dealt = Math.hypot(...frameOf(vfx, [hit()], 0).impulse);
    const taken = Math.hypot(
      ...frameOf(
        vfxOf(),
        [hit({ source: { kind: "monster", id: 7 }, target: { kind: "player", id: 1 } })],
        0,
      ).impulse,
    );
    expect(dealt).toBeGreaterThan(0);
    expect(taken).toBeGreaterThan(dealt);
    expect(taken).toBeLessThanOrEqual(0.12 + 1e-9);
    expect(frameOf(vfx, [hit()], 0.5).impulse).toBeNull();
    // Effects end, so the presenter stops drawing for them.
    expect(frameOf(vfx, [hit()], 2).active).toBe(false);
  });

  test("disabled effects submit nothing, release the atlases and stop the impulse", () => {
    const off = frameOf(vfxOf(), [hit()], 0, false);
    expect(off).toEqual({
      effects: { time: 0, enabled: false, atlases: [], instances: [] },
      active: false,
      impulse: null,
    });
  });

  test("without loaded atlases the game plays with no combat effects", async () => {
    const warnings = [];
    const source = createAtlasSource(
      () => Promise.reject(new Error("offline")),
      (...args) => warnings.push(args),
    );
    const vfx = createCombatVfx({ atlases: source.current });
    const expected = {
      effects: { time: 0, enabled: false, atlases: [], instances: [] },
      active: false,
      impulse: null,
    };
    expect(frameOf(vfx, [hit()], 0)).toEqual(expected);
    expect(await source.ready).toBe(false);
    expect(warnings).toHaveLength(1);
    expect(frameOf(vfx, [hit()], 0)).toEqual(expected);
  });

  test("loads and decodes every atlas of the release from the static directory", async () => {
    const pixels = Uint8Array.from({ length: 8 * 8 * 4 }, (_, i) => i % 256);
    const png = encodePng(8, 8, pixels);
    const tiny = {
      effects: [
        {
          key: "spark.flipbook",
          atlas: { width: 8, height: 8, columns: 2, rows: 2, frameCount: 4, filter: "linear" },
          png: { sha256: "ab", byteLength: png.byteLength },
        },
      ],
    };
    const requested = [];
    const atlases = await loadCombatAtlases({
      baseUrl: "/arpg/effects/combat-effects/",
      effects: tiny,
      fetchImpl: async (url) => {
        requested.push(url);
        return new Response(png);
      },
    });
    expect(requested).toEqual(["/arpg/effects/combat-effects/assets/ab.png"]);
    expect(atlases["spark.flipbook"]).toEqual({
      resourceKey: "combat-effects:ab",
      width: 8,
      height: 8,
      columns: 2,
      rows: 2,
      frameCount: 4,
      filter: "linear",
      pixels,
    });
    await expect(
      loadCombatAtlases({
        baseUrl: "/",
        effects: tiny,
        fetchImpl: async () => new Response("missing", { status: 404 }),
      }),
    ).rejects.toThrow("404");
  });

  test("combat effects follow the preference and reduced motion", () => {
    const resolve = (accessibility, prefersReducedMotion = false) =>
      resolvePresentationEffects({ accessibility }, { prefersReducedMotion });
    expect(resolve({ combatEffects: true }).combatEffects).toBe(true);
    expect(resolve({ combatEffects: false }).combatEffects).toBe(false);
    expect(resolve({ combatEffects: true, motion: "reduced" }).cameraImpulse).toBe(false);
    expect(resolve({ combatEffects: true }, true).combatEffects).toBe(false);
  });
});

describe("frame presenter with combat vfx", () => {
  function harness(effects) {
    const store = createSnapshotStore();
    const rendered = [];
    const poses = [];
    const frames = new Map();
    let next = 1;
    const pose = { position: [10, 11, 10], target: [0, 0, 0] };
    const cues = [hit()];
    const vfx = vfxOf();
    createFramePresenter({
      canvas: { getBoundingClientRect: () => ({ width: 640, height: 480 }) },
      snapshots: store,
      createRenderer: () => ({
        setSize() {},
        render: (frame) => rendered.push(frame),
        dispose() {},
      }),
      buildFrame: (_snapshot, _id, _w, _h, _accent, _effects, drawn) => {
        poses.push(drawn);
        return { camera: drawn };
      },
      camera: { update: () => true, pose: () => pose, subscribe: () => () => {} },
      vfx: { frame: (input) => vfx.frame({ ...input, cues }) },
      observeResize: () => () => {},
      requestFrame(callback) {
        frames.set(next, callback);
        return next++;
      },
      cancelFrame: (handle) => frames.delete(handle),
      pixelRatio: () => 1,
      view: {
        playerId: 1,
        accent: "#fff",
        graphics: { shadows: true, pixelRatioLimit: 2 },
        effects,
      },
    });
    const flush = (time) => {
      const pending = [...frames.values()];
      frames.clear();
      for (const callback of pending) callback(time);
    };
    return { store, rendered, poses, pose, frames, flush };
  }

  test("effects ride on the frame, keep drawing while they play and nudge only the drawn camera", () => {
    const on = harness(resolvePresentationEffects(undefined));
    on.store.publish(snapshot);
    on.flush(0);
    expect(on.rendered.at(-1).effects.bursts).toHaveLength(1);
    expect(on.poses.at(-1)).not.toEqual(on.pose);
    expect(on.pose).toEqual({ position: [10, 11, 10], target: [0, 0, 0] });
    expect(on.frames.size).toBe(1);
    on.flush(2000);
    expect(on.frames.size).toBe(0);
  });

  test("with effects off the frame disables them and the camera is untouched", () => {
    const off = harness(resolvePresentationEffects({ accessibility: { combatEffects: false } }));
    off.store.publish(snapshot);
    off.flush(0);
    expect(off.rendered.at(-1).effects.enabled).toBe(false);
    expect(off.poses.at(-1)).toEqual(off.pose);
    expect(off.frames.size).toBe(0);
  });
});
