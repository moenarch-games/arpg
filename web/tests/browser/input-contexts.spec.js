// Acceptance contract for #176 (input contexts, capture priority and safe release of held
// inputs), run against the production build in `web/dist` (`bun run build` first).
//
// Written independently of the implementation against baseline ea15df2. The local
// training arena is the observable authority: its "Export reproduction" file lists every
// command the Rust/Wasm authority accepted, so each test reads what the player's input
// actually did to the game, not component state.
//
// Assumptions:
// - Window blur and a hidden page are produced by dispatching the browser's own `blur` and
//   `visibilitychange` events (with `document.visibilityState` reporting "hidden"). Playwright
//   emulates focus for every page, so real focus loss does not reach a headless page.
// - Lost pointer capture: the control really holds the touch capture, but the test dispatches
//   the `lostpointercapture` event itself, because Chromium fires none for touch when script
//   releases or moves the capture. pointercancel is a real CDP `touchCancel`.
// - The dedicated disconnect uses the real native `arpg-game-server` binary (build it with
//   `cargo build --locked -p arpg-game-server --bin server`), owned by the test like
//   `dedicated.spec.js`. The outage is made permanent at the WebTransport boundary until the
//   client's own reconnect grace (600 ticks at 60 Hz, 10 s) expires and the client reports
//   "Dedicated authority disconnected". Nothing about the disconnect itself is faked.
// - Dedicated play has no command export, so the disconnect journey observes the held
//   shield (`data-guard` on "Hold shield", from authority snapshots) and the status line.
//   Movement after a dedicated reconnect is covered by `src/input-context-release.test.ts`.
import { spawn, execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { once } from "node:events";
import { createSocket } from "node:dgram";
import { existsSync, mkdtempSync, rmSync } from "node:fs";
import { readFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";

import { test as base, expect } from "@playwright/test";
import { preview } from "vite";

const webRoot = resolve(import.meta.dirname, "../..");
const repositoryRoot = resolve(webRoot, "..");

const test = base.extend({
  page: async ({ page }, use) => {
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    await use(page);
    expect(errors).toEqual([]);
  },
  // The production build, served as the Pages deployment serves it (base `/arpg/`).
  appUrl: [
    // Playwright requires a destructuring pattern for fixtures, even with no dependencies.
    // oxlint-disable-next-line no-empty-pattern
    async ({}, use) => {
      expect(
        existsSync(join(webRoot, "dist", "index.html")),
        "build the production bundle first: bun run build",
      ).toBe(true);
      const server = await preview({
        root: webRoot,
        logLevel: "silent",
        preview: { host: "127.0.0.1", port: 0, strictPort: false, open: false },
      });
      try {
        await use(server.resolvedUrls.local[0]);
      } finally {
        await server.close();
      }
    },
    { scope: "worker" },
  ],
});

// ---------------------------------------------------------------------------------------
// Helpers.

// Clicks without moving focus, so a focused text field or recorder keeps its focus.
const activate = (locator) => locator.dispatchEvent("click");

async function openArena(page, appUrl, query = "") {
  await page.goto(`${appUrl}?scenario=training&seed=42${query}`);
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  await page.evaluate(() => document.activeElement?.blur());
  await activate(page.getByRole("button", { name: "Step", exact: true }));
}

// Every command the authority accepted so far.
async function acceptedCommands(page) {
  await activate(page.getByRole("button", { name: "Step", exact: true }));
  const download = page.waitForEvent("download");
  await activate(page.getByRole("button", { name: "Export reproduction", exact: true }));
  const file = await download;
  const replay = JSON.parse(await readFile(await file.path(), "utf8"));
  return replay.commands.map((entry) => entry.command);
}

const movement = (commands) =>
  commands.filter((command) => command.type === "setMovement").map((c) => `${c.x},${c.z}`);
const guard = (commands) =>
  commands.filter((command) => command.type === "setGuard").map((c) => c.raised);
const aims = (commands) =>
  commands.filter((command) => command.type === "setAim").map((c) => c.direction);
const ONE_SHOTS = new Set([
  "primaryAttack",
  "secondaryAttack",
  "interact",
  "cycleTarget",
  "clearTarget",
  "equipWeapon",
]);
const oneShots = (commands) => commands.filter((command) => ONE_SHOTS.has(command.type));
const bow = (commands) =>
  commands
    .filter((command) => ["drawBow", "releaseBow", "cancelBow"].includes(command.type))
    .map((command) => command.type);

async function blurWindow(page) {
  await page.evaluate(() => window.dispatchEvent(new Event("blur")));
}

async function setPageHidden(page, hidden) {
  await page.evaluate((isHidden) => {
    Object.defineProperty(document, "visibilityState", {
      configurable: true,
      get: () => (isHidden ? "hidden" : "visible"),
    });
    Object.defineProperty(document, "hidden", { configurable: true, get: () => isHidden });
    document.dispatchEvent(new Event("visibilitychange"));
  }, hidden);
}

// ---------------------------------------------------------------------------------------
// Text entry and rebinding capture while gameplay keys are held.

test("focusing a text field mid-hold releases held keys once and typing never plays", async ({
  page,
  appUrl,
}) => {
  await openArena(page, appUrl);
  await page.keyboard.down("w");
  await page.keyboard.down("f");
  const seed = page.getByRole("spinbutton", { name: "Seed" });

  // The player clicks the seed field while still holding forward and the shield, then types
  // keys that are also gameplay bindings (move right, heavy, primary, interact).
  await seed.click();
  await page.keyboard.press("d");
  await page.keyboard.press("q");
  await page.keyboard.press("Space");
  await page.keyboard.press("e");
  await page.keyboard.type("7");
  await expect(seed).toBeFocused();

  const whileTyping = await acceptedCommands(page);
  expect(movement(whileTyping)).toEqual(["0,-1", "0,0"]);
  expect(guard(whileTyping)).toEqual([true, false]);
  expect(oneShots(whileTyping)).toEqual([]);

  // The keys come up in the field and focus returns to the game: nothing is released twice
  // and nothing resumes on its own.
  await page.keyboard.up("w");
  await page.keyboard.up("f");
  await page.evaluate(() => document.activeElement?.blur());
  const after = await acceptedCommands(page);
  expect(movement(after)).toEqual(["0,-1", "0,0"]);
  expect(guard(after)).toEqual([true, false]);
  expect(oneShots(after)).toEqual([]);
});

test("rebinding capture mid-hold records keys without moving, attacking or closing the menu", async ({
  page,
  appUrl,
}) => {
  await openArena(page, appUrl);
  await page.keyboard.down("w");
  await page.keyboard.down("f");

  await page.getByRole("button", { name: "Settings", exact: true }).click();
  const settings = page.getByRole("dialog", { name: "Settings", exact: true });
  await expect(settings).toBeVisible();
  const row = settings
    .getByRole("table", { name: "Keybindings" })
    .getByRole("row")
    .filter({ has: page.getByText("Move right", { exact: true }) });
  await row.getByRole("button", { name: "Edit", exact: true }).click();
  const recorder = settings.locator(".ib-recorder");
  await recorder.getByRole("button", { name: "Clear", exact: true }).first().click();
  await recorder.getByRole("button", { name: "Focus recorder", exact: true }).click();

  // Gameplay keys and the settings toggle are captured as a binding, nothing else.
  await page.keyboard.press("d");
  await page.keyboard.press("Space");
  await page.keyboard.press("Escape");
  await expect(settings).toBeVisible();
  await expect(recorder).toContainText("Escape");

  await recorder.getByRole("button", { name: "Cancel", exact: true }).first().click();
  await settings.getByRole("button", { name: "Close", exact: true }).click();
  await expect(settings).not.toBeVisible();
  await page.evaluate(() => document.activeElement?.blur());
  await page.keyboard.up("w");
  await page.keyboard.up("f");

  const commands = await acceptedCommands(page);
  expect(movement(commands)).toEqual(["0,-1", "0,0"]);
  expect(guard(commands)).toEqual([true, false]);
  expect(oneShots(commands)).toEqual([]);
});

// ---------------------------------------------------------------------------------------
// Menu interaction.

test("clicking and keyboard-activating menu controls never attacks", async ({ page, appUrl }) => {
  await openArena(page, appUrl);

  await page.getByRole("button", { name: "Settings", exact: true }).click();
  let settings = page.getByRole("dialog", { name: "Settings", exact: true });
  await settings.getByRole("button", { name: "Close", exact: true }).click();
  await expect(settings).not.toBeVisible();
  await page.getByRole("button", { name: "Step", exact: true }).click();
  await page.getByRole("button", { name: "Resume", exact: true }).click();
  await page.getByRole("button", { name: "Pause", exact: true }).click();
  await page.getByLabel("Speed").selectOption("2");

  // Space and Enter activate a focused menu button, here the dialog's Close button.
  await page.getByRole("button", { name: "Settings", exact: true }).click();
  settings = page.getByRole("dialog", { name: "Settings", exact: true });
  await settings.getByRole("button", { name: "Close", exact: true }).focus();
  await page.keyboard.press("Space");
  await expect(settings).not.toBeVisible();

  const commands = await acceptedCommands(page);
  expect(oneShots(commands)).toEqual([]);
  await expect(
    page.getByRole("button", { name: "Primary attack", exact: true }),
  ).not.toHaveAttribute("data-phase", /.+/);
});

// ---------------------------------------------------------------------------------------
// Touch: lost pointer capture and pointercancel mid-hold.

const point = (id, x, y) => ({ id, x: Math.round(x), y: Math.round(y) });
const center = (id, rect) => point(id, rect.x + rect.width / 2, rect.y + rect.height / 2);

async function phone(browser, appUrl) {
  const context = await browser.newContext({
    viewport: { width: 390, height: 844 },
    hasTouch: true,
    isMobile: true,
  });
  const page = await context.newPage();
  await page.goto(`${appUrl}?scenario=training&seed=42`);
  await page.getByRole("button", { name: "Pause", exact: true }).tap();
  // Remember which pointer each control captured, so the test can take that capture away.
  await page.evaluate(() => {
    globalThis.__arpgPointers = new Map();
    document.addEventListener(
      "pointerdown",
      (event) => {
        const control = event.target.closest?.("[aria-label]");
        if (control)
          globalThis.__arpgPointers.set(control.getAttribute("aria-label"), event.pointerId);
      },
      true,
    );
  });
  const cdp = await context.newCDPSession(page);
  const touch = (type, ...touchPoints) =>
    cdp.send("Input.dispatchTouchEvent", { type, touchPoints });
  return { context, page, touch };
}

// The control that captured a touch loses that capture while the finger stays down. The
// capture is real; the `lostpointercapture` event is dispatched by the test, because
// Chromium does not fire it for touch when the capture is released or moved by script.
async function loseCapture(page, label) {
  await page.evaluate((name) => {
    const control = document.querySelector(`[aria-label="${name}"]`);
    const pointerId = globalThis.__arpgPointers.get(name);
    if (!control.hasPointerCapture(pointerId)) throw new Error(`${name} holds no capture`);
    control.dispatchEvent(
      new PointerEvent("lostpointercapture", { pointerId, pointerType: "touch", bubbles: true }),
    );
  }, label);
}

test("the movement stick stops once when it loses its pointer capture mid-hold", async ({
  browser,
  appUrl,
}) => {
  const { context, page, touch } = await phone(browser, appUrl);
  try {
    const stick = await page.getByRole("group", { name: "Movement joystick" }).boundingBox();
    const origin = center(1, stick);
    await touch("touchStart", origin);
    await touch("touchMove", point(1, origin.x + stick.width * 0.4, origin.y));
    await loseCapture(page, "Movement joystick");
    // The same finger keeps moving and lifts: it no longer owns movement.
    await touch("touchMove", point(1, origin.x, origin.y - stick.height * 0.4));
    await touch("touchEnd");

    expect(movement(await acceptedCommands(page))).toEqual(["1,0", "0,0"]);
  } finally {
    await context.close();
  }
});

test("the movement stick stops once on pointercancel mid-hold", async ({ browser, appUrl }) => {
  const { context, page, touch } = await phone(browser, appUrl);
  try {
    const stick = await page.getByRole("group", { name: "Movement joystick" }).boundingBox();
    const origin = center(1, stick);
    await touch("touchStart", origin);
    await touch("touchMove", point(1, origin.x + stick.width * 0.4, origin.y));
    await touch("touchCancel");

    expect(movement(await acceptedCommands(page))).toEqual(["1,0", "0,0"]);
  } finally {
    await context.close();
  }
});

// Interruptions of a held touch control: a lost capture or a pointercancel.
const INTERRUPTIONS = [
  ["loses its pointer capture", (page, touch, label) => loseCapture(page, label)],
  ["gets pointercancel", (page, touch) => touch("touchCancel")],
];

for (const [interruption, interrupt] of INTERRUPTIONS) {
  test(`a held touch shield lowers once when it ${interruption}`, async ({ browser, appUrl }) => {
    const { context, page, touch } = await phone(browser, appUrl);
    try {
      const shield = center(
        1,
        await page.getByRole("button", { name: "Hold shield" }).boundingBox(),
      );
      await touch("touchStart", shield);
      await interrupt(page, touch, "Hold shield");
      if (interruption !== "gets pointercancel") await touch("touchEnd");

      expect(guard(await acceptedCommands(page))).toEqual([true, false]);
    } finally {
      await context.close();
    }
  });

  test(`a sword gesture that ${interruption} never attacks`, async ({ browser, appUrl }) => {
    const { context, page, touch } = await phone(browser, appUrl);
    try {
      const attack = center(
        1,
        await page.getByRole("button", { name: "Primary attack", exact: true }).boundingBox(),
      );
      await touch("touchStart", attack);
      await interrupt(page, touch, "Primary attack");
      if (interruption !== "gets pointercancel") await touch("touchEnd");

      expect(oneShots(await acceptedCommands(page))).toEqual([]);
    } finally {
      await context.close();
    }
  });

  test(`a touch bow draw that ${interruption} is cancelled, never shot`, async ({
    browser,
    appUrl,
  }) => {
    const { context, page, touch } = await phone(browser, appUrl);
    try {
      await page.getByRole("button", { name: "Switch to bow" }).tap();
      await page.getByRole("button", { name: "Step", exact: true }).click();
      const draw = page.getByRole("button", { name: "Draw bow" });
      await expect(draw).toBeEnabled();
      await touch("touchStart", center(1, await draw.boundingBox()));
      await interrupt(page, touch, "Draw bow");
      if (interruption !== "gets pointercancel") await touch("touchEnd");

      expect(bow(await acceptedCommands(page))).toEqual(["drawBow", "cancelBow"]);
    } finally {
      await context.close();
    }
  });
}

// ---------------------------------------------------------------------------------------
// Blur and a hidden page mid-hold.

for (const [trigger, lose, restore] of [
  ["window blur", (page) => blurWindow(page), async () => {}],
  ["a hidden page", (page) => setPageHidden(page, true), (page) => setPageHidden(page, false)],
]) {
  test(`${trigger} mid-hold releases keys and mouse aim once and nothing resumes`, async ({
    page,
    appUrl,
  }) => {
    await page.addInitScript(() => localStorage.setItem("arpg-mouse-aim-v1", "on"));
    await openArena(page, appUrl);
    const canvas = page.locator("canvas").first();
    const box = await canvas.boundingBox();
    await page.mouse.move(box.x + box.width * 0.3, box.y + box.height * 0.6);
    await page.mouse.move(box.x + box.width * 0.32, box.y + box.height * 0.62);
    await page.keyboard.down("w");
    await page.keyboard.down("d");
    await page.keyboard.down("f");

    await lose(page);
    const released = await acceptedCommands(page);
    // The keys may be released one after another, but movement stops exactly once.
    expect(movement(released).slice(0, 2)).toEqual(["0,-1", "1,-1"]);
    expect(movement(released).at(-1)).toBe("0,0");
    expect(movement(released).filter((value) => value === "0,0")).toHaveLength(1);
    expect(guard(released)).toEqual([true, false]);
    expect(aims(released).at(-1)).toBeNull();
    expect(aims(released).filter((direction) => direction === null)).toHaveLength(1);

    // The page comes back and the keys come up: nothing is released twice or resumes.
    await restore(page);
    await page.keyboard.up("w");
    await page.keyboard.up("d");
    await page.keyboard.up("f");
    const after = await acceptedCommands(page);
    expect(movement(after)).toEqual(movement(released));
    expect(guard(after)).toEqual(guard(released));
    expect(aims(after)).toEqual(aims(released));
    expect(oneShots(after)).toEqual([]);
  });
}

test("window blur mid-hold releases touch movement and a touch shield once", async ({
  browser,
  appUrl,
}) => {
  const { context, page, touch } = await phone(browser, appUrl);
  try {
    const stick = await page.getByRole("group", { name: "Movement joystick" }).boundingBox();
    const origin = center(1, stick);
    const shield = center(2, await page.getByRole("button", { name: "Hold shield" }).boundingBox());
    const right = point(1, origin.x + stick.width * 0.4, origin.y);
    await touch("touchStart", origin);
    await touch("touchMove", right);
    await touch("touchStart", right, shield);

    await blurWindow(page);
    // Both fingers stay down a little longer and move, then lift.
    await touch("touchMove", point(1, origin.x, origin.y - stick.height * 0.4), shield);
    await touch("touchEnd");

    const commands = await acceptedCommands(page);
    expect(movement(commands)).toEqual(["1,0", "0,0"]);
    expect(guard(commands)).toEqual([true, false]);
    expect(oneShots(commands)).toEqual([]);
  } finally {
    await context.close();
  }
});

// ---------------------------------------------------------------------------------------
// Dedicated disconnect mid-hold, then reconnect.

const serverBinary =
  process.env.ARPG_E2E_SERVER_BIN ??
  resolve(process.env.CARGO_TARGET_DIR ?? resolve(repositoryRoot, "target"), "debug/server");

const delay = (milliseconds) => new Promise((done) => setTimeout(done, milliseconds));

async function freeUdpPort() {
  const socket = createSocket("udp4");
  await new Promise((bound) => socket.bind(0, "127.0.0.1", bound));
  const { port } = socket.address();
  await new Promise((closed) => socket.close(closed));
  return port;
}

function createCertificate() {
  const directory = mkdtempSync(join(tmpdir(), "arpg-webtransport-"));
  const certificatePem = join(directory, "cert.pem");
  const privateKeyPem = join(directory, "key.pem");
  execFileSync(
    "openssl",
    [
      "req",
      "-x509",
      "-newkey",
      "ec",
      "-pkeyopt",
      "ec_paramgen_curve:P-256",
      "-nodes",
      "-keyout",
      privateKeyPem,
      "-out",
      certificatePem,
      "-days",
      "10",
      "-subj",
      "/CN=localhost",
      "-addext",
      "subjectAltName=DNS:localhost,IP:127.0.0.1",
    ],
    { stdio: ["ignore", "ignore", "pipe"] },
  );
  const der = execFileSync("openssl", ["x509", "-in", certificatePem, "-outform", "DER"]);
  return {
    directory,
    certificatePem,
    privateKeyPem,
    hashBytes: [...createHash("sha256").update(der).digest()],
  };
}

// Pins the test certificate and lets the test cut the network at the WebTransport
// boundary: `__arpgOutage.begin()` closes the live transport and sends new attempts to an
// unreachable port until `end()`.
async function installTransportControl(page, hashBytes) {
  await page.addInitScript((pinned) => {
    const NativeWebTransport = globalThis.WebTransport;
    if (!NativeWebTransport) return;
    const live = new Set();
    let outage = false;
    globalThis.__arpgOutage = {
      begin() {
        outage = true;
        for (const transport of live) transport.close({ closeCode: 1, reason: "simulated outage" });
      },
      end() {
        outage = false;
      },
    };
    function PinnedWebTransport(url, options = {}) {
      const target = outage ? String(url).replace(/:\d+\//, ":9/") : url;
      const transport = new NativeWebTransport(target, {
        ...options,
        serverCertificateHashes: [{ algorithm: "sha-256", value: new Uint8Array(pinned) }],
      });
      live.add(transport);
      transport.closed.finally(() => live.delete(transport)).catch(() => {});
      return transport;
    }
    PinnedWebTransport.prototype = NativeWebTransport.prototype;
    Object.setPrototypeOf(PinnedWebTransport, NativeWebTransport);
    Object.defineProperty(globalThis, "WebTransport", {
      configurable: true,
      writable: true,
      value: PinnedWebTransport,
    });
  }, hashBytes);
}

async function startDedicatedServer(certificate, port) {
  const child = spawn(serverBinary, [], {
    cwd: repositoryRoot,
    env: {
      ...process.env,
      ARPG_RUN_SEED: "42",
      ARPG_SERVER_PORT: String(port),
      ARPG_SERVER_CERT_PEM: certificate.certificatePem,
      ARPG_SERVER_KEY_PEM: certificate.privateKeyPem,
      ARPG_SERVER_SESSION_PATH: "/arpg",
      ARPG_SERVER_DRAIN_GRACE_MS: "20",
    },
    stdio: ["ignore", "ignore", "pipe"],
  });
  let stderr = "";
  child.stderr.on("data", (chunk) => {
    stderr += chunk.toString();
  });
  const deadline = Date.now() + 10_000;
  while (!stderr.includes("event=arpg_server_start run_seed=42")) {
    if (child.exitCode !== null) throw new Error(`Dedicated server exited:\n${stderr}`);
    if (Date.now() > deadline) throw new Error(`Dedicated server did not start:\n${stderr}`);
    await delay(25);
  }
  await delay(250);
  return child;
}

async function stopDedicatedServer(child) {
  if (!child || child.exitCode !== null) return;
  child.kill("SIGTERM");
  await Promise.race([once(child, "exit"), delay(5_000)]);
  if (child.exitCode === null) {
    child.kill("SIGKILL");
    await once(child, "exit");
  }
}

test("a dedicated disconnect mid-hold stops input until reconnect, then nothing is stuck", async ({
  page,
  appUrl,
}) => {
  test.setTimeout(120_000);
  expect(
    existsSync(serverBinary),
    `build the dedicated server first: cargo build --locked -p arpg-game-server --bin server (looked for ${serverBinary})`,
  ).toBe(true);
  const certificate = createCertificate();
  const port = await freeUdpPort();
  await installTransportControl(page, certificate.hashBytes);
  const server = await startDedicatedServer(certificate, port);
  try {
    await page.goto(appUrl);
    await page.getByRole("button", { name: "Enter World", exact: true }).click();
    const status = page.getByRole("region", { name: "Player status" }).locator("p").first();
    const shield = page.getByRole("button", { name: "Hold shield", exact: true });
    const connect = async () => {
      await page.getByRole("button", { name: "Settings", exact: true }).click();
      const settings = page.getByRole("dialog", { name: "Settings", exact: true });
      await settings.getByLabel("WebTransport endpoint").fill(`https://localhost:${port}/arpg`);
      await settings.getByRole("button", { name: "Connect dedicated server", exact: true }).click();
      await expect(status).toContainText(/Dedicated authority · player \d+ · 60 Hz/, {
        timeout: 30_000,
      });
      await settings.getByRole("button", { name: "Close", exact: true }).click();
      await expect(settings).not.toBeVisible();
      await page.evaluate(() => document.activeElement?.blur());
    };
    await connect();

    // Holding forward and the shield when the connection is lost for good.
    await page.keyboard.down("w");
    await page.keyboard.down("f");
    await expect(shield).toHaveAttribute("data-guard", "raised");
    await page.evaluate(() => globalThis.__arpgOutage.begin());
    await expect(status).toHaveText("Connecting to dedicated authority…", { timeout: 15_000 });
    await expect(status).toHaveText("Dedicated authority disconnected", { timeout: 30_000 });

    // While disconnected, releasing and pressing gameplay keys applies nothing: the player
    // keeps seeing the disconnect, not failed commands.
    await page.keyboard.up("w");
    await page.keyboard.press("d");
    await page.keyboard.press("Space");
    await page.waitForTimeout(500);
    await expect(status).toHaveText("Dedicated authority disconnected");

    // Reconnect while the shield key is still physically held: the fresh session starts
    // with nothing held, and only a new press raises the shield again.
    await page.evaluate(() => globalThis.__arpgOutage.end());
    await connect();
    await page.waitForTimeout(500);
    await expect(shield).not.toHaveAttribute("data-guard", /.+/);
    await page.keyboard.up("f");
    await page.waitForTimeout(300);
    await expect(shield).not.toHaveAttribute("data-guard", /.+/);
    await page.keyboard.down("f");
    await expect(shield).toHaveAttribute("data-guard", "raised", { timeout: 10_000 });
    await page.keyboard.up("f");
    await expect(shield).not.toHaveAttribute("data-guard", /.+/, { timeout: 10_000 });
  } finally {
    await stopDedicatedServer(server);
    rmSync(certificate.directory, { recursive: true, force: true });
  }
});
