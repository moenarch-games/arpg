// The CC0 combat and UI sound set the game ships (#67, 67c; asset-tooling#297). The setup
// step downloads the rolling `combat-sounds` release, checks the tar against this pin and
// stages its WAV clips and `sound-manifest.json` under `public/sounds/<release>/`.
//
// The pin is the tar's sha256 (asset-tooling commit 0e0e5b75, profile `audio-wav-v1`; the
// tar passed `bun scripts/pack-asset-bundle.ts --verify combat-sounds.tar --ref
// export.ref.json` at that commit). A replaced release fails setup until it is re-pinned.
export const COMBAT_SOUNDS = Object.freeze({
  release: "combat-sounds",
  tar: "https://github.com/moritzbrantner/asset-tooling/releases/download/combat-sounds/combat-sounds.tar",
  sha256: "f490c0676bfdce1d0ae309efc3f11ef955da0a1d9e3de3192fc36d60ee5ae15c",
  // The bundle asset holding the game-audio `SoundManifest`; clip URLs are bundle-relative.
  manifestKey: "combat-sounds.sound-manifest",
});
