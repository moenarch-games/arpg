import { defineConfig } from "@playwright/test";

export default defineConfig({
  testDir: "./tests/browser",
  fullyParallel: true,
  timeout: 60_000,
  expect: { timeout: 15_000 },
  retries: 0,
  use: {
    // Full Chromium (new headless mode) instead of the default
    // chromium-headless-shell, whose renderer segfaults (SEGV_ACCERR in JIT
    // frames) on pages that run WASM with software WebGL (#133).
    channel: "chromium",
    viewport: { width: 1280, height: 800 },
    trace: "retain-on-failure",
    screenshot: "only-on-failure",
  },
});
