// The gameplay follow camera (#58, Atlas 4.1.4 option D): a fixed isometric view that
// follows the focused player with frame-rate-independent smoothing, a bounded mouse-wheel /
// pinch zoom and an explicit reset. There is no manual rotation.
//
// Presentation only: the camera never feeds gameplay. Pointer aim projects through the pose
// the last frame was drawn with, so the cursor maps onto the ground the player sees.
import * as THREE from "three";
import { persistStoredValue, readStoredValue } from "./preferences.js";

// Offset from the followed point (metres) at zoom 1, and the vertical field of view.
export const CAMERA_OFFSET = [10, 11, 10];
export const CAMERA_FOV_DEGREES = 43;
export const ZOOM_MIN = 0.6;
export const ZOOM_MAX = 1.6;
export const DEFAULT_ZOOM = 1;
export const CAMERA_ZOOM_KEY = "arpg-camera-zoom";
// Follow rate (1/s): the camera closes ~63% of the gap every 1/FOLLOW_RATE seconds.
const FOLLOW_RATE = 8;
// A jump this far (metres) is a teleport, respawn or new focus: snap instead of gliding.
const SNAP_DISTANCE = 6;
// Larger frame gaps (background tab, debugger) count as this long.
const MAX_STEP_SECONDS = 0.1;
const SETTLED_DISTANCE = 0.002;
// Wheel delta (pixels) per e-fold of zoom.
const WHEEL_PIXELS_PER_EFOLD = 600;

export const clampZoom = (zoom) =>
  Number.isFinite(zoom) ? Math.min(ZOOM_MAX, Math.max(ZOOM_MIN, zoom)) : DEFAULT_ZOOM;

export function loadCameraZoom(storage) {
  return clampZoom(Number.parseFloat(readStoredValue(CAMERA_ZOOM_KEY, `${DEFAULT_ZOOM}`, storage)));
}

export function persistCameraZoom(zoom, storage) {
  return persistStoredValue(CAMERA_ZOOM_KEY, `${clampZoom(zoom)}`, storage);
}

// The camera pose looking at `target` (a ground point) from the zoomed isometric offset.
export function cameraPose(target, zoom = DEFAULT_ZOOM) {
  return {
    target: [target[0], target[1], target[2]],
    position: [
      target[0] + CAMERA_OFFSET[0] * zoom,
      target[1] + CAMERA_OFFSET[1] * zoom,
      target[2] + CAMERA_OFFSET[2] * zoom,
    ],
  };
}

// The ground point under the focused (or first) player, in metres; the origin without one.
export function focusPoint(snapshot, playerId) {
  const scale = snapshot.worldUnitsPerMeter;
  const focus = snapshot.players.find((player) => player.id === playerId) ?? snapshot.players[0];
  return focus ? [focus.position[0] / scale, 0, focus.position[2] / scale] : [0, 0, 0];
}

// A three.js camera for `pose`, reused by callers that need its matrices.
export function poseCamera(pose, aspect, camera = new THREE.PerspectiveCamera()) {
  camera.fov = CAMERA_FOV_DEGREES;
  camera.aspect = aspect;
  camera.near = 0.1;
  camera.far = 100;
  camera.position.set(...pose.position);
  camera.lookAt(...pose.target);
  camera.updateProjectionMatrix();
  camera.updateMatrixWorld(true);
  return camera;
}

export function createFollowCamera({ zoom = DEFAULT_ZOOM } = {}) {
  let currentZoom = clampZoom(zoom);
  let followed = null;
  let focus = [0, 0, 0];
  let lastTime = null;
  const listeners = new Set();
  const changed = () => {
    for (const listener of listeners) listener();
  };
  const setZoom = (next) => {
    const clamped = clampZoom(next);
    if (clamped === currentZoom) return;
    currentZoom = clamped;
    changed();
  };

  return {
    // Moves the followed point toward `target` for a frame drawn at `time` (ms). Returns
    // whether the camera has settled, so the caller can stop drawing frames for it.
    follow(target, time) {
      focus = [target[0], target[1], target[2]];
      const step =
        lastTime === null || time === undefined
          ? 0
          : Math.min(MAX_STEP_SECONDS, Math.max(0, (time - lastTime) / 1000));
      if (time !== undefined) lastTime = time;
      const gap = followed
        ? Math.hypot(target[0] - followed[0], target[1] - followed[1], target[2] - followed[2])
        : Infinity;
      if (gap > SNAP_DISTANCE || gap <= SETTLED_DISTANCE) {
        followed = [...focus];
        return true;
      }
      const blend = 1 - Math.exp(-FOLLOW_RATE * step);
      followed = followed.map((value, axis) => value + (target[axis] - value) * blend);
      return false;
    },
    pose() {
      return cameraPose(followed ?? focus, currentZoom);
    },
    // The latest unsmoothed followed point: where the focused player was drawn.
    focus: () => [...focus],
    zoom: () => currentZoom,
    setZoom,
    // Positive `deltaY` (wheel toward the user) zooms out.
    zoomByWheel(deltaY) {
      setZoom(currentZoom * Math.exp(deltaY / WHEEL_PIXELS_PER_EFOLD));
    },
    // Restores the default zoom and snaps onto the player on the next frame.
    reset() {
      currentZoom = DEFAULT_ZOOM;
      followed = null;
      changed();
    },
    // A new session or world: the next frame snaps onto its player.
    recenter() {
      followed = null;
      lastTime = null;
      changed();
    },
    subscribe(listener) {
      listeners.add(listener);
      return () => listeners.delete(listener);
    },
  };
}

// Two-finger pinch on the canvas: `move` returns the zoom factor to apply, or null.
export function createPinchZoom() {
  const touches = new Map();
  let lastSpread = null;
  const spread = () => {
    const [first, second] = [...touches.values()];
    return Math.hypot(first[0] - second[0], first[1] - second[1]);
  };
  return {
    down(pointerId, x, y) {
      touches.set(pointerId, [x, y]);
      lastSpread = touches.size === 2 ? spread() : null;
    },
    move(pointerId, x, y) {
      if (!touches.has(pointerId)) return null;
      touches.set(pointerId, [x, y]);
      if (touches.size !== 2 || !lastSpread) return null;
      const next = spread();
      if (!(next > 0)) return null;
      // Spreading the fingers apart zooms in (moves the camera closer).
      const factor = lastSpread / next;
      lastSpread = next;
      return factor;
    },
    up(pointerId) {
      touches.delete(pointerId);
      lastSpread = touches.size === 2 ? spread() : null;
    },
  };
}
