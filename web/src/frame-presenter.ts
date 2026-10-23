// Frame presentation owner for one game canvas (#88): the renderer, the frame projection and
// the resize observer live here instead of in a React effect. The composition creates one
// presenter when the canvas mounts, forwards view changes through `configure`, and disposes
// it when the canvas unmounts.
//
// The presenter subscribes to the session snapshot store and draws at most once per
// animation frame from the latest snapshot; one-tick active action phases seen between draws
// still reach the drawn frame. The focused player, the character accent and the viewport are
// applied in place. Graphics options the renderer only takes at creation (shadows, pixel
// ratio limit) replace the renderer, and only when they actually change. Presentation
// effects (#61: palette, stagger lift) also apply in place and redraw the latest snapshot, so
// a change shows while the game is paused.
//
// Presentation only: it reads projected snapshots and never decides gameplay outcomes.

import { createActiveCueTracker } from "./frame-cues.js";
import { FULL_EFFECTS } from "./presentation-effects.js";

export interface GraphicsOptions {
  shadows: boolean;
  pixelRatioLimit: number;
}

export interface PresenterView {
  playerId: number | null;
  accent: string;
  graphics: GraphicsOptions;
  // From `resolvePresentationEffects`; FULL_EFFECTS when omitted.
  effects?: PresentationEffects;
}

export type PresentationEffects = typeof FULL_EFFECTS;

export interface CanvasLike {
  getBoundingClientRect(): { width: number; height: number };
}

export interface SceneRendererLike<Frame> {
  setSize(width: number, height: number, pixelRatio: number): void;
  render(frame: Frame): void;
  // Adds and removes imported character roots in the renderer's scene (#55).
  renderImportedRoots?(frame: { camera: unknown; roots: unknown[] }): unknown;
  dispose(): void;
}

// In-world humanoid characters (#55): synced to each drawn snapshot before the frame.
export interface CharacterLayerLike<Snapshot> {
  sync(snapshot: Snapshot): { roots: unknown[]; changed: boolean };
  dispose(): void;
}

export interface SnapshotSource<Snapshot> {
  getSnapshot(): Snapshot | null;
  subscribe(listener: () => void): () => void;
}

export interface FramePresenterOptions<Canvas extends CanvasLike, Snapshot, Frame> {
  canvas: Canvas;
  snapshots: SnapshotSource<Snapshot>;
  createRenderer(
    canvas: Canvas,
    options: { background: string; shadows: boolean; pixelRatioLimit: number },
  ): SceneRendererLike<Frame>;
  buildFrame(
    snapshot: Snapshot,
    playerId: number | null,
    width: number,
    height: number,
    accent: string,
    effects: PresentationEffects,
  ): Frame;
  observeResize(canvas: Canvas, onResize: () => void): () => void;
  requestFrame(callback: (time: number) => void): unknown;
  cancelFrame(handle: unknown): void;
  pixelRatio?: () => number;
  background?: string;
  characters?: CharacterLayerLike<Snapshot>;
  view: PresenterView;
}

export const DEFAULT_BACKGROUND = "#11100f";

const sameGraphics = (left: GraphicsOptions, right: GraphicsOptions) =>
  left.shadows === right.shadows && left.pixelRatioLimit === right.pixelRatioLimit;

export function createFramePresenter<
  Canvas extends CanvasLike,
  Snapshot,
  Frame extends { camera: unknown },
>(options: FramePresenterOptions<Canvas, Snapshot, Frame>) {
  const { canvas, snapshots } = options;
  const pixelRatio = options.pixelRatio ?? (() => globalThis.devicePixelRatio ?? 1);
  const background = options.background ?? DEFAULT_BACKGROUND;
  let view: PresenterView = {
    effects: FULL_EFFECTS,
    ...options.view,
    graphics: { ...options.view.graphics },
  };
  let disposed = false;
  let frameRequest: unknown = null;
  const cues = createActiveCueTracker();

  const createRenderer = () =>
    options.createRenderer(canvas, {
      background,
      shadows: view.graphics.shadows,
      pixelRatioLimit: view.graphics.pixelRatioLimit,
    });
  let renderer = createRenderer();
  // A new renderer has an empty scene: character roots are handed to it on its first draw.
  let rootsSubmitted = false;

  const draw = () => {
    const snapshot = cues.take(snapshots.getSnapshot());
    if (!snapshot) return;
    const rect = canvas.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return;
    const frame = options.buildFrame(
      snapshot,
      view.playerId,
      rect.width,
      rect.height,
      view.accent,
      view.effects ?? FULL_EFFECTS,
    );
    if (options.characters && renderer.renderImportedRoots) {
      const { roots, changed } = options.characters.sync(snapshot);
      if (changed || !rootsSubmitted) renderer.renderImportedRoots({ camera: frame.camera, roots });
      rootsSubmitted = true;
    }
    renderer.render(frame);
  };

  // Coalesces every request until the next animation frame into one draw.
  const scheduleDraw = () => {
    if (disposed || frameRequest !== null) return;
    frameRequest = options.requestFrame(() => {
      frameRequest = null;
      if (!disposed) draw();
    });
  };

  const resize = () => {
    if (disposed) return;
    const rect = canvas.getBoundingClientRect();
    if (rect.width <= 0 || rect.height <= 0) return;
    renderer.setSize(rect.width, rect.height, pixelRatio());
    // A resized drawing buffer is blank until the next draw. Draw now and drop any queued
    // frame: this draw already consumes its cue, and a second draw before paint would
    // overwrite a one-tick active cue with the latest snapshot.
    if (frameRequest !== null) {
      options.cancelFrame(frameRequest);
      frameRequest = null;
    }
    draw();
  };

  const unsubscribe = snapshots.subscribe(() => {
    cues.observe(snapshots.getSnapshot());
    scheduleDraw();
  });
  const disconnect = options.observeResize(canvas, resize);
  resize();

  return {
    // Applies a view change in place; the next draw (scheduled now) uses it.
    configure(patch: Partial<PresenterView>) {
      if (disposed) return;
      const { graphics, ...rest } = patch;
      view = { ...view, ...rest };
      if (graphics && !sameGraphics(graphics, view.graphics)) {
        view = { ...view, graphics: { ...graphics } };
        // The renderer takes these options only at creation: release the old one first so a
        // single renderer ever holds the canvas.
        renderer.dispose();
        renderer = createRenderer();
        rootsSubmitted = false;
        resize();
        return;
      }
      scheduleDraw();
    },

    dispose() {
      if (disposed) return;
      disposed = true;
      unsubscribe();
      if (frameRequest !== null) options.cancelFrame(frameRequest);
      frameRequest = null;
      disconnect();
      options.characters?.dispose();
      renderer.dispose();
    },
  };
}

export type FramePresenter = ReturnType<typeof createFramePresenter>;
