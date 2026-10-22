// Downloads the released character bundles and unpacks them into `public/characters/`,
// which Vite serves and copies into the Pages build. Releases are rolling, so a bundle is
// downloaded again only when the release's `export.ref.json` differs from the unpacked
// copy's. Owner decision (arpg#55, Atlas 3.2.1): no checksum, nothing committed, no
// runtime fetch from GitHub.
import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { CHARACTER_BUNDLES } from "../src/character-bundles.js";

const web = resolve(dirname(fileURLToPath(import.meta.url)), "..");

for (const bundle of CHARACTER_BUNDLES) {
  const target = resolve(web, "public", "characters", bundle.release);
  const refResponse = await fetch(bundle.ref);
  if (!refResponse.ok) throw new Error(`Downloading ${bundle.ref} failed: ${refResponse.status}`);
  const { sha256 } = await refResponse.json();
  const current = await readFile(resolve(target, "export.ref.json"), "utf8").then(
    (text) => JSON.parse(text).sha256,
    () => null,
  );
  if (current === sha256) continue;
  const response = await fetch(bundle.url);
  if (!response.ok) throw new Error(`Downloading ${bundle.url} failed: ${response.status}`);
  await rm(target, { recursive: true, force: true });
  await mkdir(target, { recursive: true });
  const archive = resolve(target, "..", `${bundle.release}.tar`);
  await writeFile(archive, new Uint8Array(await response.arrayBuffer()));
  const extracted = spawnSync("tar", ["-xf", archive, "-C", target], { stdio: "inherit" });
  await rm(archive, { force: true });
  if (extracted.status !== 0) {
    await rm(target, { recursive: true, force: true });
    throw new Error(`Unpacking ${bundle.release} failed`);
  }
  console.log(`Unpacked ${bundle.release} into public/characters/`);
}
