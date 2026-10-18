import * as THREE from "three";

// The follow camera: offset from the focused player (metres) and vertical field of view.
// buildFrame renders with the same values, so a pointer maps onto the ground it shows.
export const CAMERA_OFFSET = [10, 11, 10];
export const CAMERA_FOV_DEGREES = 43;
// Aim intent sent to the authority uses components within ±1000 (AIM_COMPONENT_LIMIT).
const AIM_SCALE = 1000;
// Aim is quantised to 2° steps so pointer jitter does not flood the command channel.
const AIM_STEP_RADIANS = (2 * Math.PI) / 180;
// Pointers this close to the player (metres on the ground) have no meaningful direction.
const AIM_DEAD_ZONE = 0.2;

const camera = new THREE.PerspectiveCamera(CAMERA_FOV_DEGREES, 1, 0.1, 100);
const raycaster = new THREE.Raycaster();
const ground = new THREE.Plane(new THREE.Vector3(0, 1, 0), 0);
const pointer = new THREE.Vector2();
const hit = new THREE.Vector3();

// Maps a pointer position inside the game canvas to the semantic aim direction from the
// focused player (who sits where the camera looks) to the ground under the pointer.
// Returns null inside the dead zone or when the pointer ray misses the ground.
export function aimFromPointer(pointerX, pointerY, width, height) {
  if (!(width > 0 && height > 0)) return null;
  camera.aspect = width / height;
  camera.position.set(...CAMERA_OFFSET);
  camera.lookAt(0, 0, 0);
  camera.updateProjectionMatrix();
  camera.updateMatrixWorld(true);
  pointer.set((pointerX / width) * 2 - 1, 1 - (pointerY / height) * 2);
  raycaster.setFromCamera(pointer, camera);
  if (!raycaster.ray.intersectPlane(ground, hit)) return null;
  if (Math.hypot(hit.x, hit.z) < AIM_DEAD_ZONE) return null;
  const angle = Math.round(Math.atan2(hit.z, hit.x) / AIM_STEP_RADIANS) * AIM_STEP_RADIANS;
  // `+ 0` folds negative zero, so equal directions compare equal.
  return [Math.round(Math.cos(angle) * AIM_SCALE) + 0, Math.round(Math.sin(angle) * AIM_SCALE) + 0];
}

// The core accepts only eight-way {-1,0,1} movement, not the continuous aim vector.
// Match arpg-core::facing_for's sector boundaries so right-click locomotion takes the
// normal, Rust-authoritative movement path instead of inventing another resolver.
export function movementFromAim(direction) {
   if (!direction) return [0, 0];
   const [x, z] = direction;
   const axis = (own, other) =>
     Math.abs(own) * 100_000 < Math.abs(other) * 41_421 ? 0 : Math.sign(own);
   return [axis(x, z), axis(z, x)];
}
