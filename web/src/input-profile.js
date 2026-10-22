// The player's keyboard profile and its one-time migration to the Space = Jump standard
// scheme (#59; owner decisions B and C of #57).
//
// A profile is a list of input-bindings patches over the registry defaults. Version 1
// profiles were written against the defaults before Jump existed, when Space was Primary
// Attack. The version 2 defaults bind Space to Jump and R to Primary Attack.
//
// The migration runs once, when no version 2 profile exists yet, and only reads the
// version 1 key (which stays for rollback):
//
// - No stored profile, or one that provably leaves the old defaults unchanged (its patches
//   applied to the old defaults give exactly the old defaults), becomes the new standard
//   profile.
// - Any other, customized profile keeps exactly the bindings it had, a deliberate
//   Space = Primary Attack included: the migration pins Primary Attack to its old default
//   key and leaves Jump unbound, which the shared editor shows. Nothing is reassigned.
//
// The result is written as version 2, so the migration never runs again. Replaying it on
// the same version 1 profile gives the same result, so a load whose write was refused
// (blocked storage) migrates identically next time.
//
// `foundation.applyProfile` is input-bindings' own patch application, passed in so that
// these rules stay free of the vendored foundation's import path.
import { persistStoredValue, readStoredValue } from "./preferences.js";

export const LEGACY_PROFILE_KEY = "arpg-input-profile-v1";
export const PROFILE_KEY = "arpg-input-profile-v2";
export const DEFAULT_PROFILE_ID = "arpg-player";

const PRIMARY_DEFAULT = "game.primaryAttack.default";
const JUMP_DEFAULT = "game.jump.default";
// The key Primary Attack had by default before Space became Jump.
const LEGACY_PRIMARY_KEY = "Space";

const defaultProfile = () => ({ id: DEFAULT_PROFILE_ID, patches: [] });

function read(key, storage) {
  try {
    return JSON.parse(readStoredValue(key, "null", storage));
  } catch {
    // An unreadable document counts as absent.
    return null;
  }
}

const isProfile = (value) =>
  value !== null &&
  typeof value === "object" &&
  typeof value.id === "string" &&
  Array.isArray(value.patches);

// The default bindings as version 1 profiles saw them: no Jump, Primary Attack on Space.
export function legacyDefaults(defaults) {
  return defaults
    .filter((binding) => binding.id !== JUMP_DEFAULT)
    .map((binding) =>
      binding.id === PRIMARY_DEFAULT
        ? {
            ...binding,
            sequence: [{ key: { kind: "physical", value: LEGACY_PRIMARY_KEY }, modifiers: {} }],
          }
        : binding,
    );
}

const canonical = (bindings) =>
  JSON.stringify([...bindings].sort((left, right) => left.id.localeCompare(right.id)));

// Whether a version 1 profile provably leaves the old defaults exactly as they were.
export function isUnmodifiedLegacyProfile(profile, defaults, foundation) {
  const old = legacyDefaults(defaults);
  const applied = foundation.applyProfile(old, profile);
  return applied.diagnostics.length === 0 && canonical(applied.bindings) === canonical(old);
}

// The version 2 profile for a version 1 profile (see the header).
export function migrateLegacyProfile(profile, defaults, foundation) {
  if (isUnmodifiedLegacyProfile(profile, defaults, foundation)) {
    return { id: profile.id, patches: [] };
  }
  const primary = defaults.find((binding) => binding.id === PRIMARY_DEFAULT);
  const [pinnedPrimary] = legacyDefaults([primary]);
  return {
    id: profile.id,
    // Pinned first, so the profile's own patches still apply to the same binding ids.
    patches: [
      { op: "replace", bindingId: PRIMARY_DEFAULT, binding: pinnedPrimary },
      { op: "remove", bindingId: JUMP_DEFAULT },
      ...profile.patches,
    ],
  };
}

// The stored version 2 profile, migrating a version 1 profile once. `validate` checks a
// profile against the current registry; an invalid document falls back to the defaults.
export function loadInputProfile({ validate, defaults, foundation }, storage) {
  const current = read(PROFILE_KEY, storage);
  if (isProfile(current) && validate(current)) return current;
  if (current !== null) return defaultProfile();
  const legacy = read(LEGACY_PROFILE_KEY, storage);
  if (!isProfile(legacy)) return defaultProfile();
  const migrated = migrateLegacyProfile(legacy, defaults, foundation);
  if (!validate(migrated)) return defaultProfile();
  persistStoredValue(PROFILE_KEY, JSON.stringify(migrated), storage);
  return migrated;
}
