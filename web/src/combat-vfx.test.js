import { describe, expect, test } from "bun:test";
import { compileBurst, compileFlipbookLayout } from "@moritzbrantner/three-d-renderer/flipbook";
import { combatAtlases, createCombatVfx, DEFAULT_MAX_EFFECTS } from "./combat-vfx.js";
import { createFramePresenter } from "./frame-presenter.ts";
import { resolvePresentationEffects } from "./presentation-effects.js";
import { createSnapshotStore } from "./snapshot-store.js";

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
const frameOf = (vfx, cues, time, enabled = true) =>
  vfx.frame({ cues, snapshot, playerId: 1, time, enabled });

describe("combat vfx", () => {
  test("placeholder atlases satisfy the renderer's layout contract", () => {
    for (const atlas of Object.values(combatAtlases())) {
      compileFlipbookLayout(atlas);
      expect(atlas.pixels.length).toBe(atlas.width * atlas.height * 4);
      expect(atlas.width % atlas.columns).toBe(0);
    }
  });

  test("a hit is one seeded spark burst at the target that keeps its start time", () => {
    const vfx = createCombatVfx();
    const first = frameOf(vfx, [hit()], 1);
    const [burst] = first.effects.bursts;
    expect(burst.atlas).toBe(combatAtlases().spark.resourceKey);
    expect(burst.origin).toEqual([2, 0.9, 1]);
    expect(burst.startTime).toBe(1);
    expect(() => compileBurst(burst)).not.toThrow();
    expect(first.effects.maxInstances).toBe(DEFAULT_MAX_EFFECTS);
    expect(first.active).toBe(true);
    const later = frameOf(vfx, [hit()], 1.2);
    expect(later.effects.bursts[0]).toEqual({ ...burst });
  });

  test("blocks use the block spark, swings draw a trail, whiffs add nothing more", () => {
    const vfx = createCombatVfx();
    const { effects } = frameOf(
      vfx,
      [
        hit({ key: "b", kind: "blocked", damage: 0 }),
        swing,
        { key: "w", tick: 6, kind: "whiff", playerId: 1 },
      ],
      0,
    );
    expect(effects.bursts.map((burst) => burst.atlas)).toEqual([
      combatAtlases().blockSpark.resourceKey,
    ]);
    expect(effects.instances).toHaveLength(5);
    expect(new Set(effects.instances.map((instance) => instance.id)).size).toBe(5);
  });

  test("the camera impulse is restrained, stronger when hurt, and decays to nothing", () => {
    const vfx = createCombatVfx();
    const dealt = Math.hypot(...frameOf(vfx, [hit()], 0).impulse);
    const taken = Math.hypot(
      ...frameOf(
        createCombatVfx(),
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
    const vfx = createCombatVfx();
    const off = frameOf(vfx, [hit()], 0, false);
    expect(off).toEqual({
      effects: { time: 0, enabled: false, atlases: [], instances: [] },
      active: false,
      impulse: null,
    });
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
    const vfx = createCombatVfx();
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
