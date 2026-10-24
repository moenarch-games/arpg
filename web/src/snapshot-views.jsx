import { useEffect, useState, useSyncExternalStore } from "react";
import { useGestureButton, useHeldButton, usePressButton } from "./touch-buttons.js";
import { t } from "./messages.ts";

export function useSnapshot(store) {
  return useSyncExternalStore(store.subscribe, store.getSnapshot, store.getSnapshot);
}

function focusedPlayer(snapshot, playerId) {
  return snapshot?.players.find((player) => player.id === playerId) ?? null;
}

// The label of the key an action is bound to, or a marker for an unbound action.
const keyOf = (keyLabels, action) => keyLabels?.[action] ?? t("hud.unbound");

// The movement keys as one hint: `WASD` for single keys, joined otherwise.
function movementKeys(keyLabels) {
  const keys = ["game.moveForward", "game.moveLeft", "game.moveBackward", "game.moveRight"].map(
    (action) => keyOf(keyLabels, action),
  );
  return keys.every((key) => key.length === 1) ? keys.join("") : keys.join("/");
}

// The most urgent monster attack telegraphed at `playerId` (an active one before a
// wind-up), so the HUD names the phase in text as well as the ring's colour and shape (#61).
const THREAT_LABELS = {
  strike: { windup: "threat.strike.windup", active: "threat.strike.active" },
  burst: { windup: "threat.burst.windup", active: "threat.burst.active" },
  projectile: { windup: "threat.shot.windup", active: "threat.shot.active" },
};
export function incomingThreat(snapshot, playerId) {
  let threat = null;
  for (const monster of snapshot?.monsters ?? []) {
    const action = monster.alive ? monster.action : null;
    if (action?.targetPlayerId !== playerId || !THREAT_LABELS[action.delivery]) continue;
    if (action.phase !== "windup" && action.phase !== "active") continue;
    if (!threat || (action.phase === "active" && threat.phase !== "active"))
      threat = { delivery: action.delivery, phase: action.phase };
  }
  return threat;
}

export function incomingThreatText(threat) {
  return t(THREAT_LABELS[threat.delivery][threat.phase === "active" ? "active" : "windup"]);
}

export function PlayerHud({ store, playerId, status, interactKey = "E", keyLabels }) {
  const snapshot = useSnapshot(store);
  const player = focusedPlayer(snapshot, playerId);
  const threat = player?.alive ? incomingThreat(snapshot, player.id) : null;
  const healthPercent = player?.maxHealth
    ? Math.max(0, Math.min(100, (player.health / player.maxHealth) * 100))
    : 0;
  return (
    <section className="hud" aria-label={t("hud.region")}>
      <div className="health">
        <span style={{ width: `${healthPercent}%` }} />
      </div>
      {player && (
        <div className="progression" aria-label={t("hud.progression")}>
          <strong>{t("hud.level", { level: player.level })}</strong>
          <span>
            {t("hud.xp", {
              current: player.experienceIntoLevel,
              next: player.experienceForNextLevel,
            })}
          </span>
          <span>{t("hud.damage", { damage: player.attackDamage })}</span>
          <span>{t("hud.gold", { gold: player.gold })}</span>
          <span>{t("hud.guard", { current: player.guardPoints, max: player.maxGuardPoints })}</span>
          <span>{t(player.weapon === "bow" ? "hud.weapon.bow" : "hud.weapon.sword")}</span>
          {player.drawTicks != null && <span>{t("hud.draw", { ticks: player.drawTicks })}</span>}
          {player.lockedMonsterId != null && <span>{t("hud.targetLocked")}</span>}
        </div>
      )}
      <p>{status}</p>
      {player && !player.alive && <p className="defeated-status">{t("hud.defeated")}</p>}
      {player?.alive && interactionPromptText(player.interaction, interactKey) && (
        <p className="interaction-prompt">
          {interactionPromptText(player.interaction, interactKey)}
        </p>
      )}
      {player?.alive && (player.guard || player.reaction?.kind === "guardBroken") && (
        <p className="guard-status" aria-live="polite">
          {t(
            player.reaction?.kind === "guardBroken"
              ? "hud.guardBroken"
              : player.reaction?.kind === "blocked"
                ? "hud.blocked"
                : player.guard.phase === "raised"
                  ? "hud.shieldRaised"
                  : "hud.raisingShield",
          )}
        </p>
      )}
      {player?.alive && player.counter && snapshot.tick < player.counter.expiresAtTick && (
        <p className="counter-prompt">
          {/* Announced once when the opportunity opens; the per-tick countdown is visual only. */}
          <span role="alert">{t("hud.counter")}</span>{" "}
          <span aria-hidden="true">
            · {t("hud.ticks", { ticks: player.counter.expiresAtTick - snapshot.tick })}
          </span>
        </p>
      )}
      {threat && (
        <p className="threat-status" data-phase={threat.phase}>
          {incomingThreatText(threat)}
        </p>
      )}
      {player?.action && (
        <p className="action-status">
          {t(ACTION_KEYS[player.action.kind] ?? "hud.action.primaryAttack")} · {player.action.phase}{" "}
          · {t("hud.ticks", { ticks: player.action.ticksRemaining })}
        </p>
      )}
      <p className="desktop-controls-hint">
        {t("hud.desktopHint", {
          move: movementKeys(keyLabels),
          jump: keyOf(keyLabels, "game.jump"),
          primary: keyOf(keyLabels, "game.primaryAttack"),
          heavy: keyOf(keyLabels, "game.secondaryAttack"),
          guard: keyOf(keyLabels, "game.guard"),
          swap: keyOf(keyLabels, "game.switchWeapon"),
          interact: keyOf(keyLabels, "game.interact"),
          menu: keyOf(keyLabels, "game.menu"),
        })}
      </p>
      <p className="mobile-controls-hint">
        {t(player?.weapon === "bow" ? "hud.mobileHint.bow" : "hud.mobileHint.sword")}
      </p>
    </section>
  );
}

// The HUD label of each authoritative action kind; any other kind reads as the primary.
const ACTION_KEYS = {
  secondaryAttack: "hud.action.secondaryAttack",
  interact: "hud.action.interact",
  counter: "hud.action.counter",
  lightFollowUp: "hud.action.lightFollowUp",
  lightFinisher: "hud.action.lightFinisher",
  heavyFinisher: "hud.action.heavyFinisher",
};

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
    <section className="combat-actions" aria-label={t("combat.region")}>
      <button
        type="button"
        className={`combat-action combat-action-primary ${light ? "is-committed" : ""} ${
          player?.counter ? "is-counter-ready" : ""
        }`}
        data-phase={light ? player.action.phase : undefined}
        data-counter={player?.counter ? "ready" : undefined}
        aria-label={t(bow ? "combat.drawLabel" : "combat.primaryLabel")}
        data-draw={player?.drawTicks != null ? "drawing" : undefined}
        disabled={!canAttack}
        {...drawHandlers}
      >
        <strong>
          {t(bow ? "combat.draw" : player?.counter ? "combat.counter" : "combat.attack")}
        </strong>
        <span>{keyOf(keyLabels, "game.primaryAttack")}</span>
      </button>
      <button
        type="button"
        className={`combat-action combat-action-secondary ${heavy ? "is-committed" : ""}`}
        data-phase={heavy ? player.action.phase : undefined}
        aria-label={t("combat.heavyLabel")}
        disabled={!canAttack || bow}
        {...heavyHandlers}
      >
        <strong>{t("combat.heavy")}</strong>
        <span>{keyOf(keyLabels, "game.secondaryAttack")}</span>
      </button>
      <button
        type="button"
        className={`combat-action combat-action-guard ${player?.guard ? "is-committed" : ""}`}
        data-guard={player?.guard?.phase}
        aria-label={t("combat.guardLabel")}
        disabled={!playerId || !player?.alive || bow}
        {...guardHandlers}
      >
        <strong>{t("combat.guard")}</strong>
        <span>{keyOf(keyLabels, "game.guard")}</span>
      </button>
      <button
        type="button"
        className="combat-action combat-action-swap"
        aria-label={t(bow ? "combat.toSwordLabel" : "combat.toBowLabel")}
        disabled={!canAttack || Boolean(player?.action)}
        {...swapHandlers}
      >
        <strong>{t(bow ? "combat.sword" : "combat.bow")}</strong>
        <span>{keyOf(keyLabels, "game.switchWeapon")}</span>
      </button>
      <button
        type="button"
        className={`combat-action combat-action-interact ${
          player?.action?.kind === "interact" ? "is-committed" : ""
        }`}
        data-phase={player?.action?.kind === "interact" ? player.action.phase : undefined}
        aria-label={t("combat.interactLabel")}
        disabled={!playerId || !player?.alive || Boolean(player?.action)}
        {...interactHandlers}
      >
        <strong>{t("combat.interact")}</strong>
        <span>{keyOf(keyLabels, "game.interact")}</span>
      </button>
      <button
        type="button"
        className={`combat-action combat-action-jump ${
          player && !player.motion?.grounded ? "is-airborne" : ""
        }`}
        data-motion={player ? (player.motion?.grounded ? "grounded" : "airborne") : undefined}
        aria-label={t("combat.jumpLabel")}
        disabled={!canAttack}
        {...jumpHandlers}
      >
        <strong>{t("combat.jump")}</strong>
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
    const what = t(
      interaction.target.kind === "chest" ? "interaction.openChest" : "interaction.pickUpGold",
    );
    return key ? t("interaction.prompt", { key, what }) : t("interaction.unbound", { what });
  }
  if (interaction?.reason === "chestLocked") return t("interaction.chestLocked");
  if (interaction?.reason === "obstructed") return t("interaction.obstructed");
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
  const slowedMonsterCount = snapshot?.monsters.filter((monster) => monster.slow).length ?? 0;
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
        <dt>Slowed</dt>
        <dd>{slowedMonsterCount}</dd>
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
      {t("settings.currentMode")} <strong>{mode}</strong>
      {playerId ? t("settings.player", { id: String(playerId) }) : ""}
      {snapshot ? t("settings.runSeed", { seed: String(snapshot.runSeed) }) : ""}
    </p>
  );
}
