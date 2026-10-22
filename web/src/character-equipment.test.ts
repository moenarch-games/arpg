import { expect, test } from "bun:test";
import * as THREE from "three";
import { RANGER_BUNDLE } from "./character-bundles.js";
import { createEquipment } from "./character-equipment.ts";

const sockets = {
  sockets: [
    { id: "hand_r.weapon", joint: "hand_r", translation: [0.1, 0.2, 0.3], rotation: [0, 0, 0, 1] },
    {
      id: "hand_l.shield",
      joint: "hand_l",
      translation: [0, 0, 0],
      rotation: [0, 0.7071068, 0, 0.7071068],
    },
    { id: "hip_l.stowed", joint: "pelvis", translation: [0, 0, 0], rotation: [0, 0, 0, 1] },
    { id: "back.stowed", joint: "spine_03", translation: [0, 0, 0], rotation: [0, 0, 0, 1] },
  ],
};

function items() {
  const made: { id: string; disposed: boolean }[] = [];
  const item = (id: string) => ({
    createInstance() {
      const instance = {
        id,
        disposed: false,
        dispose() {
          this.disposed = true;
        },
      };
      made.push(instance);
      return instance;
    },
  });
  return { made, items: { sword: item("sword"), shield: item("shield") } };
}

function character() {
  const attached: {
    item: any;
    joint: string;
    socket: number[];
    frame: string;
    detached: boolean;
  }[] = [];
  return {
    attached,
    attach(item: any, attachment: { joint: string; socket: number[]; frame: string }) {
      const entry = { item, ...attachment, detached: false };
      attached.push(entry);
      return {
        detach() {
          entry.detached = true;
        },
      };
    },
  };
}

test("loadouts place items on the right-handed sockets by data", () => {
  const { made, items: loaded } = items();
  const equipment = createEquipment({ data: RANGER_BUNDLE.equipment, items: loaded, sockets });
  const ranger = character();
  const unequip = equipment.equip(ranger, "swordAndShield");
  expect(ranger.attached.map((entry) => [entry.item.id, entry.joint, entry.frame])).toEqual([
    ["sword", "hand_r", "grip"],
    ["shield", "hand_l", "grip"],
  ]);
  const translation = new THREE.Vector3().setFromMatrixPosition(
    new THREE.Matrix4().fromArray(ranger.attached[0].socket),
  );
  expect(translation.toArray().map((v) => +v.toFixed(6))).toEqual([0.1, 0.2, 0.3]);
  unequip();
  expect(ranger.attached.every((entry) => entry.detached)).toBe(true);
  expect(made.every((item) => item.disposed)).toBe(true);

  const archer = character();
  equipment.equip(archer, "bow");
  expect(archer.attached.map((entry) => [entry.item.id, entry.joint])).toEqual([
    ["sword", "pelvis"],
    ["shield", "spine_03"],
  ]);
  expect(equipment.equip(character(), "unknownWeapon")).toBeFunction();
});

test("data that names a missing socket, item or place is refused at load", () => {
  const { items: loaded } = items();
  const data = RANGER_BUNDLE.equipment;
  expect(() =>
    createEquipment({ data, items: loaded, sockets: { sockets: sockets.sockets.slice(1) } }),
  ).toThrow(/no socket hand_r.weapon/);
  expect(() =>
    createEquipment({ data: { ...data, handedness: "left" }, items: loaded, sockets }),
  ).toThrow(/no socket hand_l.weapon/);
  expect(() => createEquipment({ data, items: { sword: loaded.sword }, sockets })).toThrow(
    /unknown item shield/,
  );
  expect(() =>
    createEquipment({
      data: { ...data, loadouts: { bow: [{ item: "sword", place: "tail" }] } },
      items: loaded,
      sockets,
    }),
  ).toThrow(/unknown place tail/);
  expect(() =>
    createEquipment({
      data,
      items: loaded,
      sockets: {
        sockets: [{ ...sockets.sockets[0], rotation: [0, 0, NaN, 1] }, ...sockets.sockets.slice(1)],
      },
    }),
  ).toThrow(/invalid transform/);
});
