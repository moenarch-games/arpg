// Downloads the released character bundles once and unpacks them into `public/characters/`,
// which Vite serves and copies into the Pages build. Skips a bundle that is already
// unpacked. Owner decision (arpg#55, Atlas 3.2.1): no checksum, nothing committed, no
// runtime fetch from GitHub.
import { mkdir, rm, stat, writeFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { CHARACTER_BUNDLES } from "../src/character-bundles.js";

const web = resolve(dirname(fileURLToPath(import.meta.url)), "..");

for (const bundle of CHARACTER_BUNDLES) {
  const target = resolve(web, "public", "characters", bundle.release);
  const present = await stat(resolve(target, "manifest.json")).then(
    () => true,
    () => false,
  );
  if (present) continue;
  const response = await fetch(bundle.url);
  if (!response.ok) throw new Error(`Downloading ${bundle.url} failed: ${response.status}`);
  await rm(target, { recursive: true, force: true });
  await mkdir(target, { recursive: true });
  const archive = resolve(target, "..", `${bundle.release}.tar`);
  await writeFile(archive, new Uint8Array(await response.arrayBuffer()));
  const unpacked = spawnSync("tar", ["-xf", archive, "-C", target], { stdio: "inherit" });
  await rm(archive, { force: true });
  if (unpacked.status !== 0) {
    await rm(target, { recursive: true, force: true });
    throw new Error(`Unpacking ${bundle.release} failed`);
  }
  console.log(`Unpacked ${bundle.release} into public/characters/`);
}
