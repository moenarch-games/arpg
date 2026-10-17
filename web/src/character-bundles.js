// Released asset-tooling character bundles the game ships (arpg#55, Atlas 3.2.1). The
// setup step unpacks each into `public/characters/<release>/`.
export const RANGER_BUNDLE = Object.freeze({
  release: "humanoid-ranger-male-v2",
  url: "https://github.com/moritzbrantner/asset-tooling/releases/download/humanoid-ranger-male-v2/humanoid.ranger.male.tar",
  key: "humanoid.ranger.male",
  variant: "default",
  // Presentation data for the character's clips (#188). Gaits are in-place clips; each
  // `speed` is the ground speed (m/s) its stride matches, used to scale playback, and
  // `upTo` the authoritative speed up to which that gait is chosen.
  animation: Object.freeze({
    idle: "Idle_Loop",
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
});

export const CHARACTER_BUNDLES = Object.freeze([RANGER_BUNDLE]);
