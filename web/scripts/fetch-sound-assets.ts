// Explicit setup only: stages the pinned CC0 sound bundle (#67, 67c) for Vite to serve.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { mkdir, readFile, rename, rm, stat, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { COMBAT_SOUNDS } from "../src/sound-bundle.js";

type SoundBundle = { release: string; tar: string; sha256: string; manifestKey: string };
type BundleManifest = { assets: { key: string; path: string }[] };
type SoundManifest = { clips: { id: string; source: { url?: string } }[] };

const safePath = (path: string) =>
  /^[\w.-]+(\/[\w.-]+)*$/.test(path) && !path.split("/").some((part) => /^\.+$/.test(part));
const absent = (error: NodeJS.ErrnoException) => {
  if (error.code === "ENOENT") return null;
  throw error;
};

// Downloads `bundle.tar`, checks its sha256 pin, and replaces `public/sounds/<release>/` with
// its contents plus `sound-manifest.json` at the root. A matching pin already staged is kept.
export async function prepareSoundAssets({
  web,
  bundle = COMBAT_SOUNDS,
  fetchImpl = fetch,
}: {
  web: string;
  bundle?: SoundBundle;
  fetchImpl?: typeof fetch;
}) {
  if (!safePath(bundle.release) || bundle.release.includes("/"))
    throw new Error("Invalid sound release directory");
  const target = resolve(web, "public", "sounds", bundle.release);
  const staged = await readFile(resolve(target, "SHA256"), "utf8").catch(absent);
  if (staged?.trim() === bundle.sha256) return [];

  const response = await fetchImpl(bundle.tar, { signal: AbortSignal.timeout(120_000) });
  if (!response.ok) throw new Error(`Downloading ${bundle.tar} failed: ${response.status}`);
  const tar = new Uint8Array(await response.arrayBuffer());
  const sha256 = createHash("sha256").update(tar).digest("hex");
  if (sha256 !== bundle.sha256)
    throw new Error(`${bundle.release}.tar sha256 ${sha256} does not match its pin`);

  const stage = `${target}.preparing`;
  await rm(stage, { recursive: true, force: true });
  await mkdir(stage, { recursive: true });
  try {
    await writeFile(`${stage}.tar`, tar);
    execFileSync("tar", ["-xf", `${stage}.tar`, "-C", stage]);
    const manifest = JSON.parse(
      await readFile(resolve(stage, "manifest.json"), "utf8"),
    ) as BundleManifest;
    const asset = manifest.assets.find((entry) => entry.key === bundle.manifestKey);
    if (!asset || !safePath(asset.path))
      throw new Error(`${bundle.release} has no ${bundle.manifestKey}`);
    const soundText = await readFile(resolve(stage, asset.path), "utf8");
    const sounds = JSON.parse(soundText) as SoundManifest;
    for (const clip of sounds.clips) {
      const url = clip.source.url;
      if (url === undefined) continue;
      const info = safePath(url) ? await stat(resolve(stage, url)).catch(absent) : null;
      if (!info?.isFile() || info.size === 0)
        throw new Error(`${bundle.release} clip ${clip.id} is missing ${url}`);
    }
    await writeFile(resolve(stage, "sound-manifest.json"), soundText);
    await writeFile(resolve(stage, "SHA256"), `${bundle.sha256}\n`);
    await rm(target, { recursive: true, force: true });
    await rename(stage, target);
  } finally {
    await rm(stage, { recursive: true, force: true });
    await rm(`${stage}.tar`, { force: true });
  }
  return [bundle.release];
}

if (import.meta.main) {
  const web = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  console.log({ changed: await prepareSoundAssets({ web }) });
}
