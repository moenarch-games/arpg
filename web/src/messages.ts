// Player-facing text (#61, 61b): one typed English message catalog formatted with `Intl`.
// English is the only shipped locale. Every `t()` key is checked against `en`, a catalog
// may leave keys out (they fall back to English), and an unknown key renders as itself.
//
// A message is a template with `{name}` placeholders, or plural forms chosen by the
// `count` parameter with `Intl.PluralRules`. Number parameters are formatted with
// `Intl.NumberFormat`; pass identifiers (seeds, ticks, ids) as strings so they keep
// their digits.
//
// The training workbench (panel, diagnostics, event timeline) is developer tooling and
// stays in English outside this catalog.

type PluralMessage = { [form in Intl.LDMLPluralRule]?: string } & { other: string };
type Message = string | PluralMessage;
export type Params = Record<string, string | number>;

export const en = {
  "app.name": "ARPG",
  "app.loading": "Loading game…",
  "app.loadingBytes": "Loading game… {loaded} / {total} MB",
  "app.retry": "Retry",
  "app.world": "ARPG game world",

  "select.region": "Character selection",
  "select.characters": "Characters",
  "select.appearance": "{appearance} appearance · {location}",
  "select.note": "Appearance profiles currently share the same authoritative gameplay rules.",
  "select.ready": "Ready",
  "select.loadSaved": "Load Saved Game",
  "select.importSave": "Import Save",
  "select.training": "Training Arena",
  "select.enterWorld": "Enter World",
  "character.role.adventurer": "Adventurer",
  "character.appearance.bronze": "Bronze",
  "character.appearance.verdant": "Verdant",
  "character.appearance.steel": "Steel",
  "character.location.ashenHollow": "Ashen Hollow",

  "mode.training": "Training arena",
  "mode.local": "Local",
  "mode.host": "Peer host",
  "mode.guest": "Peer guest",
  "mode.dedicated": "Dedicated online",

  "header.menu": "Menu",
  "header.settings": "Settings",
  "touch.region": "Touch controls",
  "touch.joystick": "Movement joystick",

  "menu.heading": "Menu",
  "menu.resume": "Resume",
  "menu.settings": "Settings",
  "menu.leave": "Leave",
  "menu.paused": "Game paused",
  "menu.shared": "World continues · this menu does not pause a shared game",
  "leave.heading": "Leave game?",
  "leave.host": "Leaving ends the hosted session for everyone in it.",
  "leave.disconnect": "You will disconnect from this session.",
  "leave.local": "Progress since your last save is lost.",
  "leave.cancel": "Cancel",
  "leave.confirm": "Leave game",

  "settings.heading": "Settings",
  "settings.close": "Close",
  "settings.game": "Game",
  "settings.currentMode": "Current mode:",
  "settings.player": " · player {id}",
  "settings.runSeed": " · run seed {seed}",
  "settings.startLocal": "Start local game",
  "settings.save": "Save & load",
  "settings.saveAvailable": "Local save available",
  "settings.saveNone": "No local save yet",
  "settings.saveLocal": "Current local authority can be saved or exported.",
  "settings.saveRemote":
    "Loading a save returns to local play; network sessions are not overwritten.",
  "settings.saveGame": "Save game",
  "settings.loadGame": "Load saved game",
  "settings.exportSave": "Export save",
  "settings.importSave": "Import save",
  "settings.saveFile": "Save file",
  "settings.saveNote":
    "Saves are versioned and validated by the Rust authority before they replace the current game. Exported JSON can be imported on another browser or device.",
  "settings.dedicated": "Dedicated online",
  "settings.endpoint": "WebTransport endpoint",
  "settings.connectDedicated": "Connect dedicated server",
  "settings.peer": "Peer co-op",
  "settings.setupUrl": "Setup service URL",
  "settings.host": "Host co-op",
  "settings.copyInvite": "Copy invite {code}",
  "settings.lobbyCode": "Lobby code",
  "settings.join": "Join host",
  "settings.graphics": "Graphics",
  "settings.shadows": "Enable shadows",
  "settings.pixelRatio": "Pixel ratio limit",
  "settings.accessibility": "Accessibility",
  "settings.motion": "Motion effects",
  "settings.motion.system": "Follow system",
  "settings.motion.reduced": "Reduced",
  "settings.motion.full": "Full",
  "settings.flashes": "Reduce bright flashes",
  "settings.combatEffects": "Combat particles and camera impulse",
  "settings.textSize": "Text size",
  "settings.displayNote":
    "Display settings stay in this browser. They never change saves or game rules.",
  "settings.resetDisplay": "Reset display settings",
  "settings.controls": "Controls",
  "settings.mouseAim": "Aim melee and bow with the mouse",

  "input.category.movement": "Movement",
  "input.category.combat": "Combat",
  "input.category.interaction": "Interaction",
  "input.category.camera": "Camera",
  "input.category.system": "System",
  "input.moveForward": "Move forward",
  "input.moveBackward": "Move backward",
  "input.moveLeft": "Move left",
  "input.moveRight": "Move right",
  "input.jump": "Jump",
  "input.primaryAttack": "Primary attack",
  "input.secondaryAttack": "Heavy attack",
  "input.interact": "Interact / pick up",
  "input.guard": "Raise shield (hold)",
  "input.switchWeapon": "Switch sword / bow",
  "input.cycleTarget": "Lock / cycle target",
  "input.clearTarget": "Clear target lock",
  "input.resetCamera": "Reset camera",
  "input.menu": "Open the game menu or close the innermost menu",

  "status.selectCharacter": "Select a character",
  "status.badSeed": "Training seed must be an unsigned 32-bit integer",
  "status.training": "Training arena",
  "status.trainingWith": "Training arena · {name}",
  "status.trainingRestarted": "Training arena restarted · seed {seed}",
  "status.trainingScenario": "Training scenario · {scenario} · seed {seed}",
  "status.unknownScenario": 'Unknown scenario "{scenario}"; using the generated dungeon',
  "status.localGame": "Local game · {name}",
  "status.controlsSession": "Controls updated for this session; browser storage is unavailable",
  "status.displaySession":
    "Display settings updated for this session; browser storage is unavailable",
  "status.endpointSession": "Endpoint updated for this session; browser storage is unavailable",
  "status.loadedSave": "Loaded local save",
  "status.importedSave": "Imported {file}",
  "status.restored": "{source} · tick {tick} · seed {seed}",
  "status.notKept": " · not kept as the local save because browser storage is unavailable",
  "status.saveNoStorage":
    "Could not save game: browser storage is unavailable; export the save instead",
  "status.saved": "Game saved locally · tick {tick}",
  "status.saveFailed": "Could not save game: {error}",
  "status.noSave": "No local save is available",
  "status.loadFailed": "Could not load saved game: {error}",
  "status.exported": "Save exported · tick {tick}",
  "status.exportFailed": "Could not export save: {error}",
  "status.reproduction": {
    one: "Reproduction exported · {scenario} · {count} tick",
    other: "Reproduction exported · {scenario} · {count} ticks",
  },
  "status.reproductionFailed": "Could not export reproduction: {error}",
  "status.importFailed": "Could not import save: {error}",
  "status.inviteCopied": "Invite URL copied",
  "status.inviteFailed": "Could not copy invite: {error}",
  "status.loadError": "Game failed to load: {error}",
  "status.dedicatedCommandFailed": "Dedicated command failed: {error}",
  "status.hosting": "Hosting lobby {code}",
  "status.hostFailed": "Could not host: {error}",
  "status.hostLost": "Host connection lost",
  "status.joining": "Joined lobby {code}; establishing host channel…",
  "status.joinFailed": "Could not join: {error}",
  "status.noWebTransport": "Dedicated online requires browser WebTransport support",
  "status.dedicatedWelcome": "Dedicated authority · player {player} · {hz} Hz",
  "status.dedicatedRejected": "Rejected dedicated snapshot: {error}",
  "status.dedicatedConnecting": "Connecting to dedicated authority…",
  "status.dedicatedDisconnected": "Dedicated authority disconnected",
  "status.dedicatedTransport": "Dedicated transport error: {error}",
  "status.dedicatedFailed": "Could not connect dedicated server: {error}",
  "status.peerRejected": "Rejected peer message: {error}",
  "status.peerWaiting": "Connected to host; waiting for player assignment…",
  "status.lobbyFull": "Lobby is full",
  "status.peerJoined": "Peer connected as player {player}",
  "status.connectedAs": "Connected as player {player}",
  "status.peerLeft": "Player {player} disconnected",
  "status.network": "Network: {state}",

  "hud.region": "Player status",
  "hud.progression": "Character progression",
  "hud.level": "Level {level}",
  "hud.xp": "XP {current}/{next}",
  "hud.damage": "Damage {damage}",
  "hud.gold": "Gold {gold}",
  "hud.guard": "Guard {current}/{max}",
  "hud.weapon.bow": "Bow (unlimited arrows)",
  "hud.weapon.sword": "Sword & shield",
  "hud.draw": "Draw {ticks}t",
  "hud.ticks": "{ticks}t",
  "hud.targetLocked": "Target locked",
  "hud.defeated": "Defeated",
  "hud.checkpoint.entrance": "You return to the entrance with full health.",
  "hud.checkpoint.room": "You return to the last room you cleared with full health.",
  "hud.returnToCheckpoint": "Return to checkpoint",
  "hud.objective.defeat": "Defeat the enemies · {remaining} left",
  "hud.objective.phase": "Phase {phase}/{total}",
  "hud.runComplete": "Run complete",
  "hud.runComplete.detail": "Every room is cleared and the party left the crypt.",
  "hud.returnToEntrance": "Return to the entrance",
  "hud.guardBroken": "Guard broken",
  "hud.blocked": "Blocked",
  "hud.shieldRaised": "Shield raised",
  "hud.raisingShield": "Raising shield",
  "hud.counter": "Counter!",
  "hud.action.secondaryAttack": "Heavy",
  "hud.action.interact": "Interact",
  "hud.action.counter": "Counter",
  "hud.action.lightFollowUp": "Light 2",
  "hud.action.lightFinisher": "Light finisher",
  "hud.action.heavyFinisher": "Heavy finisher",
  "hud.action.primaryAttack": "Primary",
  "hud.unbound": "unbound",
  "hud.desktopHint":
    "{move} move · {jump} jump · {primary} primary / hold to draw bow · {heavy} heavy · {guard} hold shield · {swap} switch bow · {interact} interact · {menu} menu",
  "hud.mobileHint.bow":
    "Left stick · Tap Jump · Hold Draw, release to shoot · Guard and Heavy unavailable with bow",
  "hud.mobileHint.sword":
    "Left stick · Tap Jump · Tap Attack for light, swipe ↑ Heavy, → target, ← unlock · Hold Guard · Tap Interact",
  "threat.strike.windup": "Incoming strike · wind-up",
  "threat.strike.active": "Incoming strike · now",
  "threat.burst.windup": "Incoming burst · wind-up",
  "threat.burst.active": "Incoming burst · now",
  "threat.shot.windup": "Incoming shot · wind-up",
  "threat.shot.active": "Incoming shot · now",

  "hud.statuses": "Statuses",
  "hud.status.poisoned": "Poisoned · {ticks}t",
  "hud.status.slowed": "Slowed · {ticks}t",
  "hud.target": "Target",
  "hud.targetHealth": "{name} · {health}/{max}",
  "hud.combo.step": "Combo {step}/3",
  "hud.combo.finisher": "Combo finisher",
  "hud.combo.lightQueued": "light queued",
  "hud.combo.heavyQueued": "heavy queued",
  "hud.party": "Party",
  "hud.party.member": "Player {player} · {health}/{max}",
  "hud.party.down": "Player {player} · down",

  "monster.name.archer": "Archer",
  "monster.name.bruiser": "Bruiser",
  "monster.name.brute": "Brute",
  "monster.name.defender": "Defender",
  "monster.name.skirmisher": "Skirmisher",
  "monster.name.other": "Enemy",

  "cue.region": "Combat feedback",
  "cue.damageNumbers": "Damage numbers",
  "cue.hit": "Hit {name}",
  "cue.hurt": "Hurt",
  "cue.youBlocked": "Blocked",
  "cue.yourStrikeBlocked": "{name} blocked your strike",
  "cue.yourGuardBroken": "Guard broken",
  "cue.enemyGuardBroken": "{name}'s guard broken",
  "cue.obstructed": "Strike obstructed",
  "cue.whiff": "Missed",
  "cue.defeated": "{name} defeated",
  "cue.allyDown": "Player {player} is down",
  "cue.pickedUp": "+{gold} gold",
  "cue.opened": "Chest opened · +{gold} gold",
  "cue.exited": "The party left the crypt",
  "cue.arrowRelease": "Arrow loosed",
  "cue.arrowLanded": "Arrow hit the world",
  "cue.counterUsed": "Counter strike",
  "cue.counterExpired": "Counter window passed",
  "cue.counterLost": "Counter lost",
  "cue.encounter.active": "Encounter started",
  "cue.encounter.cleared": "Room cleared",
  "cue.encounter.dormant": "Encounter reset",
  "cue.phase": "The fight escalates · phase {phase}/{total}",
  "interaction.openChest": "Open chest",
  "interaction.pickUpGold": "Pick up gold",
  "interaction.prompt": "{key} · {what}",
  "interaction.unbound": "{what} (Interact is unbound)",
  "interaction.chestLocked": "Chest locked until the room is cleared",
  "interaction.takeExit": "Leave the crypt",
  "interaction.exitLocked": "Exit sealed until every room is cleared",
  "interaction.obstructed": "Out of reach behind a wall",

  "combat.region": "Combat actions",
  "combat.primaryLabel": "Primary attack",
  "combat.drawLabel": "Draw bow",
  "combat.heavyLabel": "Heavy attack",
  "combat.guardLabel": "Hold shield",
  "combat.toSwordLabel": "Switch to sword and shield",
  "combat.toBowLabel": "Switch to bow",
  "combat.interactLabel": "Interact or pick up",
  "combat.jumpLabel": "Jump",
  "combat.attack": "Attack",
  "combat.draw": "Draw",
  "combat.counter": "Counter",
  "combat.heavy": "Heavy",
  "combat.guard": "Guard",
  "combat.sword": "Sword",
  "combat.bow": "Bow",
  "combat.interact": "Interact",
  "combat.jump": "Jump",
} as const satisfies Record<string, Message>;

export type MessageKey = keyof typeof en;
export type Catalog = { readonly [key in MessageKey]?: Message };

let locale = "en";
let catalog: Catalog = en;
let plurals = new Intl.PluralRules(locale);
let numbers = new Intl.NumberFormat(locale);

// Replaces the active catalog. English fills any key the catalog leaves out.
export function setMessages(nextLocale: string, next: Catalog) {
  locale = nextLocale;
  catalog = next;
  plurals = new Intl.PluralRules(locale);
  numbers = new Intl.NumberFormat(locale);
}

export function t(key: MessageKey, params: Params = {}): string {
  const message: Message | undefined = catalog[key] ?? en[key];
  if (message === undefined) return key;
  const template =
    typeof message === "string"
      ? message
      : (message[plurals.select(Number(params.count ?? 0))] ?? message.other);
  return template.replace(/\{(\w+)\}/g, (placeholder, name) => {
    const value = params[name];
    if (value === undefined) return placeholder;
    return typeof value === "number" ? numbers.format(value) : value;
  });
}

// A percentage such as a text scale (1.25 → "125%").
export function formatPercent(fraction: number): string {
  return new Intl.NumberFormat(locale, { style: "percent" }).format(fraction);
}

// A byte count in megabytes with one decimal.
export function formatMegabytes(bytes: number): string {
  return new Intl.NumberFormat(locale, {
    minimumFractionDigits: 1,
    maximumFractionDigits: 1,
  }).format(bytes / 1_000_000);
}

// Test-only long-label locale: accents every letter and lengthens every word by about
// 60%, keeping placeholders, so browser tests can prove layouts survive longer text.
const ACCENTED: Record<string, string> = {
  a: "á",
  e: "é",
  i: "í",
  o: "ó",
  u: "ú",
  c: "ç",
  n: "ñ",
  s: "š",
  y: "ý",
  z: "ž",
  A: "Á",
  E: "É",
  I: "Í",
  O: "Ó",
  U: "Ú",
  C: "Ç",
  N: "Ñ",
  S: "Š",
  Y: "Ý",
  Z: "Ž",
};

function lengthen(text: string): string {
  return text
    .split(/(\{\w+\})/)
    .map((part, index) =>
      index % 2 === 1
        ? part
        : part.replace(/[A-Za-z]+/g, (word) => {
            const accented = [...word].map((letter) => ACCENTED[letter] ?? letter).join("");
            return accented + accented.slice(0, Math.ceil(word.length * 0.6)).toLowerCase();
          }),
    )
    .join("");
}

export function longLabelCatalog(): Catalog {
  return Object.fromEntries(
    Object.entries(en).map(([key, message]) => [
      key,
      typeof message === "string"
        ? lengthen(message)
        : Object.fromEntries(Object.entries(message).map(([form, text]) => [form, lengthen(text)])),
    ]),
  ) as Catalog;
}
