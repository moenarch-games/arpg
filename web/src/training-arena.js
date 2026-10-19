export const TRAINING_SCENARIO = "training";
export const DEFAULT_TRAINING_SEED = 0xa4200916;
export const TRAINING_SPEEDS = Object.freeze([0.25, 0.5, 1, 2]);
// Named workbench scenarios owned by arpg-core (`ScenarioId`); the browser only selects one.
export const DEFAULT_SCENARIO = "dungeon";
export const SCENARIOS = Object.freeze([
  { id: "dungeon", label: "Generated dungeon" },
  { id: "dummy", label: "Target dummy" },
  { id: "enemy", label: "Active enemy" },
  { id: "obstructed", label: "Dummy behind a pillar" },
  { id: "archery", label: "Archery range" },
  { id: "archeryObstructed", label: "Archery behind a pillar" },
  { id: "ranged", label: "Ranged enemy" },
  { id: "heavy", label: "Heavy enemy" },
  { id: "retreating", label: "Retreating enemy in reach" },
  { id: "traversal", label: "Platform to jump onto" },
]);
const SCENARIO_IDS = new Set(SCENARIOS.map((scenario) => scenario.id));

export function parseRunSeed(value) {
  if (value === null || value === undefined || value === "") return null;
  const parsed = Number(value);
  return Number.isInteger(parsed) && parsed >= 0 && parsed <= 0xffff_ffff ? parsed : null;
}

export function readTrainingRequest(search) {
  const params = new URLSearchParams(search);
  const requested = params.get("scenario") === TRAINING_SCENARIO;
  const requestedSeed = parseRunSeed(params.get("seed"));
  const fixture = params.get("fixture");
  return {
    requested,
    seed: requestedSeed ?? DEFAULT_TRAINING_SEED,
    // Unknown names fall back to the plain dungeon instead of breaking startup.
    fixture: fixture !== null && SCENARIO_IDS.has(fixture) ? fixture : DEFAULT_SCENARIO,
    unknownFixture: fixture !== null && !SCENARIO_IDS.has(fixture) ? fixture : null,
  };
}

export function trainingTicksForFrame(speed, carry = 0) {
  if (!Number.isFinite(speed) || speed <= 0) {
    return { ticks: 0, carry: 0 };
  }
  const total = carry + speed;
  const ticks = Math.floor(total);
  return { ticks, carry: total - ticks };
}

export function withTrainingRequest(href, enabled, seed, fixture = DEFAULT_SCENARIO) {
  const url = new URL(href);
  if (enabled) {
    url.searchParams.set("scenario", TRAINING_SCENARIO);
    url.searchParams.set("seed", String(seed));
    if (fixture === DEFAULT_SCENARIO) url.searchParams.delete("fixture");
    else url.searchParams.set("fixture", fixture);
  } else {
    url.searchParams.delete("scenario");
    url.searchParams.delete("fixture");
  }
  return url.toString();
}

// Monster definitions the workbench can spawn: arpg-core content ids. The core validates
// every operation; this list only fills the selector.
export const WORKBENCH_MONSTERS = Object.freeze([
  { id: "monster.brute", label: "Brute" },
  { id: "monster.skirmisher", label: "Skirmisher" },
  { id: "monster.archer", label: "Archer" },
  { id: "monster.bruiser", label: "Bruiser" },
  { id: "monster.defender", label: "Defender" },
]);

// An exact workbench input: a whole number written in plain decimal, or null. Bounds are
// the core's to check, so its validation message names the violated content bound.
export function parseWorkbenchInteger(value) {
  if (typeof value !== "string" || !/^-?\d+$/.test(value.trim())) return null;
  const parsed = Number(value.trim());
  return Number.isSafeInteger(parsed) ? parsed : null;
}

// The spawn operation for the drafted inputs, or the reason it cannot be sent.
export function spawnOperation(definition, offsetX, offsetZ) {
  const x = parseWorkbenchInteger(offsetX);
  const z = parseWorkbenchInteger(offsetZ);
  if (x === null || z === null) return { error: "Offsets must be whole numbers" };
  return { operation: { type: "spawnMonster", definition, offset: [x, z] } };
}

// The tuning operation for the drafted value, or the reason it cannot be sent.
export function tuningOperation(parameter, value) {
  const parsed = parseWorkbenchInteger(value);
  if (parsed === null) return { error: `${parameter}: enter a whole number` };
  return { operation: { type: "setTuning", parameter, value: parsed } };
}
