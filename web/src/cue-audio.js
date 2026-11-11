// Combat and world sound from presentation cues (#67, 67c; ADR 0004). The shared
// `game-audio` director decides voices, buses, cooldowns, distance fade and pan; this module
// only maps 67a cues onto the sound manifest's kinds and feeds it entity positions and the
// listener pose.
//
// - Cues are cosmetic, and so is sound: a missing manifest, blocked or absent Web Audio, or
//   a failed decode stays silent and changes nothing else.
// - Nothing sounds until the browser lets the AudioContext run (first gesture); cues before
//   that are skipped, never queued, so unlocking never plays a backlog.
// - Each cue sounds once: its 67a key, scoped to the cue stream's current epoch, is the
//   director's dedupe key, so a replayed or late snapshot never replays a sound, while a
//   restarted run's identical ticks still sound.

import { createGameAudio, wrapGameAudio } from "@moritzbrantner/game-audio";

// The game-audio handle for the staged sound bundle under `baseUrl`. A missing or unreadable
// manifest, absent Web Audio or a failed wasm load yields the silent "unavailable" handle.
export async function loadCueAudio({ baseUrl, fetchImpl = fetch, onError = console.warn }) {
  try {
    const response = await fetchImpl(`${baseUrl}sound-manifest.json`);
    if (!response.ok) throw new Error(`Sound manifest: ${response.status}`);
    return await createGameAudio({ manifest: await response.text(), baseUrl, onError });
  } catch (error) {
    onError(error);
    return wrapGameAudio(null);
  }
}

// Player and monster ids are separate number spaces; the director's entity ids are one
// space of non-negative safe integers, so the kind is folded into the low bit.
export function audioEntity(party) {
  if (!party || !Number.isSafeInteger(party.id) || party.id < 0) return null;
  const id = party.id * 2;
  if (!Number.isSafeInteger(id + 1)) return null;
  if (party.kind === "player") return id;
  if (party.kind === "monster") return id + 1;
  return null;
}

const ENCOUNTER_SOUNDS = { active: "encounter.start", cleared: "encounter.clear" };
const BOW_ARROW = "bow.arrow";

const isPlayer = (party, playerId) => party?.kind === "player" && party.id === playerId;

// The manifest cue kind for `cue` as heard by `playerId`, or null for a silent cue. Cues
// about the local player alone (hurt, defeat, counter available, refused) are only heard by
// that player; world sounds are heard from wherever they happen.
export function soundKind(cue, playerId) {
  const mine = isPlayer(cue.source, playerId) || cue.playerId === playerId;
  switch (cue.kind) {
    case "swing":
      return "combat.swing";
    case "hit":
      if (isPlayer(cue.target, playerId)) return "combat.hurt";
      return cue.defeated ? "combat.hit.defeated" : "combat.hit";
    case "blocked":
      return cue.definition === BOW_ARROW ? "combat.arrowBlocked" : "combat.blocked";
    case "guardBroken":
      return "combat.guardBroken";
    case "obstructed":
      return "combat.obstructed";
    case "whiff":
      return "combat.whiff";
    case "defeat":
      return isPlayer(cue.target, playerId) ? "combat.defeat" : null;
    case "pickedUp":
      return cue.gold > 0 ? "world.pickedUp.gold" : null;
    case "opened":
      return "world.opened";
    case "refused":
      return mine ? "world.refused" : null;
    case "arrowRelease":
      return "combat.arrowRelease";
    case "arrowLanded":
      return "combat.arrowLanded";
    case "counterOpen":
      return mine ? "combat.counterOpen" : null;
    case "counterUsed":
      return "combat.counterUsed";
    case "encounter":
      return ENCOUNTER_SOUNDS[cue.state] ?? null;
    default:
      return null;
  }
}

// The party a cue's sound belongs to: the struck target, else the acting player.
function cueParty(cue) {
  if (cue.target) return cue.target;
  if (cue.source) return cue.source;
  if (cue.playerId != null) return { kind: "player", id: cue.playerId };
  return null;
}

function partyPosition(snapshot, party) {
  const list =
    party?.kind === "player"
      ? snapshot.players
      : party?.kind === "monster"
        ? snapshot.monsters
        : null;
  return list?.find((entity) => entity.id === party.id)?.position ?? null;
}

const metres = (position, scale) =>
  position && scale > 0 ? [position[0] / scale, position[1] / scale, position[2] / scale] : null;

// The director event for `cue`, or null for a silent cue. Positions are in metres.
export function cueSound(cue, { playerId, snapshot, epoch = 0 }) {
  const kind = soundKind(cue, playerId);
  if (!kind) return null;
  // An arrow that hit the world sounds where it landed, not at its archer.
  const party = cue.kind === "arrowLanded" ? null : cueParty(cue);
  const position = metres(
    cue.position ?? partyPosition(snapshot, party),
    snapshot.worldUnitsPerMeter,
  );
  return { key: `${epoch}:${cue.key}`, kind, entity: audioEntity(party), position };
}

// The listener sits on the ground point the follow camera looks at, with its right ear along
// the camera's screen-right, so pan matches the picture.
export function listenerPose(pose) {
  const forward = pose.target.map((value, axis) => value - pose.position[axis]);
  // forward × up, with up = +y.
  const right = [-forward[2], 0, forward[0]];
  const length = Math.hypot(right[0], right[2]);
  return {
    position: pose.target,
    right: length > 0 ? [right[0] / length, 0, right[2] / length] : [1, 0, 0],
  };
}

// Feeds `cues` (a presentation-cue stream) into `audio` (a game-audio handle). `frame(pose)`
// moves the listener and frees finished voices; call it once per drawn frame.
export function attachCueAudio({ cues, audio, getPlayerId }) {
  let epoch = 0;
  let present = new Set();
  const unlisten = cues.listen({
    reset() {
      epoch += 1;
      present = new Set();
      audio.reset();
    },
    cues(emitted, snapshot) {
      const scale = snapshot.worldUnitsPerMeter;
      const next = new Set();
      for (const [kind, list] of [
        ["player", snapshot.players],
        ["monster", snapshot.monsters],
      ]) {
        for (const entity of list) {
          const id = audioEntity({ kind, id: entity.id });
          const position = metres(entity.position, scale);
          if (id === null || !position) continue;
          next.add(id);
          audio.setEntityPosition(id, position);
        }
      }
      for (const id of present) if (!next.has(id)) audio.stopEntity(id);
      present = next;
      if (emitted.length === 0 || audio.state() !== "running") return;
      const playerId = getPlayerId();
      for (const cue of emitted) {
        const sound = cueSound(cue, { playerId, snapshot, epoch });
        if (sound) audio.trigger(sound);
      }
    },
  });
  return {
    frame(pose) {
      const listener = listenerPose(pose);
      audio.setListener(listener.position, listener.right);
      audio.pump();
    },
    dispose() {
      unlisten();
      audio.reset();
    },
  };
}
