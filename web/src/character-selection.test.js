import { t } from "./messages.ts";
import { describe, expect, test } from "bun:test";
import {
  CHARACTER_PRESETS,
  CHARACTER_SELECTION_KEY,
  loadSelectedCharacterId,
  persistSelectedCharacterId,
  resolveCharacter,
} from "./character-selection.js";

function storageWith(initial = {}) {
  const values = new Map(Object.entries(initial));
  return {
    getItem(key) {
      return values.has(key) ? values.get(key) : null;
    },
    setItem(key, value) {
      values.set(key, String(value));
    },
    value(key) {
      return values.get(key);
    },
  };
}

describe("character selection", () => {
  test("ships stable, unique presentation presets", () => {
    expect(CHARACTER_PRESETS.length).toBeGreaterThanOrEqual(3);
    expect(new Set(CHARACTER_PRESETS.map((character) => character.id)).size).toBe(
      CHARACTER_PRESETS.length,
    );
    for (const character of CHARACTER_PRESETS) {
      expect(t(character.role)).toBe("Adventurer");
      expect(t(character.appearance)).not.toBe(character.appearance);
      expect(character.accent).toMatch(/^#[0-9a-f]{6}$/i);
    }
  });

  test("falls back to the first preset for unknown persisted data", () => {
    const storage = storageWith({ [CHARACTER_SELECTION_KEY]: "missing-character" });
    expect(loadSelectedCharacterId(storage)).toBe(CHARACTER_PRESETS[0].id);
    expect(resolveCharacter("missing-character")).toEqual(CHARACTER_PRESETS[0]);
  });

  test("persists only a known character id", () => {
    const storage = storageWith();
    const selected = persistSelectedCharacterId(storage, CHARACTER_PRESETS[1].id);
    expect(selected).toBe(CHARACTER_PRESETS[1].id);
    expect(storage.value(CHARACTER_SELECTION_KEY)).toBe(CHARACTER_PRESETS[1].id);

    persistSelectedCharacterId(storage, "not-real");
    expect(storage.value(CHARACTER_SELECTION_KEY)).toBe(CHARACTER_PRESETS[0].id);
  });

  test("remains usable when browser storage is unavailable", () => {
    const unavailableStorage = {
      getItem() {
        throw new Error("blocked");
      },
      setItem() {
        throw new Error("blocked");
      },
    };
    expect(loadSelectedCharacterId(unavailableStorage)).toBe(CHARACTER_PRESETS[0].id);
    expect(persistSelectedCharacterId(unavailableStorage, CHARACTER_PRESETS[2].id)).toBe(
      CHARACTER_PRESETS[2].id,
    );
  });
});
