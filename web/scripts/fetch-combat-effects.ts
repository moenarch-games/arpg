// Explicit setup only: acquires the pinned asset-tooling `combat-effects` release (#67 67b,
// asset-tooling#362) into `public/effects/combat-effects/`. The release is rolling, so it is
// pinned by its tar's SHA-256 and the game's own copies of its `export.ref.json` and
// descriptor (`src/combat-effects/`, asset-tooling docs/asset-bundles.md): a replaced
// release fails here. With `ASSET_TOOLING_ROOT` set to a checkout of the pinned asset-tooling
// commit (as CI does), the tar is also verified by its own `pack-asset-bundle.ts --verify`.
import { spawnSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, mkdtemp, readFile, rename, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { isDeepStrictEqual } from "node:util";

export const COMBAT_EFFECTS_RELEASE = Object.freeze({
  release: "combat-effects",
  tar: "https://github.com/moritzbrantner/asset-tooling/releases/download/combat-effects/combat-effects.tar",
  descriptor:
    "https://github.com/moritzbrantner/asset-tooling/releases/download/combat-effects/combat-effects.descriptor.json",
  sha256: "e02110b26d2a01a13b2bad3500537511408aa4278492cc226b292cf5babc68f4",
  // The asset-tooling commit the release was published from.
  assetTooling: "bde21cfa02b44187b347762266be728922e8a650",
});

type Release = typeof COMBAT_EFFECTS_RELEASE;
type Descriptor = { effects: { key: string; png: { sha256: string; byteLength: number } }[] };

const sha256 = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
const absent = (error: NodeJS.ErrnoException) => {
  if (error.code === "ENOENT") return null;
  throw error;
};

// The regular files of a ustar archive, by path.
export function readTar(tar: Uint8Array) {
  const files = new Map<string, Uint8Array>();
  const text = (from: number, length: number) => {
    const field = tar.subarray(from, from + length);
    const end = field.indexOf(0);
    return new TextDecoder().decode(end < 0 ? field : field.subarray(0, end));
  };
  for (let offset = 0; offset + 512 <= tar.length;) {
    if (tar.subarray(offset, offset + 512).every((byte) => byte === 0)) break;
    const prefix = text(offset + 345, 155);
    const path = prefix ? `${prefix}/${text(offset, 100)}` : text(offset, 100);
    const size = Number.parseInt(text(offset + 124, 12).trim() || "0", 8);
    const type = text(offset + 156, 1);
    if (!Number.isFinite(size) || offset + 512 + size > tar.length)
      throw new Error("Truncated release tar");
    if (type !== "0" && type !== "") throw new Error(`Release tar holds a non-file entry ${path}`);
    files.set(path, tar.slice(offset + 512, offset + 512 + size));
    offset += 512 + Math.ceil(size / 512) * 512;
  }
  return files;
}

async function verifyWithAssetTooling(
  root: string,
  release: Release,
  tar: Uint8Array,
  ref: string,
  log: (message: string) => void,
) {
  const head = spawnSync("git", ["-C", root, "rev-parse", "HEAD"], {
    encoding: "utf8",
  }).stdout?.trim();
  if (head !== release.assetTooling)
    throw new Error(`ASSET_TOOLING_ROOT is at ${head || "?"}, not ${release.assetTooling}`);
  const work = await mkdtemp(resolve(tmpdir(), "combat-effects-verify-"));
  try {
    const file = resolve(work, "combat-effects.tar");
    await writeFile(file, tar);
    await writeFile(`${file}.sha256`, `${release.sha256}  combat-effects.tar\n`);
    const check = spawnSync(
      process.execPath,
      [resolve(root, "scripts/pack-asset-bundle.ts"), "--verify", file, "--ref", ref],
      { cwd: root, encoding: "utf8" },
    );
    if (check.status !== 0)
      throw new Error(`pack-asset-bundle --verify failed: ${(check.stderr ?? "").trim()}`);
    log(`combat-effects: ${check.stdout.trim()}`);
  } finally {
    await rm(work, { recursive: true, force: true });
  }
}

export async function prepareCombatEffects({
  web,
  release = COMBAT_EFFECTS_RELEASE,
  fetchImpl = fetch,
  assetTooling = process.env.ASSET_TOOLING_ROOT,
  log = console.log,
}: {
  web: string;
  release?: Release;
  fetchImpl?: typeof fetch;
  assetTooling?: string;
  log?: (message: string) => void;
}) {
  const target = resolve(web, "public", "effects", "combat-effects");
  const pinnedRefPath = resolve(web, "src", "combat-effects", "export.ref.json");
  const pinnedRef = JSON.parse(await readFile(pinnedRefPath, "utf8"));
  const descriptor = JSON.parse(
    await readFile(resolve(web, "src", "combat-effects", "descriptor.json"), "utf8"),
  ) as Descriptor;
  const pngs = descriptor.effects.map((effect) => ({
    path: `assets/${effect.png.sha256}.png`,
    ...effect.png,
  }));

  const installed = await readFile(resolve(target, "SHA256"), "utf8").catch(absent);
  if (installed?.trim() === release.sha256) {
    const present = await Promise.all(
      pngs.map(async ({ path, byteLength }) => {
        const info = await stat(resolve(target, path)).catch(absent);
        return info?.isFile() && info.size === byteLength;
      }),
    );
    if (present.every(Boolean)) return false;
  }

  const request = async (url: string) => {
    const response = await fetchImpl(url, { signal: AbortSignal.timeout(120_000) });
    if (!response.ok) throw new Error(`Downloading ${url} failed: ${response.status}`);
    return new Uint8Array(await response.arrayBuffer());
  };
  const [tar, releaseDescriptor] = await Promise.all([
    request(release.tar),
    request(release.descriptor).then((bytes) => JSON.parse(new TextDecoder().decode(bytes))),
  ]);
  if (sha256(tar) !== release.sha256)
    throw new Error("The combat-effects tar does not match its pinned SHA-256");
  if (!isDeepStrictEqual(releaseDescriptor, descriptor))
    throw new Error("The combat-effects descriptor no longer matches the pinned copy");
  const files = readTar(tar);
  const ref = files.get("export.ref.json");
  if (!ref || !isDeepStrictEqual(JSON.parse(new TextDecoder().decode(ref)), pinnedRef))
    throw new Error("The combat-effects export.ref.json does not match the pinned copy");
  for (const { path, sha256: expected, byteLength } of pngs) {
    const bytes = files.get(path);
    if (!bytes || bytes.byteLength !== byteLength || sha256(bytes) !== expected)
      throw new Error(`The combat-effects tar has no matching ${path}`);
  }
  if (assetTooling) await verifyWithAssetTooling(assetTooling, release, tar, pinnedRefPath, log);
  else
    log(
      `combat-effects: set ASSET_TOOLING_ROOT to asset-tooling@${release.assetTooling} for the full bundle verification`,
    );

  const stage = `${target}.preparing`;
  await rm(stage, { recursive: true, force: true });
  try {
    await mkdir(resolve(stage, "assets"), { recursive: true });
    for (const { path } of pngs) await writeFile(resolve(stage, path), files.get(path)!);
    await writeFile(resolve(stage, "SHA256"), `${release.sha256}\n`);
    await rm(target, { recursive: true, force: true });
    await rename(stage, target);
  } finally {
    await rm(stage, { recursive: true, force: true });
  }
  return true;
}

if (import.meta.main) {
  const web = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  console.log({ changed: await prepareCombatEffects({ web }) });
}
