// Acceptance contract for the scene frame honouring presentation effects (#61, first slice).
//
// `buildFrame` moves unchanged out of `main.tsx` into `web/src/scene-frame.ts` (a structural
// commit of its own), so it can be tested without mounting the app. It gains one trailing
// parameter:
//
//   buildFrame(snapshot, focusPlayerId, width, height, accent = "#d6b45f",
//              effects = FULL_EFFECTS) => { camera, nodes }
//
// `effects` comes from `resolvePresentationEffects` (presentation-effects.js). Every colour
// for an action phase, reaction, guard look, monster behaviour and strike telegraph comes
// from `effects.palette`; a staggered monster (and the cue drawn above it) is lifted by
// `effects.staggerLift`. Nothing else in the frame (positions, camera, node ids, which nodes
// exist) depends on `effects`, and the authoritative snapshot is never modified.
import { expect, test } from "bun:test";
import { buildFrame } from "./scene-frame.ts";
import { FULL_EFFECTS, resolvePresentationEffects } from "./presentation-effects.js";

const reduced = resolvePresentationEffects(
  {
    graphics: { shadows: true, pixelRatioLimit: 2 },
    accessibility: { motion: "reduced", reducedFlashes: true, textScale: 1.5 },
  },
  { prefersReducedMotion: false },
);

const player = (id, extra = {}) => ({
  id,
  position: [id * 200, 0, 0],
  facing: [1, 0],
  alive: true,
  weapon: "swordAndShield",
  action: null,
  reaction: null,
  guard: null,
  lockedMonsterId: null,
  ...extra,
});

const monster = (id, extra = {}) => ({
  id,
  position: [id * 300, 0, 400],
  alive: true,
  behavior: "pursuing",
  action: null,
  reaction: null,
  guard: null,
  targetPlayerId: 1,
  ...extra,
});

function snapshot() {
  return {
    tick: 12,
    worldUnitsPerMeter: 100,
    rooms: [],
    staticColliders: [],
    chests: [],
    arrows: [],
    groundLoot: [],
    players: [
      player(1, { action: { kind: "primaryAttack", phase: "active", facing: [1, 0] } }),
      player(2, { guard: { phase: "raised" }, reaction: { kind: "blocked" } }),
      player(3, { action: { kind: "primaryAttack", phase: "windup", facing: [1, 0] } }),
      player(4, { reaction: { kind: "hurt" } }),
    ],
    monsters: [
      monster(1, { reaction: { kind: "stagger" } }),
      monster(2, {
        action: { phase: "active", delivery: "strike", range: 150, targetPlayerId: 1 },
      }),
      monster(3, {
        action: { phase: "windup", delivery: "strike", range: 150, targetPlayerId: 1 },
      }),
      monster(4, { behavior: "searching" }),
    ],
  };
}

const node = (frame, id) => {
  const found = frame.nodes.find((candidate) => candidate.id === id);
  if (!found) throw new Error(`missing node ${id}`);
  return found;
};

function looks(effects) {
  const frame = buildFrame(snapshot(), 1, 800, 600, "#d6b45f", effects);
  return {
    frame,
    activePlayer: node(frame, "player-1").color,
    blockedShield: node(frame, "player-shield-2").color,
    windupPlayer: node(frame, "player-3").color,
    hurtPlayer: node(frame, "player-4").color,
    staggerMonster: node(frame, "monster-1").color,
    staggerHeight: node(frame, "monster-1").transform.translation[1],
    activeMonster: node(frame, "monster-2").color,
    activeTelegraph: node(frame, "monster-2-telegraph-0").color,
    windupMonster: node(frame, "monster-3").color,
    windupTelegraph: node(frame, "monster-3-telegraph-0").color,
    searchingMonster: node(frame, "monster-4").color,
  };
}

test("default effects draw exactly what main draws today", () => {
  const implicit = buildFrame(snapshot(), 1, 800, 600, "#d6b45f");
  const explicit = looks(FULL_EFFECTS);
  expect(explicit.frame).toEqual(implicit);
  expect(explicit).toMatchObject({
    activePlayer: "#fff0a3",
    blockedShield: "#e8f4ff",
    windupPlayer: "#e3a45d",
    hurtPlayer: "#e05a4f",
    staggerMonster: "#e8a25d",
    staggerHeight: 0.08,
    activeMonster: "#ff6a4f",
    activeTelegraph: "#f06a4e",
    windupMonster: "#c85b46",
    windupTelegraph: "#824239",
    searchingMonster: "#8a6a3c",
  });
});

test("reduced effects draw from the reduced palette and without the stagger hop", () => {
  const palette = reduced.palette;
  expect(looks(reduced)).toMatchObject({
    activePlayer: palette.playerActive,
    blockedShield: palette.shieldBlocked,
    windupPlayer: palette.playerWindup,
    hurtPlayer: palette.playerHurt,
    staggerMonster: palette.monsterStagger,
    staggerHeight: 0,
    activeMonster: palette.monsterActive,
    activeTelegraph: palette.telegraphActive,
    windupMonster: palette.monsterWindup,
    windupTelegraph: palette.telegraphWindup,
    searchingMonster: palette.monsterBehavior.searching,
  });
  expect(looks(reduced).activePlayer).not.toBe("#fff0a3");
});

test("effects change only colours and the stagger lift, never what or where things are", () => {
  // The staggered monster (monster 1) and the nodes drawn relative to it (its behaviour cue)
  // may differ only in height.
  const lifted = (id) => id === "monster-1" || id.startsWith("monster-1-");
  const strip = (frame) =>
    frame.nodes.map(({ id, geometry, transform }) => ({
      id,
      geometry,
      translation: lifted(id)
        ? [transform.translation[0], transform.translation[2]]
        : transform.translation,
      rotation: transform.rotationQuaternion ?? null,
    }));
  const full = buildFrame(snapshot(), 1, 800, 600, "#d6b45f", FULL_EFFECTS);
  const less = buildFrame(snapshot(), 1, 800, 600, "#d6b45f", reduced);
  expect(less.camera).toEqual(full.camera);
  expect(strip(less)).toEqual(strip(full));
});

test("building a frame never modifies the authoritative snapshot", () => {
  const source = snapshot();
  const before = JSON.stringify(source);
  buildFrame(source, 1, 800, 600, "#d6b45f", reduced);
  buildFrame(source, 1, 800, 600, "#d6b45f", FULL_EFFECTS);
  expect(JSON.stringify(source)).toBe(before);
});
