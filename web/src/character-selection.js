// Display text (`role`, `appearance`, `location`) holds message-catalog keys.
export const CHARACTER_SELECTION_KEY = "arpg-character-selection-v1";

export const CHARACTER_PRESETS = Object.freeze([
  {
    id: "aldren",
    name: "Aldren",
    role: "character.role.adventurer",
    appearance: "character.appearance.bronze",
    location: "character.location.ashenHollow",
    accent: "#d6b45f",
    tone: "bronze",
  },
  {
    id: "mara",
    name: "Mara",
    role: "character.role.adventurer",
    appearance: "character.appearance.verdant",
    location: "character.location.ashenHollow",
    accent: "#7f9d72",
    tone: "verdant",
  },
  {
    id: "thorne",
    name: "Thorne",
    role: "character.role.adventurer",
    appearance: "character.appearance.steel",
    location: "character.location.ashenHollow",
    accent: "#7e9eb6",
    tone: "steel",
  },
]);

export function resolveCharacter(characterId) {
  return (
    CHARACTER_PRESETS.find((character) => character.id === characterId) ?? CHARACTER_PRESETS[0]
  );
}

export function loadSelectedCharacterId(storage) {
  try {
    const stored = (storage ?? globalThis.localStorage)?.getItem?.(CHARACTER_SELECTION_KEY);
    return resolveCharacter(stored).id;
  } catch {
    return CHARACTER_PRESETS[0].id;
  }
}

export function persistSelectedCharacterId(storage, characterId) {
  const character = resolveCharacter(characterId);
  try {
    (storage ?? globalThis.localStorage)?.setItem?.(CHARACTER_SELECTION_KEY, character.id);
  } catch {
    // Character selection remains usable when storage is unavailable.
  }
  return character.id;
}
