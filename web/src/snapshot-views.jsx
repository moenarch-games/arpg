import { useEffect, useState, useSyncExternalStore } from "react";
import { useGestureButton, useHeldButton, usePressButton } from "./touch-buttons.js";

export function useSnapshot(store) {
  return useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
}

function focusedPlayer(snapshot, playerId) {
  return snapshot?.players.find((player) => player.id === playerId) ?? null;
}

// The label of the key an action is bound to, or a marker for an unbound action.
const UNBOUND = "unbound";
const keyOf = (keyLabels, action) => keyLabels?.[action] ?? UNBOUND;

// The movement keys as one hint: `WASD` for single keys, joined otherwise.
function movementKeys(keyLabels) {
  const keys = ["game.moveForward", "game.moveLeft", "game.moveBackward", "game.moveRight"].map(
    (action) => keyOf(keyLabels, action),
  );
  return keys.every((key) => key.length === 1) ? keys.join("") : keys.join("/");
}

export function PlayerHud({ store, playerId, status, interactKey = "E", keyLabels }) {
  const snapshot = useSnapshot(store);
  const player = focusedPlayer(snapshot, playerId);
  const healthPercent = player?.maxHealth
    ? Math.max(0, Math.min(100, (player.health / player.maxHealth) * 100))
    : 0;
  return (
    <section className="hud" aria-label="Player status">
      <div className="health">
        <span style={{ width: `${healthPercent}%` }} />
      </div>
      {player && (
        <div className="progression" aria-label="Character progression">
          <strong>Level {player.level}</strong>
          <span>
            XP {player.experienceIntoLevel}/{player.experienceForNextLevel}
          </span>
          <span>Damage {player.attackDamage}</span>
          <span>Gold {player.gold}</span>
          <span>
            Guard {player.guardPoints}/{player.maxGuardPoints}
          </span>
          <span>{player.weapon === "bow" ? "Bow (unlimited arrows)" : "Sword & shield"}</span>
          {player.drawTicks != null && <span>Draw {player.drawTicks}t</span>}
          {player.lockedMonsterId != null && <span>Target locked</span>}
        </div>
      )}
      <p>{status}</p>
      {player && !player.alive && <p className="defeated-status">Defeated</p>}
      {player?.alive && interactionPromptText(player.interaction, interactKey) && (
        <p className="interaction-prompt">
          {interactionPromptText(player.interaction, interactKey)}
        </p>
      )}
      {player?.alive && (player.guard || player.reaction?.kind === "guardBroken") && (
        <p className="guard-status" aria-live="polite">
          {player.reaction?.kind === "guardBroken"
            ? "Guard broken"
            : player.reaction?.kind === "blocked"
              ? "Blocked"
              : player.guard.phase === "raised"
                ? "Shield raised"
                : "Raising shield"}
        </p>
      )}
      {player?.alive && player.counter && snapshot.tick < player.counter.expiresAtTick && (
        <p className="counter-prompt">
          {/* Announced once when the opportunity opens; the per-tick countdown is visual only. */}
          <span role="alert">Counter!</span>{" "}
          <span aria-hidden="true">· {player.counter.expiresAtTick - snapshot.tick}t</span>
        </p>
      )}
      {player?.action && (
        <p className="action-status">
          {player.action.kind === "secondaryAttack"
            ? "Heavy"
            : player.action.kind === "interact"
              ? "Interact"
              : player.action.kind === "counter"
                ? "Counter"
                : player.action.kind === "lightFollowUp"
                  ? "Light 2"
                  : player.action.kind === "lightFinisher"
                    ? "Light finisher"
                    : player.action.kind === "heavyFinisher"
                      ? "Heavy finisher"
                      : "Primary"}{" "}
          · {player.action.phase} · {player.action.ticksRemaining}t
        </p>
      )}
      <p className="desktop-controls-hint">
        {movementKeys(keyLabels)} move · {keyOf(keyLabels, "game.jump")} jump ·{" "}
        {keyOf(keyLabels, "game.primaryAttack")} primary / hold to draw bow ·{" "}
        {keyOf(keyLabels, "game.secondaryAttack")} heavy · {keyOf(keyLabels, "game.guard")} hold
        shield · {keyOf(keyLabels, "game.switchWeapon")} switch bow ·{" "}
        {keyOf(keyLabels, "game.interact")} interact · {keyOf(keyLabels, "game.settings")} settings
      </p>
      <p className="mobile-controls-hint">
        {player?.weapon === "bow"
          ? "Left stick · Tap Jump · Hold Draw, release to shoot · Guard and Heavy unavailable with bow"
          : "Left stick · Tap Jump · Tap Attack for light, swipe ↑ Heavy, → target, ← unlock · Hold Guard · Tap Interact"}
      </p>
    </section>
  );
}

const LIGHT_KINDS = new Set(["primaryAttack", "counter", "lightFollowUp", "lightFinisher"]);
const HEAVY_KINDS = new Set(["secondaryAttack", "heavyFinisher"]);

// Input-bindings owns these semantic action IDs; this button maps only local
// gestures to them. Nothing here decides combat, targeting or interaction outcomes.
const SWORD_GESTURE_ACTIONS = {
  tap: "game.primaryAttack",
  up: "game.secondaryAttack",
  right: "game.cycleTarget",
  left: "game.clearTarget",
};

export function CombatActions({
  store,
  playerId,
  triggerCombatAction,
  gestureEpochRef,
  setTouchGuard,
  setTouchDraw,
  switchWeapon,
  keyLabels,
}) {
  const player = focusedPlayer(useSnapshot(store), playerId);
  const light = LIGHT_KINDS.has(player?.action?.kind);
  const heavy = HEAVY_KINDS.has(player?.action?.kind);
  // Attack buttons stay usable during attacks: the authority decides whether a press
  // continues a combo, is buffered or is ignored.
  const canAttack = Boolean(playerId && player?.alive);
  const bow = player?.weapon === "bow";
  // Each touch owns its action even when the other thumb is moving the joystick.
  const primaryHandlers = useGestureButton(
    (gesture) => {
      const action = SWORD_GESTURE_ACTIONS[gesture];
      if (action) triggerCombatAction(action);
    },
    canAttack && !bow,
    gestureEpochRef,
  );
  const heavyHandlers = usePressButton(() => triggerCombatAction("game.secondaryAttack"));
  const swapHandlers = usePressButton(switchWeapon);
  const interactHandlers = usePressButton(() => triggerCombatAction("game.interact"));
  // Jump is a press like the keyboard's: the authority decides whether it takes off.
  const jumpHandlers = usePressButton(() => triggerCombatAction("game.jump"));
  const bowHandlers = useHeldButton(
    (held, interrupted) => setTouchDraw(held, interrupted),
    canAttack && bow,
  );
  const guardHandlers = useHeldButton((held) => setTouchGuard(held), canAttack && !bow);
  // With the bow, Attack is a held draw: press draws, release shoots.
  const drawHandlers = bow ? bowHandlers : primaryHandlers;
  return (
    <section className="combat-actions" aria-label="Combat actions">
      <button
        type="button"
        className={`combat-action combat-action-primary ${light ? "is-committed" : ""} ${
          player?.counter ? "is-counter-ready" : ""
        }`}
        data-phase={light ? player.action.phase : undefined}
        data-counter={player?.counter ? "ready" : undefined}
        aria-label={bow ? "Draw bow" : "Primary attack"}
        data-draw={player?.drawTicks != null ? "drawing" : undefined}
        disabled={!canAttack}
        {...drawHandlers}
      >
        <strong>{bow ? "Draw" : player?.counter ? "Counter" : "Attack"}</strong>
        <span>{keyOf(keyLabels, "game.primaryAttack")}</span>
      </button>
      <button
        type="button"
        className={`combat-action combat-action-secondary ${heavy ? "is-committed" : ""}`}
        data-phase={heavy ? player.action.phase : undefined}
        aria-label="Heavy attack"
        disabled={!canAttack || bow}
        {...heavyHandlers}
      >
        <strong>Heavy</strong>
        <span>{keyOf(keyLabels, "game.secondaryAttack")}</span>
      </button>
      <button
        type="button"
        className={`combat-action combat-action-guard ${player?.guard ? "is-committed" : ""}`}
        data-guard={player?.guard?.phase}
        aria-label="Hold shield"
        disabled={!playerId || !player?.alive || bow}
        {...guardHandlers}
      >
        <strong>Guard</strong>
        <span>{keyOf(keyLabels, "game.guard")}</span>
      </button>
      <button
        type="button"
        className="combat-action combat-action-swap"
        aria-label={bow ? "Switch to sword and shield" : "Switch to bow"}
        disabled={!canAttack || Boolean(player?.action)}
        {...swapHandlers}
      >
        <strong>{bow ? "Sword" : "Bow"}</strong>
        <span>{keyOf(keyLabels, "game.switchWeapon")}</span>
      </button>
      <button
        type="button"
        className={`combat-action combat-action-interact ${
          player?.action?.kind === "interact" ? "is-committed" : ""
        }`}
        data-phase={player?.action?.kind === "interact" ? player.action.phase : undefined}
        aria-label="Interact or pick up"
        disabled={!playerId || !player?.alive || Boolean(player?.action)}
        {...interactHandlers}
      >
        <strong>Interact</strong>
        <span>{keyOf(keyLabels, "game.interact")}</span>
      </button>
      <button
        type="button"
        className={`combat-action combat-action-jump ${
          player && !player.motion?.grounded ? "is-airborne" : ""
        }`}
        data-motion={player ? (player.motion?.grounded ? "grounded" : "airborne") : undefined}
        aria-label="Jump"
        disabled={!canAttack}
        {...jumpHandlers}
      >
        <strong>Jump</strong>
        <span>{keyOf(keyLabels, "game.jump")}</span>
      </button>
    </section>
  );
}

export function TrainingTick({ store }) {
  const snapshot = useSnapshot(store);
  return <span>Tick {snapshot?.tick ?? 0}</span>;
}

const TIMELINE_LIMIT = 8;

// Prompts come from the authority's own interaction choice, never a browser radius.
function interactionPromptText(interaction, key) {
  if (interaction?.kind === "available") {
    const what = interaction.target.kind === "chest" ? "Open chest" : "Pick up gold";
    return key ? `${key} · ${what}` : `${what} (Interact is unbound)`;
  }
  if (interaction?.reason === "chestLocked") return "Chest locked until the room is cleared";
  if (interaction?.reason === "obstructed") return "Out of reach behind a wall";
  return null;
}

function describeInteraction(event) {
  const { result } = event;
  if (result.kind === "refused")
    return `player ${event.playerId} · interact refused · ${result.reason}`;
  const verb = result.kind === "opened" ? "opened" : "picked up";
  return `player ${event.playerId} · ${verb} ${result.target.kind} ${result.target.id} · +${result.gold} gold`;
}

function describeParty(party) {
  return `${party.kind} ${party.id}`;
}

function describeResult(result) {
  if (result.kind === "hit") return `hit ${result.damage}${result.defeated ? " · defeated" : ""}`;
  if (result.kind === "blocked") return `blocked · guard −${result.guardDamage}`;
  if (result.kind === "guardBroken") return "guard broken";
  return result.kind;
}

// Bounded, tick-stamped log of authoritative strike outcomes. It is evidence from the
// simulation, not a renderer animation; restarting the simulation clears it.
export function StrikeTimeline({ store }) {
  const [entries, setEntries] = useState([]);
  useEffect(() => {
    let lastTick = -1;
    return store.subscribe(() => {
      const snapshot = store.getSnapshot();
      if (!snapshot) return;
      const restarted = snapshot.tick < lastTick;
      lastTick = snapshot.tick;
      // The authority numbers every event of a tick; show them in that order.
      const lines = [
        ...(snapshot.strikeEvents ?? []).map((event) => ({
          order: event.order,
          text: `${event.definition} · ${describeParty(event.source)} → ${describeParty(
            event.target,
          )} · ${describeResult(event.result)}`,
        })),
        ...(snapshot.interactionEvents ?? []).map((event) => ({
          order: event.order,
          text: describeInteraction(event),
        })),
      ]
        .sort((left, right) => left.order - right.order)
        .map((line) => line.text);
      if (!restarted && lines.length === 0) return;
      setEntries((current) =>
        [
          ...lines.map((line, index) => ({
            key: `${snapshot.tick}-${index}`,
            text: `tick ${snapshot.tick} · ${line}`,
          })),
          ...(restarted ? [] : current),
        ].slice(0, TIMELINE_LIMIT),
      );
    });
  }, [store]);
  return (
    <section className="strike-timeline" aria-label="Event timeline">
      <h2>Events</h2>
      {entries.length === 0 ? (
        <p>No events yet</p>
      ) : (
        <ol>
          {entries.map((entry) => (
            <li key={entry.key}>{entry.text}</li>
          ))}
        </ol>
      )}
    </section>
  );
}

export function TrainingDiagnostics({ store, playerId }) {
  const snapshot = useSnapshot(store);
  const player = focusedPlayer(snapshot, playerId);
  const aliveMonsterCount = snapshot?.monsters.filter((monster) => monster.alive).length ?? 0;
  const actingMonsterCount =
    snapshot?.monsters.filter((monster) => monster.alive && monster.action).length ?? 0;
  const pursuingMonsterCount =
    snapshot?.monsters.filter((monster) => monster.behavior === "pursuing").length ?? 0;
  const retreatingMonsterCount =
    snapshot?.monsters.filter((monster) => monster.behavior === "retreating").length ?? 0;
  const playerActionLabel = player?.action
    ? [
        player.action.kind,
        player.action.phase,
        `${player.action.ticksRemaining}t`,
        player.action.connected ? "hit" : null,
        player.action.buffered ? `queued ${player.action.buffered}` : null,
      ]
        .filter(Boolean)
        .join(" · ")
    : "idle";
  return (
    <dl className="training-diagnostics">
      <div>
        <dt>Monsters</dt>
        <dd>
          {aliveMonsterCount}/{snapshot?.monsters.length ?? 0}
        </dd>
      </div>
      <div>
        <dt>Enemy actions</dt>
        <dd>{actingMonsterCount}</dd>
      </div>
      <div>
        <dt>Pursuing</dt>
        <dd>{pursuingMonsterCount}</dd>
      </div>
      <div>
        <dt>Retreating</dt>
        <dd>{retreatingMonsterCount}</dd>
      </div>
      <div>
        <dt>Player action</dt>
        <dd>{playerActionLabel}</dd>
      </div>
      <div>
        <dt>Arrows</dt>
        <dd>{snapshot?.arrows?.length ?? 0}</dd>
      </div>
      <div>
        <dt>Aim</dt>
        <dd>{player?.aim ? `aim ${player.aim[0]}, ${player.aim[1]}` : "facing"}</dd>
      </div>
      <div>
        <dt>Feet</dt>
        <dd>{player?.motion ? `feet ${player.motion.feetY}` : "none"}</dd>
      </div>
      <div>
        <dt>Poison</dt>
        <dd>{player?.poison ? `poison ${player.poison.ticksRemaining}` : "none"}</dd>
      </div>
      <div>
        <dt>Region</dt>
        <dd>
          {player?.region?.kind === "transit"
            ? `transit ${player.region.connection}`
            : player?.region
              ? `region ${player.region.id}`
              : "none"}
        </dd>
      </div>
      <div>
        <dt>Target</dt>
        <dd>
          {player?.lockedMonsterId != null ? `locked monster ${player.lockedMonsterId}` : "none"}
        </dd>
      </div>
    </dl>
  );
}

export function GameSummary({ store, mode, playerId }) {
  const snapshot = useSnapshot(store);
  return (
    <p>
      Current mode: <strong>{mode}</strong>
      {playerId ? ` · player ${playerId}` : ""}
      {snapshot ? ` · run seed ${snapshot.runSeed}` : ""}
    </p>
  );
}
