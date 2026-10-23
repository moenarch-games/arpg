// Manual production-build comparison. Inputs must already be acquired and built:
// bun scripts/measure-character-loading.ts <baseline-web-root> [output-dir]
import { mkdir, writeFile } from "node:fs/promises";
import { resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";
import { preview } from "vite";
const candidate = resolve(fileURLToPath(new URL("../", import.meta.url)));
if (!process.argv[2]) throw new Error("Pass a built baseline web directory");
const baseline = resolve(process.argv[2]);
const output = resolve(process.argv[3] ?? ".cache/character-loading-evidence");
await mkdir(output, { recursive: true });
const servers = [];
const browser = await chromium.launch({
  headless: true,
  ...(process.env.CHROMIUM_EXECUTABLE
    ? { executablePath: process.env.CHROMIUM_EXECUTABLE }
    : { channel: "chromium" }),
  args: ["--enable-unsafe-swiftshader", "--use-gl=angle", "--use-angle=swiftshader"],
});
const results = [];
try {
  for (const root of [baseline, candidate])
    servers.push(
      await preview({
        root,
        configFile: resolve(root, "vite.config.js"),
        preview: { host: "127.0.0.1", port: 0 },
      }),
    );
  for (let pair = 0; pair < 3; pair++) {
    for (const [i, mode] of ["before", "after"].entries()) {
      const context = await browser.newContext({ viewport: { width: 1280, height: 800 } });
      const page = await context.newPage();
      const errors: string[] = [];
      page.on("pageerror", (error) => errors.push(error.message));
      const session = await context.newCDPSession(page);
      await session.send("Network.enable");
      await session.send("Network.emulateNetworkConditions", {
        offline: false,
        latency: 40,
        downloadThroughput: 1_250_000,
        uploadThroughput: 1_250_000,
      });
      const address = servers[i]!.httpServer.address();
      if (!address || typeof address === "string") throw new Error("Preview server has no port");
      const start = performance.now();
      await page.goto(`http://127.0.0.1:${address.port}/arpg/`);
      await page
        .getByRole("status")
        .first()
        .filter({ hasText: /^Ready$/ })
        .waitFor({ timeout: 90_000 });
      const readyMs = performance.now() - start;
      await page.getByRole("button", { name: "Enter World", exact: true }).click();
      await page.locator("canvas.game-canvas").waitFor({ state: "visible" });
      await page.evaluate(
        () =>
          new Promise<void>((resolve) =>
            requestAnimationFrame(() => requestAnimationFrame(() => resolve())),
          ),
      );
      const worldMs = performance.now() - start;
      const resources = await page.evaluate(() =>
        (performance.getEntriesByType("resource") as PerformanceResourceTiming[]).map((entry) => ({
          path: new URL(entry.name).pathname,
          encodedBodyBytes: entry.encodedBodySize,
          transferBytes: entry.transferSize,
          durationMs: entry.duration,
        })),
      );
      const assetBytes = resources
        .filter((resource) => resource.path.includes("/characters/"))
        .reduce((n, resource) => n + resource.encodedBodyBytes, 0);
      const totalBodyBytes = resources.reduce((n, resource) => n + resource.encodedBodyBytes, 0);
      await page
        .locator("canvas.game-canvas")
        .screenshot({ path: resolve(output, `${mode}-${pair + 1}.png`) });
      if (errors.length) throw new Error(errors.join("\n"));
      results.push({
        pair: pair + 1,
        mode,
        readyMs,
        worldMs,
        assetBytes,
        totalBodyBytes,
        transferBytes: resources.reduce((n, r) => n + r.transferBytes, 0),
        resources,
      });
      console.log(
        `${mode} #${pair + 1}: ${assetBytes} asset bytes, ${readyMs.toFixed(0)} ms ready, ${worldMs.toFixed(0)} ms world`,
      );
      await context.close();
    }
  }
  const median = (values: number[]) =>
    [...values].sort((a, b) => a - b)[Math.floor(values.length / 2)];
  const summary = Object.fromEntries(
    ["before", "after"].map((mode) => {
      const samples = results.filter((result) => result.mode === mode);
      return [
        mode,
        Object.fromEntries(
          ["assetBytes", "totalBodyBytes", "transferBytes", "readyMs", "worldMs"].map((key) => [
            key,
            median(samples.map((sample) => sample[key as keyof typeof sample] as number)),
          ]),
        ),
      ];
    }),
  );
  await writeFile(
    resolve(output, "observations.json"),
    JSON.stringify(
      {
        browser: browser.version(),
        baseline,
        candidate,
        serving: "Vite production preview, no HTTP body compression",
        network: { latencyMs: 40, downloadBytesPerSecond: 1_250_000 },
        summary,
        results,
      },
      null,
      2,
    ),
  );
  console.log(JSON.stringify(summary));
} finally {
  await browser.close();
  for (const server of servers) await server.close();
}
