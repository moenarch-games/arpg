// Released asset-tooling character bundles the game ships (arpg#55, Atlas 3.2.1). The
// setup step unpacks each into `public/characters/<release>/`.

const segment = (clip, from, to) => Object.freeze({ clip, from, to });
// A light strike: wind-up to the contact, the active phase to the strike's end, then the
// clip's own recovery.
const lightStrike = (clip, contact, end, recoveryEnd) =>
  Object.freeze({
    windup: segment(clip, 0, contact),
    active: segment(clip, contact, end),
    recovery: segment(`${clip}_Rec`, 0, recoveryEnd),
  });
// The heavy strike `Sword_Regular_C` (2 s, contact 0.65 s) carries its own recovery.
const heavyStrike = () =>
  Object.freeze({
    windup: segment("Sword_Regular_C", 0, 0.65),
    active: segment("Sword_Regular_C", 0.65, 0.85),
    recovery: segment("Sword_Regular_C", 0.85, 2),
  });
export const RANGER_BUNDLE = Object.freeze({
  release: "humanoid-ranger-male",
  index:
    "https://github.com/moritzbrantner/asset-tooling/releases/download/humanoid-ranger-male/humanoid-ranger-male.index.json",
  ref: "https://github.com/moritzbrantner/asset-tooling/releases/download/humanoid-ranger-male/export.ref.json",
  key: "humanoid.ranger.male",
  variant: "default",
  // Presentation data for the character's clips (#188). Gaits are in-place clips; each
  // `speed` is the ground speed (m/s) its stride matches, used to scale playback, and
  // `upTo` the authoritative speed up to which that gait is chosen.
  animation: Object.freeze({
    idle: "Idle_Loop",
    // Standing while a monster is engaged with the character, and while guarding (#183).
    engagedIdle: "Sword_Idle",
    guard: "Idle_Shield_Loop",
    death: "Death01",
    standingBelow: 0.3,
    gaits: Object.freeze([
      Object.freeze({ clip: "Walk_Loop", speed: 1.4, upTo: 2.2 }),
      Object.freeze({ clip: "Jog_Fwd_Loop", speed: 3.4, upTo: 4.8 }),
      Object.freeze({ clip: "Sprint_Loop", speed: 5.6, upTo: Infinity }),
    ]),
    crossfadeSeconds: 0.2,
    rate: Object.freeze({ min: 0.6, max: 1.6 }),
    // Actions (#56): each authoritative phase plays a stretch of a clip, `from`..`to` clip
    // seconds over the phase's ticks, so the swing's contact (the `humanoid-clips` contact
    // time) lands exactly when the core's active phase opens. Kinds without an entry
    // (interact) keep the locomotion pose.
    actions: Object.freeze({
      primaryAttack: lightStrike("Sword_Regular_A", 0.25, 0.4333, 0.9667),
      lightFollowUp: lightStrike("Sword_Regular_B", 0.25, 0.5333, 1.0333),
      lightFinisher: lightStrike("Sword_Regular_A", 0.25, 0.4333, 0.9667),
      counter: lightStrike("Sword_Regular_B", 0.25, 0.5333, 1.0333),
      secondaryAttack: heavyStrike(),
      heavyFinisher: heavyStrike(),
      shoot: Object.freeze({
        windup: segment("Bow_Release", 0, 0.0167),
        active: segment("Bow_Release", 0.0167, 0.4667),
        recovery: segment("Bow_Recovery", 0, 1.1),
      }),
    }),
    actionCrossfadeSeconds: 0.08,
    // Reactions play once from the moment the core starts them (a new hit restarts them).
    reactions: Object.freeze({
      hurt: "Hit_Chest",
      blocked: "Sword_Block",
      guardBroken: "Idle_Shield_Break",
    }),
    // The bow's draw follows the authoritative draw ticks, then holds.
    bow: Object.freeze({ draw: "Bow_Draw", hold: "Bow_Hold_Loop" }),
    // Airborne from the body's authoritative support (#59): a jump (rising) or a fall
    // longer than `fallAfterSeconds` takes off then loops; touching down while standing
    // plays the first `landSeconds` of the landing.
    air: Object.freeze({
      takeoff: "Jump_Start",
      fall: "Jump_Loop",
      land: "Jump_Land",
      fallAfterSeconds: 0.1,
      landSeconds: 0.4,
    }),
  }),
  // Equipment presentation data (#183, Atlas 3.2.3); see `character-equipment.ts`.
  equipment: Object.freeze({
    items: Object.freeze({
      sword: Object.freeze({ key: "equipment.sword.arming", frame: "grip" }),
      shield: Object.freeze({ key: "equipment.shield.heater", frame: "grip" }),
    }),
    places: Object.freeze({
      right: Object.freeze({
        mainHand: "hand_r.weapon",
        offHand: "hand_l.shield",
        hipStow: "hip_l.stowed",
        backStow: "back.stowed",
      }),
      left: Object.freeze({
        mainHand: "hand_l.weapon",
        offHand: "hand_r.shield",
        hipStow: "hip_r.stowed",
        backStow: "back.stowed",
      }),
    }),
    handedness: "right",
    loadouts: Object.freeze({
      swordAndShield: Object.freeze([
        Object.freeze({ item: "sword", place: "mainHand" }),
        Object.freeze({ item: "shield", place: "offHand" }),
      ]),
      bow: Object.freeze([
        Object.freeze({ item: "sword", place: "hipStow" }),
        Object.freeze({ item: "shield", place: "backStow" }),
      ]),
    }),
  }),
});

export const CHARACTER_BUNDLES = Object.freeze([RANGER_BUNDLE]);
