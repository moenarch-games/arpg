// Explicit setup only. Runtime and Vite builds use the acquired per-entry files.
import { mkdir, readFile, rename, rm, stat, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { CHARACTER_BUNDLES } from "../src/character-bundles.js";

type Bundle = {
  release: string;
  index: string;
  ref: string;
  key: string;
  variant: string;
  equipment: { items: Record<string, { key: string }> };
};
type DownloadEntry = {
  key: string;
  variant: string;
  files: string[];
  textures: string[];
  clips?: string;
};
type DownloadIndex = { schema: string; entries: DownloadEntry[] };
const safeFile = (path: string) => /^[\w.-]+$/.test(path) && path !== "." && path !== "..";
const absent = (error: NodeJS.ErrnoException) => {
  if (error.code === "ENOENT") return null;
  throw error;
};

export async function prepareCharacterAssets({
  web,
  bundles = CHARACTER_BUNDLES,
  fetchImpl = fetch,
}: {
  web: string;
  bundles?: readonly Bundle[];
  fetchImpl?: typeof fetch;
}) {
  const changes: string[] = [];
  const request = async (url: string) => {
    const response = await fetchImpl(url, { signal: AbortSignal.timeout(120_000) });
    if (!response.ok) throw new Error(`Downloading ${url} failed: ${response.status}`);
    return response;
  };
  for (const bundle of bundles) {
    if (!safeFile(bundle.release)) throw new Error("Invalid character release directory");
    const target = resolve(web, "public", "characters", bundle.release);
    const [indexText, refText] = await Promise.all([
      request(bundle.index).then((r) => r.text()),
      request(bundle.ref).then((r) => r.text()),
    ]);
    const index = JSON.parse(indexText) as DownloadIndex;
    const ref = JSON.parse(refText) as { sha256: string };
    if (
      index.schema !== "asset-downloads-v1" ||
      !Array.isArray(index.entries) ||
      typeof ref.sha256 !== "string"
    )
      throw new Error("Invalid rolling release metadata");
    const selected = [
      [bundle.key, bundle.variant],
      ...Object.values(bundle.equipment.items).map((item) => [item.key, "default"]),
    ];
    const entries = selected.map(([key, variant]) => {
      const matches = index.entries.filter(
        (entry) => entry.key === key && entry.variant === variant,
      );
      if (matches.length !== 1) throw new Error(`Release has no unique ${key}/${variant}`);
      return matches[0]!;
    });
    const files = [
      ...new Set(
        entries.flatMap((entry) => [
          ...entry.files,
          ...entry.textures,
          ...(entry.clips ? [entry.clips] : []),
        ]),
      ),
    ].sort();
    if (!files.every(safeFile)) throw new Error("Release contains an unsafe filename");
    const manifestText = JSON.stringify({ ...index, entries }) + "\n";
    const current = await readFile(resolve(target, "manifest.json"), "utf8").catch(absent);
    const currentRef = await readFile(resolve(target, "export.ref.json"), "utf8").catch(absent);
    const same = current === manifestText && currentRef === refText;
    const missing = same
      ? (
          await Promise.all(
            files.map(async (path) => {
              const info = await stat(resolve(target, path)).catch(absent);
              return info?.isFile() && info.size > 0 ? null : path;
            }),
          )
        ).filter((path): path is string => path !== null)
      : files;
    if (same && missing.length === 0) continue;
    const stage = `${target}.preparing`;
    await rm(stage, { recursive: true, force: true });
    await mkdir(stage, { recursive: true });
    try {
      let next = 0;
      const outcomes = await Promise.allSettled(
        Array.from({ length: Math.min(6, missing.length) }, async () => {
          while (next < missing.length) {
            const path = missing[next++]!;
            const bytes = new Uint8Array(
              await (await request(new URL(path, bundle.index).href)).arrayBuffer(),
            );
            if (!bytes.byteLength) throw new Error(`Empty character file ${path}`);
            await writeFile(resolve(stage, path), bytes);
          }
        }),
      );
      const failed = outcomes.find((outcome) => outcome.status === "rejected");
      if (failed?.status === "rejected") throw failed.reason;
      if (same) {
        for (const path of missing) await rename(resolve(stage, path), resolve(target, path));
      } else {
        await writeFile(resolve(stage, "manifest.json"), manifestText);
        await writeFile(resolve(stage, "export.ref.json"), refText);
        const previous = `${target}.previous`;
        await rm(previous, { recursive: true, force: true });
        const present = await stat(target).catch(absent);
        if (present) await rename(target, previous);
        try {
          await rename(stage, target);
        } catch (error) {
          if (present) await rename(previous, target);
          throw error;
        }
        await rm(previous, { recursive: true, force: true });
      }
      changes.push(bundle.release);
    } finally {
      await rm(stage, { recursive: true, force: true });
    }
  }
  // Matching locally installed Three.js decoder files; no CDN or build-time fetch.
  const basis = resolve(web, "node_modules", "three", "examples", "jsm", "libs", "basis");
  const output = resolve(web, "public", "decoders", "basis");
  for (const file of ["basis_transcoder.js", "basis_transcoder.wasm"]) {
    const bytes = await readFile(resolve(basis, file));
    const current = await readFile(resolve(output, file)).catch(absent);
    if (current?.equals(bytes)) continue;
    await mkdir(output, { recursive: true });
    await writeFile(resolve(output, `${file}.tmp`), bytes);
    await rename(resolve(output, `${file}.tmp`), resolve(output, file));
    changes.push(file);
  }
  return changes;
}
if (import.meta.main) {
  const web = resolve(dirname(fileURLToPath(import.meta.url)), "..");
  console.log({ changed: await prepareCharacterAssets({ web }) });
}
