// Scene frame projection: maps one authoritative snapshot to the renderer's camera and
// scene nodes. Presentation only: it reads the snapshot and never modifies it.

import * as THREE from "three";
import { presentedFacing } from "./character-layer.ts";
import { CAMERA_FOV_DEGREES, CAMERA_OFFSET } from "./aim-input.js";
import { FULL_EFFECTS } from "./presentation-effects.js";

// Monster head cues by published behaviour (#109). Body colours by behaviour, attack
// phase and stagger come from the presentation palette (presentation-effects.js).
const MONSTER_BEHAVIOR_CUES: Record<string, string> = {
  pursuing: "#ff7a59",
  retreating: "#d58bd0",
  holding: "#ff7a59",
  searching: "#f2c14e",
  returning: "#9a9aa8",
};

function segmentIntersectsRect(startX, startZ, endX, endZ, minX, maxX, minZ, maxZ) {
  let minimumT = 0;
  let maximumT = 1;
  for (const [start, end, minimum, maximum] of [
    [startX, endX, minX, maxX],
    [startZ, endZ, minZ, maxZ],
  ]) {
    const delta = end - start;
    if (Math.abs(delta) < 1e-9) {
      if (start < minimum || start > maximum) return false;
      continue;
    }
    const first = (minimum - start) / delta;
    const second = (maximum - start) / delta;
    const enter = Math.min(first, second);
    const exit = Math.max(first, second);
    minimumT = Math.max(minimumT, enter);
    maximumT = Math.min(maximumT, exit);
    if (minimumT > maximumT) return false;
  }
  return maximumT >= 0.03 && minimumT <= 0.97;
}

function barrierOccludesFocus(collider, target, scale) {
  if (!target || (collider.kind !== "wall" && collider.kind !== "door")) return false;
  const centerX = collider.position[0] / scale;
  const centerZ = collider.position[2] / scale;
  const halfX = collider.halfExtents[0] / scale + 0.35;
  const halfZ = collider.halfExtents[2] / scale + 0.35;
  const cameraX = target[0] + 10;
  const cameraZ = target[2] + 10;
  return segmentIntersectsRect(
    cameraX,
    cameraZ,
    target[0],
    target[2],
    centerX - halfX,
    centerX + halfX,
    centerZ - halfZ,
    centerZ + halfZ,
  );
}

function dungeonFloor(snapshot, scale) {
  if (!snapshot.rooms?.length) {
    return { center: [0, -0.08, 0], size: [18, 0.12, 13] };
  }
  const minX = Math.min(...snapshot.rooms.map((room) => room.minX)) / scale;
  const maxX = Math.max(...snapshot.rooms.map((room) => room.maxX)) / scale;
  const minZ = Math.min(...snapshot.rooms.map((room) => room.minZ)) / scale;
  const maxZ = Math.max(...snapshot.rooms.map((room) => room.maxZ)) / scale;
  return {
    center: [(minX + maxX) / 2, -0.08, (minZ + maxZ) / 2],
    size: [maxX - minX, 0.12, maxZ - minZ],
  };
}

// The primitive shield board of a raised or rising guard, `offset` in front of `position`
// along the unit direction it faces. Players and shield monsters share it.
function shieldBoard(id, position, [directionX, directionZ], offset, look, palette) {
  const yaw = Math.atan2(directionX, directionZ);
  return {
    id,
    geometry: { kind: "box", size: [0.5, 0.6, 0.08] },
    color:
      look === "blocked"
        ? palette.shieldBlocked
        : look === "raised"
          ? palette.shieldRaised
          : palette.shieldRising,
    transform: {
      translation: [
        position[0] + directionX * offset,
        Math.max(position[1], 0.5),
        position[2] + directionZ * offset,
      ],
      rotationQuaternion: [0, Math.sin(yaw / 2), 0, Math.cos(yaw / 2)],
    },
  };
}

// `effects` (presentation-effects.js) picks the combat palette and the stagger lift; nothing
// else in the frame depends on it.
export function buildFrame(
  snapshot,
  focusPlayerId,
  width,
  height,
  focusPlayerAccent = "#d6b45f",
  effects = FULL_EFFECTS,
) {
  const { palette } = effects;
  const scale = snapshot.worldUnitsPerMeter;
  const focus =
    snapshot.players.find((player) => player.id === focusPlayerId) ?? snapshot.players[0];
  const target = focus ? [focus.position[0] / scale, 0, focus.position[2] / scale] : [0, 0, 0];
  const aspect = Math.max(1, width) / Math.max(1, height);
  const near = 0.1;
  const far = 100;
  const fovYRadians = (CAMERA_FOV_DEGREES * Math.PI) / 180;
  const top = near * Math.tan(fovYRadians * 0.5);
  const projectionHeight = 2 * top;
  const projectionWidth = aspect * projectionHeight;
  const left = -0.5 * projectionWidth;
  const projectionMatrix = new THREE.Matrix4().makePerspective(
    left,
    left + projectionWidth,
    top,
    top - projectionHeight,
    near,
    far,
    THREE.WebGPUCoordinateSystem,
    false,
  );
  const camera = new THREE.PerspectiveCamera(CAMERA_FOV_DEGREES, aspect, near, far);
  camera.position.set(target[0] + CAMERA_OFFSET[0], CAMERA_OFFSET[1], target[2] + CAMERA_OFFSET[2]);
  camera.lookAt(target[0], 0, target[2]);
  camera.updateMatrixWorld(true);
  const floor = dungeonFloor(snapshot, scale);

  const nodes = [
    {
      id: "floor",
      geometry: { kind: "box", size: floor.size },
      color: "#292722",
      transform: { translation: floor.center },
    },
    ...snapshot.staticColliders.map((collider) => {
      const fullSize = collider.halfExtents.map((value) => (value * 2) / scale);
      const isBarrier = collider.kind === "wall" || collider.kind === "door";
      const lowered = isBarrier && barrierOccludesFocus(collider, target, scale);
      const visualHeight = lowered ? Math.min(fullSize[1], 0.55) : fullSize[1];
      const translation = collider.position.map((value) => value / scale);
      if (isBarrier) translation[1] = visualHeight / 2;
      return {
        id: `static-${collider.id}`,
        geometry: {
          kind: "box",
          size: [fullSize[0], visualHeight, fullSize[2]],
        },
        color: lowered ? "#62594e" : collider.kind === "pillar" ? "#5d5142" : "#403a33",
        transform: { translation },
      };
    }),
    ...snapshot.players.flatMap((player) => {
      const position = player.position.map((value) => value / scale);
      // The exact committed or intended direction, as the humanoid faces it.
      const facing = presentedFacing(player);
      const facingLength = Math.hypot(facing[0], facing[1]) || 1;
      const facingX = facing[0] / facingLength;
      const facingZ = facing[1] / facingLength;
      const actionColor =
        player.action?.phase === "windup"
          ? palette.playerWindup
          : player.action?.phase === "active"
            ? palette.playerActive
            : player.action?.phase === "recovery"
              ? palette.playerRecovery
              : null;
      const color = !player.alive
        ? "#45413d"
        : player.reaction?.kind === "hurt"
          ? palette.playerHurt
          : player.reaction?.kind === "guardBroken"
            ? palette.playerGuardBroken
            : (actionColor ??
              (player.id === focusPlayerId ? focusPlayerAccent : palette.playerPeer));
      // The thin shield board faces along the authoritative 8-way facing, which the core's
      // guard cone uses, not the exact aim shown by the weapon and marker.
      const guardFacing = player.facing ?? [1, 0];
      const guardLength = Math.hypot(guardFacing[0], guardFacing[1]) || 1;
      const guardX = guardFacing[0] / guardLength;
      const guardZ = guardFacing[1] / guardLength;
      const shield = player.guard
        ? [
            shieldBoard(
              `player-shield-${player.id}`,
              position,
              [guardX, guardZ],
              0.42,
              player.reaction?.kind === "blocked" ? "blocked" : player.guard.phase,
              palette,
            ),
          ]
        : [];
      const bow =
        player.weapon === "bow"
          ? [
              {
                id: `player-bow-${player.id}`,
                geometry: { kind: "box", size: [0.06, 0.7, 0.06] },
                color: player.drawTicks != null ? "#f5df9b" : "#9d8060",
                transform: {
                  translation: [
                    position[0] + facingX * (player.drawTicks != null ? 0.5 : 0.36),
                    Math.max(position[1], 0.5),
                    position[2] + facingZ * (player.drawTicks != null ? 0.5 : 0.36),
                  ],
                },
              },
            ]
          : [];
      return [
        ...shield,
        ...bow,
        // The humanoid itself is an imported character (#55, `character-layer.ts`); this flat
        // ground ring under it carries the action phase, hurt, guard-broken, dead and
        // focus-accent colours (owner decision A on #55).
        {
          id: `player-${player.id}`,
          geometry: { kind: "cylinder", radius: 0.42, height: 0.03 },
          color,
          transform: { translation: [position[0], position[1] + 0.015, position[2]] },
        },
      ];
    }),
    ...(snapshot.chests ?? []).map((chest) => ({
      id: `chest-${chest.id}`,
      geometry: { kind: "box", size: [0.6, 0.4, 0.4] },
      color: chest.opened ? "#3a3530" : chest.available ? "#d6b45f" : "#6b5236",
      transform: {
        translation: chest.position.map((value, axis) => (axis === 1 ? 0.2 : value / scale)),
      },
    })),
    ...(snapshot.arrows ?? []).map((arrow) => {
      const translation = arrow.position.map((value) => value / scale);
      const yaw = Math.atan2(arrow.velocity[0], arrow.velocity[2]);
      return {
        id: `arrow-${arrow.id}`,
        geometry: { kind: "box", size: [0.04, 0.04, 0.5] },
        // Player arrows and monster shots fly the same authoritative path (#108).
        color: arrow.source.kind === "monster" ? "#ff8a5c" : "#e8dcc0",
        transform: {
          translation: [translation[0], Math.max(translation[1], 0.55), translation[2]],
          rotationQuaternion: [0, Math.sin(yaw / 2), 0, Math.cos(yaw / 2)],
        },
      };
    }),
    ...snapshot.monsters
      .filter((monster) => monster.alive)
      .flatMap((monster) => {
        const translation = monster.position.map((value) => value / scale);
        if (monster.reaction?.kind === "stagger") translation[1] += effects.staggerLift;
        const phase = monster.action?.phase;
        const monsterColor =
          monster.reaction?.kind === "stagger"
            ? palette.monsterStagger
            : phase === "active"
              ? palette.monsterActive
              : phase === "windup"
                ? palette.monsterWindup
                : phase === "recovery"
                  ? palette.monsterRecovery
                  : (palette.monsterBehavior[monster.behavior] ?? palette.monsterDefault);
        const nodes = [
          {
            id: `monster-${monster.id}`,
            geometry: { kind: "sphere", radius: 0.42 },
            color: monsterColor,
            transform: { translation },
          },
        ];
        // A shield monster's authoritative guard faces its engaged target (#92).
        const guardTarget = snapshot.players.find(
          (candidate) => candidate.id === monster.targetPlayerId,
        );
        if (monster.guard && guardTarget) {
          const towardX = guardTarget.position[0] - monster.position[0];
          const towardZ = guardTarget.position[2] - monster.position[2];
          const length = Math.hypot(towardX, towardZ) || 1;
          nodes.push(
            shieldBoard(
              `monster-shield-${monster.id}`,
              translation,
              [towardX / length, towardZ / length],
              0.55,
              monster.guard.phase,
              palette,
            ),
          );
        }
        // The focused player's authoritative target lock rings its monster.
        if (focus?.lockedMonsterId === monster.id) {
          for (let index = 0; index < 8; index += 1) {
            const angle = (index / 8) * Math.PI * 2;
            nodes.push({
              id: `monster-${monster.id}-lock-${index}`,
              geometry: { kind: "sphere", radius: 0.06 },
              color: "#7fd1ff",
              transform: {
                translation: [
                  translation[0] + Math.cos(angle) * 0.62,
                  0.06,
                  translation[2] + Math.sin(angle) * 0.62,
                ],
              },
            });
          }
        }
        // The authoritative behaviour, not motion, picks the cue above the head.
        const cue = MONSTER_BEHAVIOR_CUES[monster.behavior];
        if (cue)
          nodes.push({
            id: `monster-${monster.id}-behavior`,
            geometry: { kind: "sphere", radius: 0.09 },
            color: cue,
            transform: {
              translation: [translation[0], translation[1] + 0.62, translation[2]],
            },
          });
        const targetPlayer = snapshot.players.find(
          (candidate) => candidate.id === monster.action?.targetPlayerId,
        );
        if (phase === "windup" && monster.action?.delivery === "projectile") {
          // A shot telegraphs its line of fire toward the target, not a ring.
          if (targetPlayer?.alive)
            for (let index = 1; index < 8; index += 1) {
              const along = index / 8;
              nodes.push({
                id: `monster-${monster.id}-aim-${index}`,
                geometry: { kind: "sphere", radius: 0.06 },
                color: "#c86a4a",
                transform: {
                  translation: [
                    (monster.position[0] +
                      (targetPlayer.position[0] - monster.position[0]) * along) /
                      scale,
                    0.075,
                    (monster.position[2] +
                      (targetPlayer.position[2] - monster.position[2]) * along) /
                      scale,
                  ],
                },
              });
            }
        } else if (
          (phase === "windup" || phase === "active") &&
          monster.action?.delivery === "strike" &&
          monster.action.range
        ) {
          const radius = monster.action.range / scale;
          const telegraphColor =
            phase === "active" ? palette.telegraphActive : palette.telegraphWindup;
          for (let index = 0; index < 12; index += 1) {
            const angle = (index / 12) * Math.PI * 2;
            nodes.push({
              id: `monster-${monster.id}-telegraph-${index}`,
              geometry: {
                kind: "sphere",
                radius: phase === "active" ? 0.1 : 0.075,
              },
              color: telegraphColor,
              transform: {
                translation: [
                  monster.position[0] / scale + Math.cos(angle) * radius,
                  0.075,
                  monster.position[2] / scale + Math.sin(angle) * radius,
                ],
              },
            });
          }
          if (targetPlayer?.alive) {
            nodes.push({
              id: `monster-${monster.id}-target`,
              geometry: { kind: "sphere", radius: 0.12 },
              color: telegraphColor,
              transform: {
                translation: [
                  targetPlayer.position[0] / scale,
                  0.14,
                  targetPlayer.position[2] / scale,
                ],
              },
            });
          }
        }
        return nodes;
      }),
    ...(snapshot.groundLoot ?? []).map((loot) => ({
      id: `loot-${loot.id}`,
      geometry: { kind: "sphere", radius: 0.16 },
      color: loot.kind === "gold" ? "#d9b44a" : "#c8c1b7",
      transform: {
        translation: [loot.position[0] / scale, 0.16, loot.position[2] / scale],
      },
    })),
  ];

  return {
    camera: {
      viewMatrix: camera.matrixWorldInverse.toArray(),
      projectionMatrix: projectionMatrix.toArray(),
    },
    nodes,
  };
}
