import { expect, test } from "bun:test";
import { createHash } from "node:crypto";
import { cp, mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { COMBAT_EFFECTS_RELEASE, prepareCombatEffects, readTar } from "./fetch-combat-effects.ts";

const web = resolve(import.meta.dir, "..");
const sha256 = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");

// A minimal ustar archive of `files`.
function tar(files: [string, Uint8Array][]) {
  const blocks: Uint8Array[] = [];
  for (const [path, bytes] of files) {
    const header = new Uint8Array(512);
    header.set(new TextEncoder().encode(path), 0);
    header.set(new TextEncoder().encode(bytes.byteLength.toString(8).padStart(11, "0")), 124);
    header[156] = "0".charCodeAt(0);
    blocks.push(header, bytes, new Uint8Array((512 - (bytes.byteLength % 512)) % 512));
  }
  blocks.push(new Uint8Array(1024));
  return new Uint8Array(Buffer.concat(blocks));
}

async function fixture() {
  const root = await mkdtemp(resolve(tmpdir(), "combat-effects-"));
  await cp(resolve(web, "src", "combat-effects"), resolve(root, "src", "combat-effects"), {
    recursive: true,
  });
  const descriptorText = await readFile(
    resolve(root, "src/combat-effects/descriptor.json"),
    "utf8",
  );
  const descriptor = JSON.parse(descriptorText);
  const ref = await readFile(resolve(root, "src/combat-effects/export.ref.json"));
  // Stand-in PNG bytes whose hashes the fixture descriptor declares.
  const pngs = descriptor.effects.map((effect: { key: string }) => {
    const bytes = new TextEncoder().encode(`png:${effect.key}`);
    return [effect, bytes] as const;
  });
  for (const [effect, bytes] of pngs)
    (effect as { png: object }).png = { sha256: sha256(bytes), byteLength: bytes.byteLength };
  await writeFile(resolve(root, "src/combat-effects/descriptor.json"), JSON.stringify(descriptor));
  const archive = tar([
    ...pngs.map(
      ([effect, bytes]) =>
        [`assets/${(effect as { png: { sha256: string } }).png.sha256}.png`, bytes] as [
          string,
          Uint8Array,
        ],
    ),
    ["export.ref.json", new Uint8Array(ref)],
    ["manifest.json", new TextEncoder().encode("{}")],
  ]);
  const release = { ...COMBAT_EFFECTS_RELEASE, sha256: sha256(archive) };
  const requests: string[] = [];
  const fetchImpl = (async (url: string) => {
    requests.push(url);
    if (url === release.tar) return new Response(archive);
    if (url === release.descriptor) return Response.json(descriptor);
    return new Response("missing", { status: 404 });
  }) as typeof fetch;
  return { root, release, requests, fetchImpl, pngs };
}

test("reads ustar archives", () => {
  const files = readTar(tar([["a/b.txt", new TextEncoder().encode("hello")]]));
  expect([...files.keys()]).toEqual(["a/b.txt"]);
  expect(new TextDecoder().decode(files.get("a/b.txt"))).toBe("hello");
});

test("installs the pinned release once and rejects a replaced one", async () => {
  const { root, release, requests, fetchImpl, pngs } = await fixture();
  const quiet = () => {};
  try {
    const options = { web: root, release, fetchImpl, assetTooling: "", log: quiet };
    expect(await prepareCombatEffects(options)).toBe(true);
    for (const [effect, bytes] of pngs) {
      const path = `public/effects/combat-effects/assets/${(effect as { png: { sha256: string } }).png.sha256}.png`;
      expect(new Uint8Array(await readFile(resolve(root, path)))).toEqual(bytes);
    }
    expect(await prepareCombatEffects(options)).toBe(false);
    expect(requests).toHaveLength(2);

    const replaced = { ...release, sha256: "0".repeat(64) };
    await expect(prepareCombatEffects({ ...options, release: replaced })).rejects.toThrow(
      "pinned SHA-256",
    );
    // The verified install stays in place.
    expect(await prepareCombatEffects(options)).toBe(false);
  } finally {
    await rm(root, { recursive: true, force: true });
  }
});
