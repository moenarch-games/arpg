import { expect, test } from "bun:test";
import { createCharacterAssets } from "./character-assets.ts";

const bundle = {
  release: "humanoid-ranger-male-v1",
  key: "humanoid.ranger.male",
  variant: "default",
};
const manifest = { profile: "gltf-resources-v1" };

function fixture({ failFile = null as string | null } = {}) {
  const requested: string[] = [];
  const adapted: unknown[] = [];
  const assets = createCharacterAssets({
    bundle,
    baseUrl: "/arpg/characters/",
    fetch: async (url) => {
      requested.push(url);
      const ok = !failFile || !url.endsWith(failFile);
      return {
        ok,
        status: ok ? 200 : 404,
        json: async () => manifest,
        arrayBuffer: async () => new ArrayBuffer(4),
      };
    },
    adapt: async (source, { onProgress }) => {
      onProgress({ loadedBytes: 0, totalBytes: 8 });
      await source.readResource("resources/a.bin");
      onProgress({ loadedBytes: 4, totalBytes: 8 });
      await source.readResource("resources/b.png");
      onProgress({ loadedBytes: 8, totalBytes: 8 });
      const asset = { source };
      adapted.push(asset);
      return asset;
    },
  });
  return { assets, requested, adapted };
}

test("the bundle loads once from the release directory with byte progress", async () => {
  const { assets, requested, adapted } = fixture();
  const seen: unknown[] = [];
  assets.subscribe(() => seen.push(assets.getProgress()));
  const first = assets.load();
  expect(assets.load()).toBe(first);
  const asset = await first;
  expect(requested).toEqual([
    "/arpg/characters/humanoid-ranger-male-v1/manifest.json",
    "/arpg/characters/humanoid-ranger-male-v1/resources/a.bin",
    "/arpg/characters/humanoid-ranger-male-v1/resources/b.png",
  ]);
  expect(seen).toContainEqual({ loadedBytes: 4, totalBytes: 8 });
  expect(assets.getProgress()).toBeNull();
  expect(assets.get()).toBe(asset);
  expect(await assets.load()).toBe(asset);
  expect(adapted).toHaveLength(1);
  const source = (asset as { source: { key: string; variant: string; manifest: unknown } }).source;
  expect([source.key, source.variant, source.manifest]).toEqual([
    bundle.key,
    bundle.variant,
    manifest,
  ]);
});

test("a missing file fails visibly and a retry loads again", async () => {
  const broken = fixture({ failFile: "b.png" });
  await expect(broken.assets.load()).rejects.toThrow(
    /Character assets failed to load: .*b.png.*404/,
  );
  expect(broken.assets.get()).toBeNull();
  expect(broken.assets.getProgress()).toBeNull();
  await expect(broken.assets.load()).rejects.toThrow(/b.png/);
  expect(broken.requested.filter((url) => url.endsWith("manifest.json"))).toHaveLength(2);
});
