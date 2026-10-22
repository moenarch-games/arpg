// Equipment presentation (#183, Atlas 3.2.3): which item shows where is data. Items name
// bundle entries and the item frame that holds them; places name hand-agnostic positions
// (main hand, off hand, stowed); a handedness table maps places to the character's socket
// ids; loadouts, keyed by the authoritative weapon, list item → place. New items, places,
// handedness or multi-grip items only add data. 3d-lab parents items under joints.

import * as THREE from "three";

export interface EquipmentData {
  // Item id → bundle entry key and the item frame that lands on the socket.
  items: Readonly<Record<string, { key: string; frame: string }>>;
  // Handedness → place → socket id of the character's humanoid-sockets descriptor.
  places: Readonly<Record<string, Readonly<Record<string, string>>>>;
  handedness: string;
  // Authoritative weapon (snapshot `player.weapon`) → items and where they go.
  loadouts: Readonly<Record<string, readonly { item: string; place: string }[]>>;
}

interface SocketDescriptor {
  sockets: readonly {
    id: string;
    joint: string;
    translation: readonly number[];
    rotation: readonly number[];
    scale?: readonly number[];
  }[];
}

export interface ItemAssetLike<Instance> {
  createInstance(): Instance;
}

export interface AttachableInstance<Instance> {
  attach(
    item: Instance,
    attachment: { joint: string; socket: number[]; frame: string },
  ): { detach(): void };
}

const finite = (values: readonly number[] | undefined, length: number) =>
  Array.isArray(values) && values.length === length && values.every(Number.isFinite);

/** Validates the data against the loaded items and sockets, then equips instances by weapon. */
export function createEquipment<Item>(options: {
  data: EquipmentData;
  items: Readonly<Record<string, ItemAssetLike<Item & { dispose(): void }>>>;
  sockets: SocketDescriptor;
}) {
  const { data } = options;
  const places = data.places[data.handedness];
  if (!places) throw new Error(`No equipment places for handedness ${data.handedness}`);
  const sockets = new Map<string, { joint: string; matrix: number[] }>();
  for (const socket of options.sockets.sockets ?? []) {
    if (
      !finite(socket.translation, 3) ||
      !finite(socket.rotation, 4) ||
      (socket.scale && !finite(socket.scale, 3))
    )
      throw new Error(`Socket ${socket.id} has an invalid transform`);
    const matrix = new THREE.Matrix4().compose(
      new THREE.Vector3(...socket.translation),
      new THREE.Quaternion(...socket.rotation),
      new THREE.Vector3(...(socket.scale ?? [1, 1, 1])),
    );
    sockets.set(socket.id, { joint: socket.joint, matrix: matrix.toArray() });
  }
  for (const [weapon, entries] of Object.entries(data.loadouts)) {
    for (const { item, place } of entries) {
      if (!options.items[item]) throw new Error(`Loadout ${weapon} uses unknown item ${item}`);
      const socket = places[place];
      if (!socket) throw new Error(`Loadout ${weapon} uses unknown place ${place}`);
      if (!sockets.has(socket)) throw new Error(`The character has no socket ${socket}`);
    }
  }

  return {
    /** Attaches the weapon's loadout to a character instance; the result detaches it again. */
    equip(instance: AttachableInstance<Item>, weapon: string) {
      const attached = (data.loadouts[weapon] ?? []).map(({ item, place }) => {
        const socket = sockets.get(places[place])!;
        const itemInstance = options.items[item].createInstance();
        const handle = instance.attach(itemInstance, {
          joint: socket.joint,
          socket: socket.matrix,
          frame: data.items[item].frame,
        });
        return () => {
          handle.detach();
          itemInstance.dispose();
        };
      });
      return () => {
        for (const detach of attached) detach();
      };
    },
  };
}

export type Equipment = ReturnType<typeof createEquipment>;
