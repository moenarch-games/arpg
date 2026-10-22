// The one-time migration to the Space = Jump standard keyboard scheme (#59).
import { expect, test } from "bun:test";
import {
  DEFAULT_PROFILE_ID,
  LEGACY_PROFILE_KEY,
  PROFILE_KEY,
  isUnmodifiedLegacyProfile,
  legacyDefaults,
  loadInputProfile,
  migrateLegacyProfile,
} from "./input-profile.js";

const key = (value) => [{ key: { kind: "physical", value }, modifiers: {} }];
const binding = (action, value) => ({
  id: `${action}.default`,
  action,
  sequence: key(value),
  when: { op: "context", id: "gameplay" },
  priority: 0,
});

// The current standard defaults (a subset of the registry is enough).
const DEFAULTS = [
  binding("game.jump", "Space"),
  binding("game.primaryAttack", "KeyR"),
  binding("game.secondaryAttack", "KeyQ"),
  binding("game.interact", "KeyE"),
];

// A stand-in for input-bindings' `applyProfile`: add, remove and replace by binding id,
// reporting a patch that names a missing binding. The browser suite runs the real one.
const foundation = {
  applyProfile(base, profile) {
    const bindings = new Map(base.map((entry) => [entry.id, structuredClone(entry)]));
    const diagnostics = [];
    profile.patches.forEach((patch, patchIndex) => {
      const id = patch.op === "add" ? patch.binding.id : patch.bindingId;
      if (patch.op === "add") {
        if (bindings.has(id)) diagnostics.push({ patchIndex, kind: "addCollision", bindingId: id });
        else bindings.set(id, patch.binding);
      } else if (!bindings.has(id)) {
        diagnostics.push({ patchIndex, kind: "missingBinding", bindingId: id });
      } else if (patch.op === "remove") bindings.delete(id);
      else bindings.set(id, patch.binding);
    });
    return { bindings: [...bindings.values()], diagnostics };
  },
};

const effective = (profile) =>
  Object.fromEntries(
    foundation
      .applyProfile(DEFAULTS, profile)
      .bindings.map((entry) => [entry.action, entry.sequence[0].key.value]),
  );

function memoryStorage(initial = {}) {
  const values = new Map(Object.entries(initial));
  const writes = [];
  return {
    values,
    writes,
    getItem: (name) => (values.has(name) ? values.get(name) : null),
    setItem(name, value) {
      writes.push(name);
      values.set(name, String(value));
    },
  };
}

const load = (storage, validate = () => true) =>
  loadInputProfile({ validate, defaults: DEFAULTS, foundation }, storage);

test("the old defaults had Primary Attack on Space and no Jump", () => {
  expect(PROFILE_KEY).toBe("arpg-input-profile-v2");
  expect(LEGACY_PROFILE_KEY).toBe("arpg-input-profile-v1");
  const old = Object.fromEntries(
    legacyDefaults(DEFAULTS).map((entry) => [entry.action, entry.sequence[0].key.value]),
  );
  expect(old).toEqual({
    "game.primaryAttack": "Space",
    "game.secondaryAttack": "KeyQ",
    "game.interact": "KeyE",
  });
});

test("a fresh browser gets the standard scheme without writing anything", () => {
  const storage = memoryStorage();
  const profile = load(storage);
  expect(profile).toEqual({ id: DEFAULT_PROFILE_ID, patches: [] });
  expect(effective(profile)["game.jump"]).toBe("Space");
  expect(effective(profile)["game.primaryAttack"]).toBe("KeyR");
  expect(storage.writes).toEqual([]);
});

test("a provably unmodified old profile migrates to the standard scheme", () => {
  // Empty, and a no-op replacement that restores the old default exactly.
  for (const patches of [
    [],
    [
      {
        op: "replace",
        bindingId: "game.primaryAttack.default",
        binding: binding("game.primaryAttack", "Space"),
      },
    ],
  ]) {
    const legacy = { id: "arpg-player", patches };
    expect(isUnmodifiedLegacyProfile(legacy, DEFAULTS, foundation)).toBe(true);
    const storage = memoryStorage({ [LEGACY_PROFILE_KEY]: JSON.stringify(legacy) });
    const profile = load(storage);
    expect(profile).toEqual({ id: "arpg-player", patches: [] });
    expect(effective(profile)["game.jump"]).toBe("Space");
    expect(effective(profile)["game.primaryAttack"]).toBe("KeyR");
    expect(JSON.parse(storage.values.get(PROFILE_KEY))).toEqual(profile);
    // The old document stays for rollback.
    expect(storage.values.get(LEGACY_PROFILE_KEY)).toBe(JSON.stringify(legacy));
  }
});

test("a customized old profile keeps every binding it had and leaves Jump unbound", () => {
  // Heavy moved to KeyH; Primary Attack deliberately stays on the old default Space.
  const legacy = {
    id: "arpg-player",
    patches: [
      {
        op: "replace",
        bindingId: "game.secondaryAttack.default",
        binding: binding("game.secondaryAttack", "KeyH"),
      },
    ],
  };
  expect(isUnmodifiedLegacyProfile(legacy, DEFAULTS, foundation)).toBe(false);
  const profile = migrateLegacyProfile(legacy, DEFAULTS, foundation);
  expect(effective(profile)).toEqual({
    "game.primaryAttack": "Space",
    "game.secondaryAttack": "KeyH",
    "game.interact": "KeyE",
  });
  // Its own patches still apply to the same binding ids, after the pinning ones.
  expect(profile.patches.slice(2)).toEqual(legacy.patches);

  // A customized profile that had moved Primary Attack keeps that key.
  const moved = migrateLegacyProfile(
    {
      id: "arpg-player",
      patches: [
        {
          op: "replace",
          bindingId: "game.primaryAttack.default",
          binding: binding("game.primaryAttack", "KeyJ"),
        },
      ],
    },
    DEFAULTS,
    foundation,
  );
  expect(effective(moved)["game.primaryAttack"]).toBe("KeyJ");
  expect(effective(moved)["game.jump"]).toBeUndefined();
});

test("the migration runs once and replaying it is idempotent", () => {
  const legacy = {
    id: "arpg-player",
    patches: [{ op: "remove", bindingId: "game.interact.default" }],
  };
  const storage = memoryStorage({ [LEGACY_PROFILE_KEY]: JSON.stringify(legacy) });
  const first = load(storage);
  expect(storage.writes).toEqual([PROFILE_KEY]);
  // A later load reads the version 2 profile and writes nothing.
  expect(load(storage)).toEqual(first);
  expect(storage.writes).toEqual([PROFILE_KEY]);
  // A version 2 profile the player edits later is never migrated again.
  const edited = { id: "arpg-player", patches: [] };
  storage.values.set(PROFILE_KEY, JSON.stringify(edited));
  expect(load(storage)).toEqual(edited);
  // Where the write was refused, the next load migrates to the same profile.
  const blocked = memoryStorage({ [LEGACY_PROFILE_KEY]: JSON.stringify(legacy) });
  blocked.setItem = () => {
    throw new Error("blocked");
  };
  expect(load(blocked)).toEqual(first);
  expect(load(blocked)).toEqual(first);
});

test("unreadable or invalid profiles fall back to the standard scheme", () => {
  const standard = { id: DEFAULT_PROFILE_ID, patches: [] };
  expect(load(memoryStorage({ [PROFILE_KEY]: "{not json" }))).toEqual(standard);
  expect(load(memoryStorage({ [PROFILE_KEY]: JSON.stringify({ id: 3 }) }))).toEqual(standard);
  const legacy = memoryStorage({
    [LEGACY_PROFILE_KEY]: JSON.stringify({ id: "arpg-player", patches: [{ op: "bogus" }] }),
  });
  expect(load(legacy, () => false)).toEqual(standard);
  expect(legacy.writes).toEqual([]);
  const throwing = {
    getItem() {
      throw new Error("blocked");
    },
    setItem() {
      throw new Error("blocked");
    },
  };
  expect(load(throwing)).toEqual(standard);
});
