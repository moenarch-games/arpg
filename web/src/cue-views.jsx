// HUD views of presentation cues (#67, 67a): a short feedback feed and floating damage
// numbers. Both only describe cues the authority already resolved; nothing here computes
// damage, hits or windows.

import * as THREE from "three";
import { useEffect, useRef, useState, useSyncExternalStore } from "react";
import { poseCamera } from "./follow-camera.js";
import { t } from "./messages.ts";
import { monsterName, useSnapshot } from "./snapshot-views.jsx";

export function usePresentationCues(cues) {
  return useSyncExternalStore(cues.subscribe, cues.getCues, cues.getCues);
}

const ENCOUNTER_KEYS = {
  active: "cue.encounter.active",
  cleared: "cue.encounter.cleared",
  dormant: "cue.encounter.dormant",
};

const isPlayer = (party, playerId) => party?.kind === "player" && party.id === playerId;

// The feed line one cue reads as for `playerId`, or null when it does not concern them.
export function cueText(cue, playerId, snapshot) {
  const nameOf = (party) =>
    party?.kind === "monster"
      ? monsterName(snapshot?.monsters.find((monster) => monster.id === party.id)?.definition)
      : t("monster.name.other");
  const mine = isPlayer(cue.source, playerId) || cue.playerId === playerId;
  const atMe = isPlayer(cue.target, playerId);
  switch (cue.kind) {
    case "hit":
      if (atMe) return t("cue.hurt");
      return mine && !cue.defeated ? t("cue.hit", { name: nameOf(cue.target) }) : null;
    case "blocked":
      if (atMe) return t("cue.youBlocked");
      return mine ? t("cue.yourStrikeBlocked", { name: nameOf(cue.target) }) : null;
    case "guardBroken":
      if (atMe) return t("cue.yourGuardBroken");
      return mine ? t("cue.enemyGuardBroken", { name: nameOf(cue.target) }) : null;
    case "obstructed":
      return mine ? t("cue.obstructed") : null;
    case "whiff":
      return mine ? t("cue.whiff") : null;
    case "defeat":
      if (cue.target.kind === "monster")
        return t("cue.defeated", { name: monsterName(cue.definition) });
      return atMe ? null : t("cue.allyDown", { player: String(cue.target.id) });
    case "pickedUp":
      return mine ? t("cue.pickedUp", { gold: cue.gold }) : null;
    case "opened":
      return mine ? t("cue.opened", { gold: cue.gold }) : null;
    case "arrowRelease":
      return mine ? t("cue.arrowRelease") : null;
    case "arrowLanded":
      return mine ? t("cue.arrowLanded") : null;
    case "counterUsed":
      return mine ? t("cue.counterUsed") : null;
    case "counterExpired":
      return mine ? t("cue.counterExpired") : null;
    case "counterLost":
      return mine ? t("cue.counterLost") : null;
    case "encounter":
      return ENCOUNTER_KEYS[cue.state] ? t(ENCOUNTER_KEYS[cue.state]) : null;
    default:
      return null;
  }
}

const FEED_LINES = 4;

export function CueFeed({ store, cues, playerId }) {
  const snapshot = useSnapshot(store);
  const recent = usePresentationCues(cues);
  const lines = [];
  for (let index = recent.length - 1; index >= 0 && lines.length < FEED_LINES; index -= 1) {
    const text = cueText(recent[index], playerId, snapshot);
    if (text) lines.push({ key: recent[index].key, kind: recent[index].kind, text });
  }
  return (
    <ol className="cue-feed" aria-label={t("cue.region")} aria-live="polite">
      {lines.reverse().map((line) => (
        <li key={line.key} data-cue={line.kind}>
          {line.text}
        </li>
      ))}
    </ol>
  );
}

// Metres above an entity's position at which its damage number starts.
const NUMBER_LIFT = 1.9;
const projector = new THREE.PerspectiveCamera();
const point = new THREE.Vector3();

// Floating numbers for authoritative hit damage dealt by or to `playerId`, anchored where
// the target stood when the hit resolved. Blocks, whiffs and obstructions never get one.
export function DamageNumbers({ store, cues, playerId, camera }) {
  const snapshot = useSnapshot(store);
  const recent = usePresentationCues(cues);
  const overlayRef = useRef(null);
  const [size, setSize] = useState({ width: 0, height: 0 });
  useEffect(() => {
    const overlay = overlayRef.current;
    if (!overlay || typeof ResizeObserver === "undefined") return undefined;
    const measure = () => {
      const rect = overlay.getBoundingClientRect();
      setSize({ width: rect.width, height: rect.height });
    };
    measure();
    const observer = new ResizeObserver(measure);
    observer.observe(overlay);
    return () => observer.disconnect();
  }, []);

  const scale = snapshot?.worldUnitsPerMeter ?? 0;
  const ready = scale > 0 && size.width > 0 && size.height > 0;
  if (ready) poseCamera(camera.pose(), size.width / size.height, projector);
  const numbers = ready
    ? recent.filter(
        (cue) =>
          cue.kind === "hit" &&
          cue.damage > 0 &&
          cue.position &&
          (isPlayer(cue.source, playerId) || isPlayer(cue.target, playerId)),
      )
    : [];
  return (
    <div ref={overlayRef} className="damage-numbers" aria-label={t("cue.damageNumbers")}>
      {numbers.map((cue) => {
        point
          .set(
            cue.position[0] / scale,
            cue.position[1] / scale + NUMBER_LIFT,
            cue.position[2] / scale,
          )
          .project(projector);
        const toMe = isPlayer(cue.target, playerId);
        return (
          <span
            key={cue.key}
            className="damage-number"
            data-target={toMe ? "self" : "enemy"}
            style={{
              left: `${((point.x + 1) / 2) * 100}%`,
              top: `${((1 - point.y) / 2) * 100}%`,
            }}
          >
            {/* Damage taken reads as a loss in text, not only in colour. */}
            {toMe ? `−${cue.damage}` : String(cue.damage)}
          </span>
        );
      })}
    </div>
  );
}
