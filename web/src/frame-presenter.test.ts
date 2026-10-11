// Acceptance contract for the frame presenter owned outside React (#88).
//
// `web/src/frame-presenter.ts` owns the renderer, the frame projection and the resize
// observer for one game canvas. React creates it once when the canvas mounts, forwards
// view changes through `configure`, and disposes it when the canvas unmounts. It never
// rebuilds the renderer because the focused player, the character accent or the HUD tick
// changed.
//
// API (everything host-specific is injected, so the presenter is testable without a DOM):
//
//   createFramePresenter({
//     canvas,                         // { getBoundingClientRect(): { width, height } }
//     snapshots,                      // { getSnapshot(), subscribe(listener) => unsubscribe }
//     createRenderer(canvas, { background, shadows, pixelRatioLimit }),
//                                     // => { setSize(w, h, dpr), render(frame), dispose() }
//     buildFrame(snapshot, playerId, width, height, accent),  // => renderer frame
//     observeResize(canvas, onResize),                         // => disconnect()
//     requestFrame(callback),         // => handle (requestAnimationFrame in the browser)
//     cancelFrame(handle),            // cancelAnimationFrame in the browser
//     pixelRatio?: () => number,      // defaults to devicePixelRatio
//     view: { playerId, accent, graphics: { shadows, pixelRatioLimit } },
//   }) => {
//     configure(patch),               // Partial<view>; applies in place
//     dispose(),                      // idempotent; releases everything
//   }
//
// Behaviour:
// - Exactly one renderer exists per presenter. It is created synchronously by
//   `createFramePresenter`, sized from the canvas rect and drawn once if a snapshot exists.
// - Snapshot publishes coalesce into at most one draw per animation frame, always from the
//   latest snapshot. One-tick active action phases seen between draws still reach the drawn
//   frame (the existing `createActiveCueTracker` behaviour).
// - `configure({ playerId })` / `configure({ accent })` never create a renderer; the next
//   draw uses the new values. Graphics options that the renderer can only take at creation
//   (shadows, pixel ratio limit) may replace the renderer, but only when they actually
//   change, disposing the old renderer first, keeping one live renderer, and without
//   re-subscribing to the store or re-observing the canvas.
// - `dispose()` unsubscribes, cancels a pending frame, disconnects the observer and disposes
//   the renderer. After dispose nothing renders and `configure` creates nothing.
import { expect, test } from "bun:test";
import { createFramePresenter } from "./frame-presenter.ts";
import { createSnapshotStore } from "./snapshot-store.js";

const GRAPHICS = { shadows: true, pixelRatioLimit: 2 };

function snapshot(tick, { actionPhase = null } = {}) {
  return {
    tick,
    players: [
      {
        id: 1,
        position: [0, 0, 0],
        action: actionPhase
          ? { kind: "primaryAttack", phase: actionPhase, ticksRemaining: 1 }
          : null,
      },
    ],
    monsters: [],
  };
}

function harness({ view = {} } = {}) {
  const renderers = [];
  const frames = new Map();
  const observers = [];
  const built = [];
  let nextFrame = 1;
  let rect = { width: 800, height: 600 };
  const canvas = { getBoundingClientRect: () => rect };

  const store = createSnapshotStore();
  let subscriptions = 0;
  const snapshots = {
    getSnapshot: () => store.getSnapshot(),
    subscribe(listener) {
      subscriptions += 1;
      const unsubscribe = store.subscribe(listener);
      let active = true;
      return () => {
        if (!active) return;
        active = false;
        subscriptions -= 1;
        unsubscribe();
      };
    },
    publish: (next) => store.publish(next),
  };

  const options = {
    canvas,
    snapshots,
    createRenderer(target, rendererOptions) {
      const renderer = {
        target,
        options: rendererOptions,
        disposed: 0,
        sizes: [],
        rendered: [],
        setSize(width, height, dpr) {
          if (renderer.disposed) throw new Error("setSize after dispose");
          renderer.sizes.push([width, height, dpr]);
        },
        render(frame) {
          if (renderer.disposed) throw new Error("render after dispose");
          renderer.rendered.push(frame);
        },
        dispose() {
          renderer.disposed += 1;
        },
      };
      renderers.push(renderer);
      return renderer;
    },
    buildFrame(current, playerId, width, height, accent) {
      const frame = { snapshot: current, playerId, width, height, accent };
      built.push(frame);
      return frame;
    },
    observeResize(target, onResize) {
      const observer = { target, onResize, connected: true };
      observers.push(observer);
      return () => {
        observer.connected = false;
      };
    },
    requestFrame(callback) {
      const handle = nextFrame++;
      frames.set(handle, callback);
      return handle;
    },
    cancelFrame(handle) {
      frames.delete(handle);
    },
    pixelRatio: () => 1.5,
    view: { playerId: 1, accent: "#d6b45f", graphics: GRAPHICS, ...view },
  };

  const flushFrames = () => {
    const pending = [...frames.entries()];
    frames.clear();
    for (const [, callback] of pending) callback(0);
  };

  return {
    options,
    canvas,
    snapshots,
    renderers,
    frames,
    observers,
    built,
    flushFrames,
    setRect: (next) => {
      rect = next;
    },
    subscriptions: () => subscriptions,
    liveRenderers: () => renderers.filter((renderer) => renderer.disposed === 0),
    connectedObservers: () => observers.filter((observer) => observer.connected),
    lastFrame: () => built.at(-1),
  };
}

test("creates one renderer for the canvas, sizes it and observes resizes once", () => {
  const h = harness();
  createFramePresenter(h.options);

  expect(h.renderers).toHaveLength(1);
  expect(h.renderers[0].target).toBe(h.canvas);
  expect(h.renderers[0].options).toMatchObject({ shadows: true, pixelRatioLimit: 2 });
  expect(h.renderers[0].sizes.at(-1)).toEqual([800, 600, 1.5]);
  expect(h.observers).toHaveLength(1);
  expect(h.observers[0].target).toBe(h.canvas);
  expect(h.subscriptions()).toBe(1);
});

test("a missing snapshot draws nothing and does not throw", () => {
  const h = harness();
  createFramePresenter(h.options);
  h.snapshots.publish(snapshot(1));
  h.snapshots.publish(null);
  h.flushFrames();

  expect(h.renderers[0].rendered).toHaveLength(0);
});

test("snapshot bursts coalesce into one draw per animation frame from the latest snapshot", () => {
  const h = harness();
  createFramePresenter(h.options);
  const drawnBefore = h.renderers[0].rendered.length;

  h.snapshots.publish(snapshot(1));
  h.snapshots.publish(snapshot(2));
  h.snapshots.publish(snapshot(3));
  expect(h.frames.size).toBe(1);
  h.flushFrames();

  expect(h.renderers[0].rendered.length - drawnBefore).toBe(1);
  expect(h.lastFrame()).toMatchObject({
    playerId: 1,
    width: 800,
    height: 600,
    accent: "#d6b45f",
  });
  expect(h.lastFrame().snapshot.tick).toBe(3);
});

test("a one-tick active action between draws still reaches the drawn frame", () => {
  const h = harness();
  createFramePresenter(h.options);

  h.snapshots.publish(snapshot(1, { actionPhase: "active" }));
  h.snapshots.publish(snapshot(2, { actionPhase: "recovery" }));
  h.flushFrames();

  expect(h.lastFrame().snapshot.tick).toBe(2);
  expect(h.lastFrame().snapshot.players[0].action.phase).toBe("active");
});

test("changing the focused player or accent reconfigures in place without a new renderer", () => {
  const h = harness();
  const presenter = createFramePresenter(h.options);
  h.snapshots.publish(snapshot(1));
  h.flushFrames();

  presenter.configure({ playerId: 2 });
  presenter.configure({ accent: "#ff0000" });
  h.snapshots.publish(snapshot(2));
  h.flushFrames();

  expect(h.renderers).toHaveLength(1);
  expect(h.renderers[0].disposed).toBe(0);
  expect(h.lastFrame()).toMatchObject({ playerId: 2, accent: "#ff0000" });
  expect(h.subscriptions()).toBe(1);
  expect(h.observers).toHaveLength(1);
});

test("a view change is drawn without waiting for the next snapshot", () => {
  const h = harness();
  const presenter = createFramePresenter(h.options);
  h.snapshots.publish(snapshot(1));
  h.flushFrames();

  presenter.configure({ playerId: 3 });
  h.flushFrames();

  expect(h.lastFrame()).toMatchObject({ playerId: 3 });
  expect(h.lastFrame().snapshot.tick).toBe(1);
});

test("unchanged graphics options never replace the renderer", () => {
  const h = harness();
  const presenter = createFramePresenter(h.options);

  presenter.configure({ graphics: { ...GRAPHICS } });
  presenter.configure({ playerId: 1, accent: "#d6b45f", graphics: { ...GRAPHICS } });

  expect(h.renderers).toHaveLength(1);
  expect(h.renderers[0].disposed).toBe(0);
});

test("changed graphics keep one live renderer without re-subscribing or re-observing", () => {
  const h = harness();
  const presenter = createFramePresenter(h.options);
  h.snapshots.publish(snapshot(4));
  h.flushFrames();

  presenter.configure({ graphics: { shadows: false, pixelRatioLimit: 1 } });
  h.flushFrames();

  expect(h.liveRenderers()).toHaveLength(1);
  expect(h.renderers.length).toBeLessThanOrEqual(2);
  const live = h.liveRenderers()[0];
  if (live !== h.renderers[0]) {
    // Replaced: the old renderer is released once and the new one takes the new options.
    expect(h.renderers[0].disposed).toBe(1);
    expect(live.options).toMatchObject({ shadows: false, pixelRatioLimit: 1 });
  }
  expect(live.sizes.length).toBeGreaterThan(0);
  expect(live.rendered.at(-1).snapshot.tick).toBe(4);
  expect(h.subscriptions()).toBe(1);
  expect(h.observers).toHaveLength(1);
  expect(h.connectedObservers()).toHaveLength(1);
});

test("a resize re-sizes the renderer and redraws with the new viewport", () => {
  const h = harness();
  createFramePresenter(h.options);
  h.snapshots.publish(snapshot(1));
  h.flushFrames();

  h.setRect({ width: 390, height: 844 });
  h.observers[0].onResize();
  h.flushFrames();

  expect(h.renderers[0].sizes.at(-1)).toEqual([390, 844, 1.5]);
  expect(h.lastFrame()).toMatchObject({ width: 390, height: 844 });
  expect(h.renderers).toHaveLength(1);
});

test("dispose releases the subscription, pending frame, observer and renderer", () => {
  const h = harness();
  const presenter = createFramePresenter(h.options);
  h.snapshots.publish(snapshot(1));
  expect(h.frames.size).toBe(1);

  presenter.dispose();

  expect(h.subscriptions()).toBe(0);
  expect(h.frames.size).toBe(0);
  expect(h.connectedObservers()).toHaveLength(0);
  expect(h.renderers[0].disposed).toBe(1);

  h.snapshots.publish(snapshot(2));
  h.flushFrames();
  expect(h.frames.size).toBe(0);
  expect(h.renderers[0].rendered).toHaveLength(0);
});

test("dispose is idempotent and configure after dispose creates nothing", () => {
  const h = harness();
  const presenter = createFramePresenter(h.options);

  presenter.dispose();
  presenter.dispose();
  presenter.configure({ playerId: 2, graphics: { shadows: false, pixelRatioLimit: 1 } });
  h.flushFrames();

  expect(h.renderers).toHaveLength(1);
  expect(h.renderers[0].disposed).toBe(1);
  expect(h.liveRenderers()).toHaveLength(0);
});

test("repeated create and dispose cycles leave no renderer, listener, observer or frame", () => {
  const h = harness();
  for (let round = 0; round < 20; round += 1) {
    const presenter = createFramePresenter(h.options);
    h.snapshots.publish(snapshot(round + 1));
    if (round % 2 === 0) h.flushFrames();
    presenter.configure({ playerId: (round % 3) + 1 });
    presenter.dispose();
  }

  expect(h.renderers).toHaveLength(20);
  expect(h.liveRenderers()).toHaveLength(0);
  expect(h.renderers.every((renderer) => renderer.disposed === 1)).toBe(true);
  expect(h.subscriptions()).toBe(0);
  expect(h.connectedObservers()).toHaveLength(0);
  expect(h.frames.size).toBe(0);
});
