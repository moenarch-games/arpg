import { expect, test } from "bun:test";
import { mkdtemp, mkdir, rm, stat, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { resolve } from "node:path";
import { prepareSoundAssets } from "./fetch-sound-assets.ts";

const bundle = {
  release: "sounds",
  tar: "https://assets.test/sounds.tar",
  sha256: "0".repeat(64),
  manifestKey: "sounds.sound-manifest",
};

test("a release that does not match its sha256 pin is never staged", async () => {
  const web = await mkdtemp(resolve(tmpdir(), "sound-downloads-"));
  try {
    const fetchImpl = (async () => new Response("replaced release")) as unknown as typeof fetch;
    await expect(prepareSoundAssets({ web, bundle, fetchImpl })).rejects.toThrow(
      "does not match its pin",
    );
    expect(await stat(resolve(web, "public/sounds/sounds")).catch(() => null)).toBeNull();
  } finally {
    await rm(web, { recursive: true, force: true });
  }
});

test("an already staged pin is kept without downloading", async () => {
  const web = await mkdtemp(resolve(tmpdir(), "sound-downloads-"));
  try {
    await mkdir(resolve(web, "public/sounds/sounds"), { recursive: true });
    await writeFile(resolve(web, "public/sounds/sounds/SHA256"), `${bundle.sha256}\n`);
    const fetchImpl = (async () => {
      throw new Error("no download expected");
    }) as unknown as typeof fetch;
    expect(await prepareSoundAssets({ web, bundle, fetchImpl })).toEqual([]);
  } finally {
    await rm(web, { recursive: true, force: true });
  }
});
