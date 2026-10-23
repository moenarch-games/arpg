// Released asset-tooling character bundles the game ships (arpg#55, Atlas 3.2.1). The
// setup step unpacks each into `public/characters/<release>/`.
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
