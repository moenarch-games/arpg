import { expect, test } from "bun:test";
import { mkdtemp, mkdir, readFile, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { prepareCharacterAssets } from "./fetch-character-assets.ts";

const bundle = {
  release: "ranger",
  index: "https://assets.test/ranger.index.json",
  ref: "https://assets.test/export.ref.json",
  key: "ranger",
  variant: "default",
  equipment: { items: { sword: { key: "sword" } } },
};
const index = {
  schema: "asset-downloads-v1",
  entries: [
    {
      key: "ranger",
      variant: "default",
      files: ["ranger.gltf", "ranger.bin", "ranger.default.humanoid-sockets.json"],
      textures: ["shared.ktx2"],
      clips: "ranger.clips.glb",
    },
    {
      key: "sword",
      variant: "default",
      files: ["sword.gltf", "sword.bin"],
      textures: ["shared.ktx2"],
    },
    { key: "ranger", variant: "knight", files: ["knight.gltf"], textures: [] },
  ],
};
async function fixture() {
  const web = await mkdtemp(resolve(tmpdir(), "ranger-downloads-"));
  const basis = resolve(web, "node_modules/three/examples/jsm/libs/basis");
  await mkdir(basis, { recursive: true });
  for (const file of ["basis_transcoder.js", "basis_transcoder.wasm"])
    await writeFile(resolve(basis, file), "fixture-decoder");
  const requests: string[] = [];
  let fail = "";
  let revision = "one";
  const fetchImpl = (async (url: string) => {
    const path = new URL(url).pathname.slice(1);
    requests.push(path);
    if (path === fail) return new Response("missing", { status: 404 });
    if (path === "ranger.index.json") return Response.json(index);
    if (path === "export.ref.json") return Response.json({ sha256: revision });
    return new Response(path);
  }) as typeof fetch;
  return {
    web,
    requests,
    fetchImpl,
    fail: (path: string) => {
      fail = path;
    },
    revision: (value: string) => {
      revision = value;
    },
    close: () => rm(web, { recursive: true, force: true }),
  };
}
test("setup fetches only default/equipment, deduplicates textures, then performs zero writes", async () => {
  const f = await fixture();
  try {
    const args = { web: f.web, bundles: [bundle], fetchImpl: f.fetchImpl };
    expect(await prepareCharacterAssets(args)).toEqual([
      "ranger",
      "basis_transcoder.js",
      "basis_transcoder.wasm",
    ]);
    expect(f.requests.filter((path) => path === "shared.ktx2")).toHaveLength(1);
    expect(f.requests).not.toContain("knight.gltf");
    expect(f.requests.some((path) => path.endsWith(".tar"))).toBe(false);
    const path = resolve(f.web, "public/characters/ranger/ranger.bin");
    const before = (await stat(path)).mtimeMs;
    f.requests.length = 0;
    expect(await prepareCharacterAssets(args)).toEqual([]);
    expect((await stat(path)).mtimeMs).toBe(before);
    expect(f.requests.sort()).toEqual(["export.ref.json", "ranger.index.json"]);
    await rm(path);
    f.requests.length = 0;
    expect(await prepareCharacterAssets(args)).toEqual(["ranger"]);
    expect(f.requests.sort()).toEqual(["export.ref.json", "ranger.bin", "ranger.index.json"]);
  } finally {
    await f.close();
  }
});
test("a failed replacement preserves the previous complete release and retry recovers", async () => {
  const f = await fixture();
  try {
    const args = { web: f.web, bundles: [bundle], fetchImpl: f.fetchImpl };
    await prepareCharacterAssets(args);
    f.revision("two");
    f.fail("ranger.bin");
    await expect(prepareCharacterAssets(args)).rejects.toThrow(/404/);
    const ref = resolve(f.web, "public/characters/ranger/export.ref.json");
    expect(JSON.parse(await readFile(ref, "utf8")).sha256).toBe("one");
    f.fail("");
    await prepareCharacterAssets(args);
    expect(JSON.parse(await readFile(ref, "utf8")).sha256).toBe("two");
  } finally {
    await f.close();
  }
});
test("items named by their own release download from it beside the character", async () => {
  const f = await fixture();
  const equipmentIndex = {
    schema: "asset-downloads-v1",
    entries: [
      { key: "sword", variant: "default", files: ["sword.gltf", "sword.bin"], textures: [] },
    ],
  };
  const requests: string[] = [];
  let equipmentRevision = "e1";
  const fetchImpl = (async (url: string) => {
    const { host, pathname } = new URL(url);
    if (host === "equipment.test") {
      requests.push(`equipment:${pathname.slice(1)}`);
      if (pathname === "/equipment.index.json") return Response.json(equipmentIndex);
      if (pathname === "/export.ref.json") return Response.json({ sha256: equipmentRevision });
      return new Response(`equipment ${pathname}`);
    }
    return f.fetchImpl(url);
  }) as typeof fetch;
  const split = {
    ...bundle,
    equipment: {
      ...bundle.equipment,
      index: "https://equipment.test/equipment.index.json",
      ref: "https://equipment.test/export.ref.json",
    },
  };
  try {
    const args = { web: f.web, bundles: [split], fetchImpl };
    await prepareCharacterAssets(args);
    const target = resolve(f.web, "public/characters/ranger");
    expect(await readFile(resolve(target, "sword.bin"), "utf8")).toBe("equipment /sword.bin");
    expect(f.requests).not.toContain("sword.bin");
    const manifest = JSON.parse(await readFile(resolve(target, "manifest.json"), "utf8"));
    expect(manifest.entries.map((entry: { key: string }) => entry.key)).toEqual([
      "ranger",
      "sword",
    ]);
    // A newer equipment upload refreshes the copy; an unchanged one writes nothing.
    expect(await prepareCharacterAssets(args)).toEqual([]);
    equipmentRevision = "e2";
    expect(await prepareCharacterAssets(args)).toEqual(["ranger"]);
    expect(requests).toContain("equipment:sword.gltf");
  } finally {
    await f.close();
  }
});
