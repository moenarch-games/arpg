// Acceptance contract for applying presentation effects through the frame presenter (#61,
// first slice). Extends the #88 presenter contract in `frame-presenter.test.ts`.
//
// `PresenterView` gains `effects` (from `resolvePresentationEffects`, default FULL_EFFECTS
// when omitted) and `buildFrame` receives it as a sixth argument:
//
//   buildFrame(snapshot, playerId, width, height, accent, effects)
//
// `configure({ effects })` applies in place: it never creates or disposes a renderer, never
// re-subscribes to the snapshot store or re-observes the canvas, and schedules a redraw of
// the latest snapshot so a change shows while the game is paused (no new snapshot arrives).
import { expect, test } from "bun:test";
import { createFramePresenter } from "./frame-presenter.ts";
import { createSnapshotStore } from "./snapshot-store.js";
import { FULL_EFFECTS, resolvePresentationEffects } from "./presentation-effects.js";

const REDUCED = resolvePresentationEffects({
  graphics: { shadows: true, pixelRatioLimit: 2 },
  accessibility: { motion: "reduced", reducedFlashes: true, textScale: 1.5 },
});

function harness(view = {}) {
  const renderers = [];
  const built = [];
  const frames = new Map();
  let subscriptions = 0;
  let observations = 0;
  let nextFrame = 1;
  const store = createSnapshotStore();
  const options = {
    canvas: { getBoundingClientRect: () => ({ width: 640, height: 480 }) },
    snapshots: {
      getSnapshot: () => store.getSnapshot(),
      subscribe(listener) {
        subscriptions += 1;
        return store.subscribe(listener);
      },
    },
    createRenderer(_canvas, rendererOptions) {
      const renderer = {
        options: rendererOptions,
        disposed: 0,
        rendered: [],
        setSize() {},
        render(frame) {
          renderer.rendered.push(frame);
        },
        dispose() {
          renderer.disposed += 1;
        },
      };
      renderers.push(renderer);
      return renderer;
    },
    buildFrame(snapshot, playerId, width, height, accent, effects) {
      const frame = { tick: snapshot.tick, playerId, accent, effects };
      built.push(frame);
      return frame;
    },
    observeResize() {
      observations += 1;
      return () => {};
    },
    requestFrame(callback) {
      const handle = nextFrame++;
      frames.set(handle, callback);
      return handle;
    },
    cancelFrame(handle) {
      frames.delete(handle);
    },
    pixelRatio: () => 1,
    view: {
      playerId: 1,
      accent: "#d6b45f",
      graphics: { shadows: true, pixelRatioLimit: 2 },
      ...view,
    },
  };
  const flush = () => {
    const pending = [...frames.values()];
    frames.clear();
    for (const callback of pending) callback(0);
  };
  return {
    options,
    store,
    renderers,
    built,
    frames,
    flush,
    subscriptions: () => subscriptions,
    observations: () => observations,
  };
}

const snapshot = (tick) => ({
  tick,
  players: [{ id: 1, position: [0, 0, 0], action: null }],
  monsters: [],
});

test("frames are built with the full effects when the view names none", () => {
  const h = harness();
  h.store.publish(snapshot(1));
  createFramePresenter(h.options);
  expect(h.built.at(-1).effects).toEqual(FULL_EFFECTS);
});

test("the initial view's effects reach every built frame", () => {
  const h = harness({ effects: REDUCED });
  h.store.publish(snapshot(1));
  createFramePresenter(h.options);
  h.store.publish(snapshot(2));
  h.flush();
  expect(h.built.length).toBeGreaterThan(0);
  for (const frame of h.built) expect(frame.effects).toBe(REDUCED);
});

test("changing effects redraws the paused frame in place without a new renderer", () => {
  const h = harness();
  h.store.publish(snapshot(7));
  const presenter = createFramePresenter(h.options);
  const drawn = h.renderers[0].rendered.length;

  presenter.configure({ effects: REDUCED });
  expect(h.frames.size).toBe(1);
  h.flush();

  expect(h.renderers).toHaveLength(1);
  expect(h.renderers[0].disposed).toBe(0);
  expect(h.subscriptions()).toBe(1);
  expect(h.observations()).toBe(1);
  expect(h.renderers[0].rendered.length).toBe(drawn + 1);
  // No new snapshot arrived: the same tick is redrawn with the new effects.
  expect(h.built.at(-1)).toMatchObject({ tick: 7, playerId: 1, effects: REDUCED });

  presenter.configure({ effects: FULL_EFFECTS });
  h.flush();
  expect(h.renderers).toHaveLength(1);
  expect(h.built.at(-1)).toMatchObject({ tick: 7, effects: FULL_EFFECTS });
});

test("effects survive unrelated view changes and a graphics renderer swap", () => {
  const h = harness();
  h.store.publish(snapshot(3));
  const presenter = createFramePresenter(h.options);
  presenter.configure({ effects: REDUCED });
  h.flush();

  presenter.configure({ accent: "#7f9d72" });
  h.flush();
  expect(h.built.at(-1)).toMatchObject({ accent: "#7f9d72", effects: REDUCED });

  // Graphics options are creation-time renderer options: this one swap is the existing
  // behaviour, and the current effects carry over to the new renderer's frames.
  presenter.configure({ graphics: { shadows: false, pixelRatioLimit: 2 } });
  h.flush();
  expect(h.renderers).toHaveLength(2);
  expect(h.renderers[1].rendered.at(-1)).toMatchObject({ tick: 3, effects: REDUCED });
});

test("configuring effects after dispose draws and creates nothing", () => {
  const h = harness();
  h.store.publish(snapshot(1));
  const presenter = createFramePresenter(h.options);
  presenter.dispose();
  const built = h.built.length;
  presenter.configure({ effects: REDUCED });
  h.flush();
  expect(h.built.length).toBe(built);
  expect(h.renderers).toHaveLength(1);
});
