// Released asset-tooling character bundles the game ships (arpg#55, Atlas 3.2.1). The
// setup step unpacks each into `public/characters/<release>/`.
export const RANGER_BUNDLE = Object.freeze({
  release: "humanoid-ranger-male-v1",
  url: "https://github.com/moritzbrantner/asset-tooling/releases/download/humanoid-ranger-male-v1/humanoid.ranger.male.tar",
  key: "humanoid.ranger.male",
  variant: "default",
  idleClip: "Idle_Loop",
});

export const CHARACTER_BUNDLES = Object.freeze([RANGER_BUNDLE]);
