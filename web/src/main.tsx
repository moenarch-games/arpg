import {
  StrictMode,
  useCallback,
  useEffect,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
  useSyncExternalStore,
} from "react";
import { createRoot } from "react-dom/client";
import { createThreeSceneRenderer } from "@moritzbrantner/three-d-renderer";
import {
  adaptImportedCharacterBundle,
  adaptImportedItemBundle,
  readBundleAttachment,
} from "@moritzbrantner/three-d-renderer/imported-character";
import { createEquipment } from "./character-equipment.ts";
import { createCharacterAssets } from "./character-assets.ts";
import { RANGER_BUNDLE } from "./character-bundles.js";
import { createCharacterLayer } from "./character-layer.ts";
import { applyProfile, validateRegistry } from "@moritzbrantner/input-bindings";
import { keyLabelForAction } from "./binding-labels.js";
import { createSnapshotStore } from "./snapshot-store.js";
import {
  PlayerHud,
  CombatActions,
  TrainingDiagnostics,
  StrikeTimeline,
  TrainingTick,
  GameSummary,
} from "./snapshot-views.jsx";
import { SettingsDialog } from "./settings-dialog.jsx";
import {
  PLAY,
  closeLayer,
  escapeFrom,
  leaveConsequence,
  leaveNeedsConfirmation,
  menuPausesWorld,
  menuWorldNotice,
  openLeaveConfirmation,
  openMenu,
  openSettings,
} from "./game-menu.js";
import { PROFILE_KEY, loadInputProfile } from "./input-profile.js";
import {
  SETUP_URL_KEY,
  DEDICATED_URL_KEY,
  MOUSE_AIM_KEY,
  DEFAULT_PRESENTATION,
  MOTION_PREFERENCES,
  TEXT_SCALES,
  loadPresentation,
  loadMouseAim,
  normalizePresentation,
  persistPresentation,
  readStoredValue,
  persistStoredValue,
} from "./preferences.js";
import { InputRuntimeController } from "@moritzbrantner/input-bindings-runtime";
import { attachKeyboardRuntime, isTextEntryTarget } from "@moritzbrantner/input-bindings-web";
import { KeybindingEditor } from "@moritzbrantner/input-bindings-react";
import "@moritzbrantner/input-bindings-react/styles.css";
import initWasm, {
  WasmGame,
  contentRevision,
  loadGameFromSaveStateJson,
} from "./wasm/arpg_web_wasm.js";
import { attachPeerGameSession } from "./peer-session.js";
import { DedicatedGameSession } from "./dedicated-session.js";
import { DemoLobbySession } from "./vendor/multiplayer-setup-service/demo-session.ts";
import { sampleVirtualStick } from "./virtual-stick.js";
import { aimFromPointer, movementFromAim } from "./aim-input.js";
import { createFollowCamera, createPinchZoom, focusPoint } from "./follow-camera.js";
import {
  CHARACTER_PRESETS,
  loadSelectedCharacterId,
  persistSelectedCharacterId,
  resolveCharacter,
} from "./character-selection.js";
import {
  MAX_SAVE_FILE_BYTES,
  createSaveDocument,
  hasPersistedSaveDocument,
  loadPersistedSaveDocument,
  parseSaveDocument,
  persistSaveDocument,
  saveFileName,
  serializeSaveDocument,
} from "./save-state.js";
import {
  DEFAULT_TRAINING_SEED,
  TRAINING_SPEEDS,
  parseRunSeed,
  readTrainingRequest,
  withTrainingRequest,
  SCENARIOS,
} from "./training-arena.js";
import { createGameClientRuntime } from "./game-client-runtime.ts";
import { createFramePresenter } from "./frame-presenter.ts";
import { buildFrame } from "./scene-frame.ts";
import { applyDocumentPresentation, resolvePresentationEffects } from "./presentation-effects.js";
import { createInputAttachment } from "./input-attachment.ts";
import { WorkbenchControls } from "./workbench-controls.jsx";
import {
  type MessageKey,
  formatMegabytes,
  formatPercent,
  longLabelCatalog,
  setMessages,
  t,
} from "./messages.ts";
import "./styles.css";

// English is the only shipped locale. Development builds (and so the browser tests) can
// render the long-label test locale with `?locale=x-long` to check layouts with longer text.
if (
  import.meta.env.DEV &&
  new URLSearchParams(globalThis.location?.search).get("locale") === "x-long"
)
  setMessages("en", longLabelCatalog());

const gameplayContext = { op: "context", id: "gameplay" };

const MOTION_LABELS: Record<string, MessageKey> = {
  system: "settings.motion.system",
  reduced: "settings.motion.reduced",
  full: "settings.motion.full",
};
// Marks the history entry pushed on entering the world (#60). It holds no session data.
const WORLD_HISTORY_KEY = "arpgWorld";

// The live `prefers-reduced-motion` media query; "Follow system" motion tracks its changes.
let reducedMotionQuery = null;
const reducedMotionMedia = () =>
  (reducedMotionQuery ??= globalThis.matchMedia?.("(prefers-reduced-motion: reduce)") ?? null);
function subscribeReducedMotion(listener) {
  const query = reducedMotionMedia();
  query?.addEventListener("change", listener);
  return () => query?.removeEventListener("change", listener);
}
const prefersReducedMotion = () => reducedMotionMedia()?.matches ?? false;

const physical = (id, action, code, when = gameplayContext) => ({
  id,
  action,
  sequence: [{ key: { kind: "physical", value: code }, modifiers: {} }],
  when,
  priority: 0,
});

const inputRegistry = {
  actions: [
    ["game.moveForward", "input.moveForward", "KeyW", "allow", "input.category.movement"],
    ["game.moveBackward", "input.moveBackward", "KeyS", "allow", "input.category.movement"],
    ["game.moveLeft", "input.moveLeft", "KeyA", "allow", "input.category.movement"],
    ["game.moveRight", "input.moveRight", "KeyD", "allow", "input.category.movement"],
    // The standard scheme (#59): Space jumps and R attacks. Older stored profiles migrate
    // once (`input-profile.js`); customized ones keep their bindings.
    ["game.jump", "input.jump", "Space", "never", "input.category.movement"],
    ["game.primaryAttack", "input.primaryAttack", "KeyR", "never", "input.category.combat"],
    ["game.secondaryAttack", "input.secondaryAttack", "KeyQ", "never", "input.category.combat"],
    ["game.interact", "input.interact", "KeyE", "never", "input.category.interaction"],
    ["game.guard", "input.guard", "KeyF", "never", "input.category.combat"],
    ["game.switchWeapon", "input.switchWeapon", "KeyX", "never", "input.category.combat"],
    ["game.cycleTarget", "input.cycleTarget", "KeyT", "never", "input.category.combat"],
    ["game.clearTarget", "input.clearTarget", "KeyG", "never", "input.category.combat"],
    // The follow camera has no manual rotation (Atlas 4.1.4); this restores its zoom.
    ["game.resetCamera", "input.resetCamera", "KeyC", "never", "input.category.camera"],
  ]
    .map(([id, title, code, repeatPolicy, category]) => ({
      id,
      title: t(title as MessageKey),
      categoryPath: [t("app.name"), t(category as MessageKey)],
      repeatPolicy,
      allowedDevices: ["keyboard"],
      defaults: [physical(`${id}.default`, id, code)],
      provenance: { source: "arpg", version: "1" },
    }))
    .concat([
      {
        id: "game.menu",
        title: t("input.menu"),
        categoryPath: [t("app.name"), t("input.category.system")],
        repeatPolicy: "never",
        allowedDevices: ["keyboard"],
        defaults: [
          physical("game.menu.default", "game.menu", "Escape", {
            op: "always",
          }),
        ],
        provenance: { source: "arpg", version: "1" },
      },
    ]),
};

const inputDefaults = inputRegistry.actions.flatMap((action) => action.defaults);

// Labels of the keys each action is bound to under `profile`; `null` for an unbound action.
function actionKeyLabels(profile) {
  const bindings = applyProfile(inputDefaults, profile).bindings;
  return Object.fromEntries(
    inputRegistry.actions.map((action) => [action.id, keyLabelForAction(bindings, action.id)]),
  );
}

// Offers `text` as a JSON file download.
function downloadJson(text, fileName) {
  const blob = new Blob([text], { type: "application/json" });
  const url = URL.createObjectURL(blob);
  const link = globalThis.document?.createElement?.("a");
  if (!link) throw new Error("Browser download API is unavailable");
  link.href = url;
  link.download = fileName;
  link.click();
  URL.revokeObjectURL(url);
}

function freshRunSeed() {
  const values = new Uint32Array(1);
  globalThis.crypto.getRandomValues(values);
  return values[0];
}

function requestedRunSeed() {
  return parseRunSeed(new URLSearchParams(location.search).get("seed"));
}

// The Ranger character bundle (#55), unpacked into the static `characters/` directory by
// setup. Loaded once per page with the game core and shared by every session.
// Preloading and drawing share the persistent game canvas and actual renderer.
let characterRenderer = null;

// The gameplay follow camera (#58) for the one game canvas. It owns the live zoom, which
// persists in the presentation document (#61). Pinch zoom tracks the touches on that canvas.
const followCamera = createFollowCamera({
  zoom: loadPresentation().camera.zoom,
});
const pinchZoom = createPinchZoom();
const textureCapabilities = {
  configureTextureLoader(loader) {
    if (!characterRenderer) throw new Error("Character renderer is not ready");
    characterRenderer.configureTextureLoader(loader);
  },
};
const characterAssets = createCharacterAssets({
  bundle: RANGER_BUNDLE,
  baseUrl: `${import.meta.env.BASE_URL}characters/`,
  fetch: (url) => fetch(url),
  // The character, its equipment items and its sockets (#183) from the same bundle.
  adapt: async (source, options) => {
    const decoderOptions = {
      ...options,
      renderer: textureCapabilities,
      transcoderPath: `${import.meta.env.BASE_URL}decoders/basis/`,
    };
    const character = await adaptImportedCharacterBundle(source, decoderOptions);
    const { equipment: data } = RANGER_BUNDLE;
    const items = Object.fromEntries(
      await Promise.all(
        Object.entries(data.items).map(async ([id, item]) => [
          id,
          await adaptImportedItemBundle(
            { ...source, key: item.key, variant: "default" },
            { renderer: textureCapabilities, transcoderPath: decoderOptions.transcoderPath },
          ),
        ]),
      ),
    );
    const sockets = await readBundleAttachment({ ...source, name: "humanoid-sockets" });
    const equipment = createEquipment({ data, items, sockets });
    return {
      clips: character.clips,
      createInstance: character.createInstance,
      equip: equipment.equip,
    };
  },
});

// The game world canvas. One frame presenter owns its renderer, projection and resize
// observer for as long as the canvas is mounted; view changes reconfigure it in place.
// The keyboard owner for the focused element, highest capture priority first (#176): the
// shared keybinding editor's recorder, then any text-entry control. `null` leaves the
// context to the menu/gameplay state.
function focusedInputOwner(element) {
  if (element?.closest?.(".ib-recorder")) return "rebinding-capture";
  if (isTextEntryTarget(element)) return "text-entry";
  return null;
}

function GameCanvas({
  snapshots,
  playerId,
  accent,
  graphics,
  effects,
  camera,
  cameraSmoothing,
  onWheel,
  onPointerMove,
  onPointerDown,
  onPointerUp,
  onPointerCancel,
  onLostPointerCapture,
  onMouseUp,
  onContextMenu,
}) {
  const canvasRef = useRef(null);
  const presenterRef = useRef(null);
  const { shadows, pixelRatioLimit } = graphics;
  // The view the presenter is (or will be) created with; later changes apply in place.
  const viewRef = useRef({
    playerId,
    accent,
    graphics: { shadows, pixelRatioLimit },
    effects,
    cameraSmoothing,
  });
  useEffect(() => {
    // A new session starts with the camera on its player, not gliding from the last one.
    camera.recenter();
    const presenter = createFramePresenter({
      canvas: canvasRef.current,
      snapshots,
      camera: {
        update: (snapshot, id, time, smoothing) =>
          camera.follow(focusPoint(snapshot, id), time, smoothing),
        pose: () => camera.pose(),
        subscribe: (listener) => camera.subscribe(listener),
      },
      createRenderer: (canvas, options) => {
        const renderer = createThreeSceneRenderer(canvas, options);
        characterRenderer = renderer;
        return renderer;
      },
      buildFrame,
      observeResize: (canvas, onResize) => {
        const observer = new ResizeObserver(() => onResize());
        observer.observe(canvas);
        return () => observer.disconnect();
      },
      requestFrame: (callback) => requestAnimationFrame(callback),
      cancelFrame: (handle) => cancelAnimationFrame(handle),
      characters: createCharacterLayer({
        getAsset: characterAssets.get,
        animation: RANGER_BUNDLE.animation,
        // Animation follows the simulation's fixed tick (arpg-core TICK_HZ).
        tickHz: 60,
      }),
      view: viewRef.current,
    });
    presenterRef.current = presenter;
    return () => {
      presenterRef.current = null;
      presenter.dispose();
      characterRenderer = null;
    };
  }, [snapshots, camera]);

  useEffect(() => {
    viewRef.current = {
      playerId,
      accent,
      graphics: { shadows, pixelRatioLimit },
      effects,
      cameraSmoothing,
    };
    presenterRef.current?.configure(viewRef.current);
  }, [playerId, accent, shadows, pixelRatioLimit, effects, cameraSmoothing]);

  return (
    <canvas
      ref={canvasRef}
      className="game-canvas"
      aria-label={t("app.world")}
      onPointerMove={onPointerMove}
      onPointerDown={onPointerDown}
      onPointerUp={onPointerUp}
      onPointerCancel={onPointerCancel}
      onLostPointerCapture={onLostPointerCapture}
      onMouseUp={onMouseUp}
      onContextMenu={onContextMenu}
      onWheel={onWheel}
    />
  );
}

function App() {
  const settingsTriggerRef = useRef(null);
  const touchStickRef = useRef(null);
  const touchKnobRef = useRef(null);
  const touchPointerIdRef = useRef(null);
  const mousePointerIdRef = useRef(null);
  // Synchronous epoch invalidation protects gestures across a pending
  // weapon switch/session change, even before a new game snapshot arrives.
  const touchGestureEpochRef = useRef(0);
  const saveFileInputRef = useRef(null);
  const [initialRequest] = useState(() => {
    const training = readTrainingRequest(location.search);
    return {
      training,
      seed: requestedRunSeed() ?? (training.requested ? training.seed : freshRunSeed()),
    };
  });
  const initialTrainingRef = useRef(initialRequest.training);
  const initialRunSeedRef = useRef(initialRequest.seed);
  const [snapshotStore] = useState(createSnapshotStore);
  // Shown once the core is ready; while it loads the shell shows the load state instead.
  const [status, setStatus] = useState(() => t("status.selectCharacter"));
  const [runtime] = useState(() =>
    createGameClientRuntime({
      snapshots: snapshotStore,
      // The game starts once both the Rust/Wasm core and the character assets are ready.
      loadCore: () => Promise.all([initWasm(), characterAssets.load()]),
      // Training sessions (including the default dungeon scenario) are scenario games,
      // which record their session for reproduction export; ordinary play does not.
      createGame: (seed, scenario) =>
        scenario ? WasmGame.newScenario(scenario, seed) : new WasmGame(seed),
      createPeerSession: (apiBase) => new DemoLobbySession({ apiBase, topology: "host" }),
      attachPeerSession: attachPeerGameSession,
      createDedicatedSession: (endpoint) => new DedicatedGameSession({ endpoint }),
      supportsWebTransport: () => "WebTransport" in globalThis,
      freshRunSeed,
      localContentRevision: contentRevision,
      onStatus: setStatus,
    }),
  );
  const client = useSyncExternalStore(runtime.subscribe, runtime.getState);
  const { mode, playerId, lobbyCode } = client;
  const ready = client.core === "ready";
  const coreFailed = client.core === "failed";
  // An honest load state: character byte progress while it loads, failed (with the
  // runtime's failure message in `status`) or ready.
  const characterProgress = useSyncExternalStore(
    characterAssets.subscribe,
    characterAssets.getProgress,
  );
  const shownStatus =
    ready || coreFailed
      ? status
      : characterProgress?.totalBytes
        ? t("app.loadingBytes", {
            loaded: formatMegabytes(characterProgress.loadedBytes),
            total: formatMegabytes(characterProgress.totalBytes),
          })
        : t("app.loading");
  const scenario = client.training ? "training" : "adventure";
  const trainingPaused = client.training?.paused ?? false;
  const trainingSpeed = client.training?.speed ?? 1;
  const currentPlayer = () =>
    snapshotStore.getSnapshot()?.players.find((player) => player.id === playerId) ?? null;
  // The in-world overlay layer (#60): play, the game menu, settings or the leave confirmation.
  const [overlay, setOverlay] = useState(PLAY);
  const settingsOpen = overlay.layer === "settings";
  const overlayOpen = overlay.layer !== null;
  const stepOverlay = useCallback((step) => setOverlay((current) => step(current)), []);
  const [inWorld, setInWorld] = useState(false);
  const [savedGameAvailable, setSavedGameAvailable] = useState(() => hasPersistedSaveDocument());
  const [trainingSeedDraft, setTrainingSeedDraft] = useState(String(initialRequest.training.seed));
  const [trainingFixture, setTrainingFixture] = useState(initialRequest.training.fixture);
  const [selectedCharacterId, setSelectedCharacterId] = useState(() => loadSelectedCharacterId());
  const [profile, setProfile] = useState(() =>
    loadInputProfile({
      validate: (profile) => validateRegistry(inputRegistry, profile).valid,
      defaults: inputDefaults,
      foundation: { applyProfile },
    }),
  );
  // Local display preferences (#61): never part of a save or the authority.
  const [presentation, setPresentation] = useState(() => loadPresentation());
  const { graphics, accessibility } = presentation;
  // Wheel, pinch and reset zoom changes save into the same document without re-rendering.
  const presentationRef = useRef(presentation);
  useLayoutEffect(() => {
    presentationRef.current = presentation;
  }, [presentation]);
  useEffect(() => {
    let savedZoom = followCamera.zoom();
    return followCamera.subscribe(() => {
      if (followCamera.zoom() === savedZoom) return;
      savedZoom = followCamera.zoom();
      persistPresentation({
        ...presentationRef.current,
        camera: { zoom: savedZoom },
      });
    });
  }, []);
  const systemReducedMotion = useSyncExternalStore(
    subscribeReducedMotion,
    prefersReducedMotion,
    () => false,
  );
  const presentationEffects = useMemo(
    () => resolvePresentationEffects(presentation, { prefersReducedMotion: systemReducedMotion }),
    [presentation, systemReducedMotion],
  );
  useLayoutEffect(() => {
    applyDocumentPresentation(document.documentElement, presentationEffects);
  }, [presentationEffects]);
  const [mouseAim, setMouseAim] = useState(() => loadMouseAim());
  const [setupUrl, setSetupUrl] = useState(() =>
    readStoredValue(SETUP_URL_KEY, "http://127.0.0.1:8787"),
  );
  const [dedicatedUrl, setDedicatedUrl] = useState(() =>
    readStoredValue(DEDICATED_URL_KEY, "https://127.0.0.1:4433/arpg"),
  );
  const [joinCode, setJoinCode] = useState(
    () => new URLSearchParams(location.search).get("join") ?? "",
  );

  // Prompts and hints name the keys of the effective profile, never the defaults.
  const keyLabels = useMemo(() => actionKeyLabels(profile), [profile]);
  const interactKey = keyLabels["game.interact"];

  const selectedCharacter = useMemo(
    () => resolveCharacter(selectedCharacterId),
    [selectedCharacterId],
  );

  const resetTouchVisual = useCallback(() => {
    touchPointerIdRef.current = null;
    if (touchKnobRef.current) {
      touchKnobRef.current.style.transform = "translate3d(0px, 0px, 0)";
    }
  }, []);

  // Weapon changes and world/focus transitions invalidate action strokes.
  // Ordinary joystick release only resets its own visual and movement.
  const invalidateTouchGestures = useCallback(() => {
    touchGestureEpochRef.current += 1;
    resetTouchVisual();
  }, [resetTouchVisual]);

  const releaseInput = useCallback(() => {
    invalidateTouchGestures();
    mousePointerIdRef.current = null;
    runtime.releaseInput();
  }, [invalidateTouchGestures, runtime]);

  const resetTouchStick = () => {
    // Ending movement must never invalidate a different finger's attack.
    resetTouchVisual();
    runtime.setTouchMovement(0, 0);
  };

  const updateTouchStick = (event) => {
    if (touchPointerIdRef.current !== event.pointerId) return;
    const stick = touchStickRef.current;
    if (!stick) return;
    const sample = sampleVirtualStick(event.clientX, event.clientY, stick.getBoundingClientRect());
    if (touchKnobRef.current) {
      touchKnobRef.current.style.transform = `translate3d(${sample.visualX}px, ${sample.visualY}px, 0)`;
    }
    runtime.setTouchMovement(sample.x, sample.z);
  };

  const beginTouchStick = (event) => {
    if (!playerId || !currentPlayer()?.alive || touchPointerIdRef.current !== null) return;
    event.preventDefault();
    touchPointerIdRef.current = event.pointerId;
    event.currentTarget.setPointerCapture?.(event.pointerId);
    updateTouchStick(event);
  };

  const endTouchStick = (event) => {
    if (touchPointerIdRef.current !== event.pointerId) return;
    event.preventDefault();
    if (event.currentTarget.hasPointerCapture?.(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId);
    }
    resetTouchStick();
  };

  // Reads the focused player from the runtime, so long-lived input handlers never hold a
  // stale player id.
  const focusedPlayer = useCallback(() => {
    const id = runtime.getState().playerId;
    return snapshotStore.getSnapshot()?.players.find((player) => player.id === id) ?? null;
  }, [runtime, snapshotStore]);

  // Keyboard and touch gestures submit the same input-bindings action vocabulary.
  // Only arpg-core adjudicates whether a submitted command is possible or succeeds.
  const triggerCombatAction = useCallback(
    (action) => {
      if (!focusedPlayer()?.alive) return;
      if (action === "game.cycleTarget") return runtime.cycleTarget();
      if (action === "game.clearTarget") return runtime.clearTarget();
      const type = {
        "game.jump": "jump",
        "game.primaryAttack": "primaryAttack",
        "game.secondaryAttack": "secondaryAttack",
        "game.interact": "interact",
      }[action];
      if (type) runtime.dispatch({ type });
    },
    [focusedPlayer, runtime],
  );

  const switchWeapon = useCallback(() => {
    const player = focusedPlayer();
    if (!player?.alive) return;
    // A touch already in progress belongs to the previous weapon.
    touchGestureEpochRef.current += 1;
    // Switching never fires: any held draw is cancelled first.
    runtime.cancelBowDraw();
    runtime.dispatch({
      type: "equipWeapon",
      weapon: player.weapon === "bow" ? "swordAndShield" : "bow",
    });
  }, [focusedPlayer, runtime]);

  // Keyboard input is owned outside React: one attachment per mounted runtime, attached while
  // in the world, with profile and gameplay/menu context changes applied in place (below).
  const inputRef = useRef(null);
  const profileRef = useRef(profile);
  useEffect(() => {
    const input = createInputAttachment({
      profile: profileRef.current,
      createController: (controllerOptions) => {
        const controller = new InputRuntimeController({
          registry: inputRegistry,
          consumePolicy: "matched",
          ...controllerOptions,
        });
        return Object.assign(controller, {
          updateProfile: (next) => controller.updateConfiguration(inputRegistry, next),
        });
      },
      attachKeyboard: (controller) =>
        attachKeyboardRuntime(controller, {
          mode: "physical",
          ignoreTextEntry: true,
          stopPropagation: true,
        }),
      // The menu key also steps out of open menus, so it stays live in menu context.
      alwaysActions: ["game.menu"],
      onDispatch: (dispatch) => {
        if (dispatch.action === "game.menu") {
          if (dispatch.phase === "press") stepOverlay(escapeFrom);
          return;
        }
        // Always forward a release so a draw held across a weapon switch cannot stick.
        if (dispatch.action === "game.primaryAttack" && dispatch.phase === "release") {
          runtime.setBowDraw("keyboard", false);
          return;
        }
        if (dispatch.action === "game.primaryAttack" && focusedPlayer()?.weapon === "bow") {
          if (dispatch.phase === "press") runtime.setBowDraw("keyboard", true);
          return;
        }
        if (dispatch.action === "game.resetCamera") {
          if (dispatch.phase === "press") followCamera.reset();
          return;
        }
        if (dispatch.action === "game.switchWeapon") {
          if (dispatch.phase === "press") switchWeapon();
          return;
        }
        if (
          [
            "game.jump",
            "game.primaryAttack",
            "game.secondaryAttack",
            "game.interact",
            "game.cycleTarget",
            "game.clearTarget",
          ].includes(dispatch.action)
        ) {
          if (dispatch.phase === "press") triggerCombatAction(dispatch.action);
          return;
        }
        if (dispatch.action === "game.guard") {
          runtime.setGuard("keyboard", dispatch.phase !== "release");
          return;
        }
        const key = {
          "game.moveForward": "forward",
          "game.moveBackward": "backward",
          "game.moveLeft": "left",
          "game.moveRight": "right",
        }[dispatch.action];
        if (!key) return;
        runtime.setHeldMovement(key, dispatch.phase !== "release");
      },
      // Held keys and touches do not survive focus loss, menus or leaving the world.
      onRelease: releaseInput,
      focusTarget: window,
      visibilityTarget: document,
    });
    inputRef.current = input;
    return () => {
      inputRef.current = null;
      input.detach();
    };
    // Every dependency is stable for the runtime's lifetime, so this runs once per mount.
  }, [runtime, focusedPlayer, switchWeapon, triggerCombatAction, releaseInput, stepOverlay]);

  const leaveTrainingUrl = () => {
    history.replaceState(history.state, "", withTrainingRequest(location.href, false, 0));
  };

  const startTraining = (seed, fixture = trainingFixture) => {
    invalidateTouchGestures();
    runtime.startLocal({ seed, training: true, scenario: fixture });
    initialRunSeedRef.current = null;
    setTrainingSeedDraft(String(seed));
    setTrainingFixture(fixture);
    history.replaceState(
      history.state,
      "",
      withTrainingRequest(location.href, true, seed, fixture),
    );
  };

  const enterTrainingArena = (candidateSeed = trainingSeedDraft) => {
    if (!ready) return;
    const seed = parseRunSeed(candidateSeed);
    if (seed === null) {
      setStatus(t("status.badSeed"));
      return;
    }
    const selectedId = persistSelectedCharacterId(undefined, selectedCharacterId);
    const selected = resolveCharacter(selectedId);
    startTraining(seed);
    setInWorld(true);
    setStatus(t("status.trainingWith", { name: selected.name }));
  };

  const restartTrainingArena = (candidateSeed = trainingSeedDraft) => {
    const seed = parseRunSeed(candidateSeed);
    if (seed === null || scenario !== "training") {
      setStatus(t("status.badSeed"));
      return;
    }
    startTraining(seed);
    setStatus(t("status.trainingRestarted", { seed: String(seed) }));
  };

  const stepTrainingArena = () => {
    if (scenario !== "training") return;
    runtime.stepTraining();
  };

  const enterWorld = () => {
    if (!ready) return;
    const selectedId = persistSelectedCharacterId(undefined, selectedCharacterId);
    const selected = resolveCharacter(selectedId);
    invalidateTouchGestures();
    runtime.startLocal({ seed: initialRunSeedRef.current ?? freshRunSeed() });
    initialRunSeedRef.current = null;
    leaveTrainingUrl();
    setInWorld(true);
    setStatus(t("status.localGame", { name: selected.name }));
  };

  const returnToCharacters = () => {
    runtime.stop();
    invalidateTouchGestures();
    setOverlay(PLAY);
    setInWorld(false);
    leaveTrainingUrl();
    setStatus(t("status.selectCharacter"));
  };

  // Leave from the menu or browser Back (#60): a run asks first, training leaves at once.
  const requestLeave = () => {
    if (leaveNeedsConfirmation({ training: runtime.getState().training !== null })) {
      setOverlay(openLeaveConfirmation());
    } else {
      returnToCharacters();
    }
  };
  const requestLeaveRef = useRef(requestLeave);
  const inWorldRef = useRef(inWorld);
  useEffect(() => {
    requestLeaveRef.current = requestLeave;
    inWorldRef.current = inWorld;
  });

  // The menu pauses a local authority while any in-world layer is open; shared worlds run on.
  useEffect(() => {
    runtime.setMenuPaused(inWorld && overlayOpen);
  }, [runtime, inWorld, overlayOpen]);

  // Browser history (#60): entering the world pushes one entry, so Back asks to leave instead
  // of leaving the page. A reload never resumes a session: it starts on the character screen
  // and drops the stale world marker. Leaving the world pops the entry it pushed.
  const historyStartedRef = useRef(false);
  const expectingPopRef = useRef(false);
  const confirmingLeave = overlay.layer === "confirmLeave";
  useEffect(() => {
    const marked = history.state?.[WORLD_HISTORY_KEY] === true;
    if (!historyStartedRef.current) {
      historyStartedRef.current = true;
      if (marked && !inWorld) {
        history.replaceState(null, "", location.href);
        return;
      }
    }
    if (inWorld && !confirmingLeave && !marked) {
      history.pushState({ [WORLD_HISTORY_KEY]: true }, "", location.href);
    } else if (!inWorld && marked) {
      expectingPopRef.current = true;
      history.back();
    }
  }, [inWorld, confirmingLeave]);
  useEffect(() => {
    const onPopState = () => {
      if (expectingPopRef.current) {
        expectingPopRef.current = false;
        leaveTrainingUrl();
        return;
      }
      if (inWorldRef.current) requestLeaveRef.current();
    };
    window.addEventListener("popstate", onPopState);
    return () => window.removeEventListener("popstate", onPopState);
  }, []);

  const startLocal = () => {
    invalidateTouchGestures();
    runtime.startLocal();
    leaveTrainingUrl();
    setStatus(t("status.localGame", { name: selectedCharacter.name }));
  };

  const startDedicated = async () => {
    invalidateTouchGestures();
    const connecting = runtime.startDedicated(dedicatedUrl.trim());
    if (runtime.getState().mode === "dedicated") leaveTrainingUrl();
    await connecting;
  };

  const hostPeerGame = async () => {
    invalidateTouchGestures();
    leaveTrainingUrl();
    await runtime.hostPeer(setupUrl);
  };

  const joinPeerGame = async () => {
    invalidateTouchGestures();
    leaveTrainingUrl();
    await runtime.joinPeer(setupUrl, joinCode);
  };

  useEffect(() => {
    // The runtime owns the core load; a start requested before it is ready waits for it and
    // is superseded by the cleanup's stop, so a slow load never activates after unmount.
    void runtime.load();
    const initialTraining = initialTrainingRef.current;
    if (initialTraining.requested) {
      // Read from the immutable initial request: a StrictMode remount starts the same seed.
      const seed = initialRequest.seed ?? DEFAULT_TRAINING_SEED;
      const { fixture, unknownFixture } = initialTraining;
      runtime.startLocal({ seed, training: true, scenario: fixture });
      initialRunSeedRef.current = null;
      setTrainingSeedDraft(String(seed));
      setInWorld(true);
      history.replaceState(
        history.state,
        "",
        withTrainingRequest(location.href, true, seed, fixture),
      );
      setStatus(
        unknownFixture
          ? t("status.unknownScenario", { scenario: unknownFixture })
          : t("status.training"),
      );
    }
    return () => {
      // Releases the authority, transports and tick loop. The runtime stays reusable so a
      // StrictMode remount can start again.
      runtime.stop();
    };
  }, [runtime, initialRequest]);

  // Retries a failed core load. In the world (the training route) that restarts the
  // requested arena, which waits for the new attempt.
  const retryCoreLoad = () => {
    if (inWorld) {
      const seed = parseRunSeed(trainingSeedDraft) ?? DEFAULT_TRAINING_SEED;
      startTraining(seed);
      setStatus(t("status.training"));
      return;
    }
    setStatus(t("status.selectCharacter"));
    void runtime.load();
  };

  useEffect(() => {
    profileRef.current = profile;
    inputRef.current?.setProfile(profile);
  }, [profile]);

  useEffect(() => {
    const input = inputRef.current;
    if (!input || !ready || !inWorld) return undefined;
    input.attach();
    return () => input.detach();
  }, [ready, inWorld]);

  // Which keyboard owner has focus (#176): a binding recorder, a text-entry control or none.
  const [focusOwner, setFocusOwner] = useState(() => focusedInputOwner(document.activeElement));
  useEffect(() => {
    const update = () => setFocusOwner(focusedInputOwner(document.activeElement));
    document.addEventListener("focusin", update);
    document.addEventListener("focusout", update);
    return () => {
      document.removeEventListener("focusin", update);
      document.removeEventListener("focusout", update);
    };
  }, []);

  useEffect(() => {
    // Non-gameplay contexts suppress gameplay bindings and release held input.
    inputRef.current?.setContext(focusOwner ?? (overlayOpen ? "menu" : "gameplay"));
  }, [focusOwner, overlayOpen]);

  const updateProfile = (next) => {
    if (!persistStoredValue(PROFILE_KEY, JSON.stringify(next)))
      setStatus(t("status.controlsSession"));
    setProfile(next);
  };

  const updateMouseAim = (enabled) => {
    if (!persistStoredValue(MOUSE_AIM_KEY, enabled ? "on" : "off"))
      setStatus(t("status.controlsSession"));
    setMouseAim(enabled);
  };

  useEffect(() => {
    // Turning mouse aim off returns melee and the bow to committed facing.
    if (!mouseAim) runtime.setAim(null);
  }, [mouseAim, runtime]);

  // Cursor projection is shared between continuous facing and right-button movement.
  // The Rust authority owns both the resulting aim and physical locomotion.
  const pointerAim = (event) => {
    const rect = event.currentTarget.getBoundingClientRect();
    return aimFromPointer(
      event.clientX - rect.left,
      event.clientY - rect.top,
      rect.width,
      rect.height,
      followCamera.pose(),
      followCamera.focus(),
    );
  };

  // Camera zoom follows the gameplay context: menus and focused UI consume it.
  const cameraInputAllowed = () => inputRef.current?.getContext() === "gameplay";
  const zoomWithWheel = (event) => {
    if (!cameraInputAllowed()) return;
    // Pixel, line and page deltas: a line is ~16px, a page one canvas height.
    const pixels =
      event.deltaY * (event.deltaMode === 1 ? 16 : event.deltaMode === 2 ? window.innerHeight : 1);
    followCamera.zoomByWheel(pixels);
  };
  const pinchPointer = (event, phase) => {
    if (event.pointerType !== "touch") return;
    const rect = event.currentTarget.getBoundingClientRect();
    const [x, y] = [event.clientX - rect.left, event.clientY - rect.top];
    if (phase === "down") return pinchZoom.down(event.pointerId, x, y);
    if (phase !== "move") return pinchZoom.up(event.pointerId);
    const factor = pinchZoom.move(event.pointerId, x, y);
    if (factor === null || !cameraInputAllowed()) return;
    followCamera.setZoom(followCamera.zoom() * factor);
  };

  const stopMouseMovement = (pointerId) => {
    if (mousePointerIdRef.current !== pointerId) return;
    mousePointerIdRef.current = null;
    runtime.setMouseMovement(0, 0);
  };

  const aimWithPointer = (event) => {
    if (
      event.pointerType !== "mouse" ||
      overlayOpen ||
      inputRef.current?.getContext() !== "gameplay"
    )
      return;
    const aim = pointerAim(event);
    // Aim remains independent from locomotion. A pointer in the player dead zone
    // clears aim and stops right-button movement instead of walking indefinitely.
    if (mouseAim) runtime.setAim(aim);
    if (mousePointerIdRef.current !== event.pointerId) return;
    if ((event.buttons & 2) === 0) stopMouseMovement(event.pointerId);
    else {
      const [x, z] = movementFromAim(aim);
      runtime.setMouseMovement(x, z);
    }
  };

  const beginMouseMovement = (event) => {
    if (
      event.pointerType !== "mouse" ||
      event.button !== 2 ||
      overlayOpen ||
      !ready ||
      !inWorld ||
      inputRef.current?.getContext() !== "gameplay"
    )
      return;
    event.preventDefault();
    mousePointerIdRef.current = event.pointerId;
    event.currentTarget.setPointerCapture?.(event.pointerId);
    aimWithPointer(event);
  };

  const endMouseMovement = (event) => stopMouseMovement(event.pointerId);
  // Native mouseup fires for each button; pointerup fires only after all buttons
  // are released. Releasing RMB during an LMB hold must also stop walking.
  const mouseUp = (event) => {
    if (event.button === 2) stopMouseMovement(mousePointerIdRef.current);
  };

  // The follow camera owns the live zoom, so a settings change never restores a stale one
  // and "Reset display settings" leaves the zoom to the camera's own reset.
  const updatePresentation = (next) => {
    const normalized = normalizePresentation({
      ...next,
      camera: { zoom: followCamera.zoom() },
    });
    if (!persistPresentation(normalized)) setStatus(t("status.displaySession"));
    setPresentation(normalized);
  };
  const updateGraphics = (patch) =>
    updatePresentation({ ...presentation, graphics: { ...graphics, ...patch } });
  const updateAccessibility = (patch) =>
    updatePresentation({ ...presentation, accessibility: { ...accessibility, ...patch } });

  const updateSetupUrl = (value) => {
    setSetupUrl(value);
    if (!persistStoredValue(SETUP_URL_KEY, value)) setStatus(t("status.endpointSession"));
  };

  const updateDedicatedUrl = (value) => {
    setDedicatedUrl(value);
    if (!persistStoredValue(DEDICATED_URL_KEY, value)) setStatus(t("status.endpointSession"));
  };

  const selectCharacter = (characterId) => {
    setSelectedCharacterId(persistSelectedCharacterId(undefined, characterId));
  };

  const captureSaveDocument = () => {
    const captured = runtime.captureSaveState();
    if (!captured) {
      throw new Error("Saving is available for local games");
    }
    return createSaveDocument(captured.saveStateJson, {
      characterId: selectedCharacter.id,
      controlledPlayerId: captured.playerId,
    });
  };

  const applySaveDocument = (document, sourceLabel, persistImported = false) => {
    const controlledPlayerId = document.client.controlledPlayerId;
    const savedPlayer = document.coreState.players?.find(
      (candidate) => candidate.id === controlledPlayerId,
    );
    if (!savedPlayer) {
      throw new Error("Save file does not contain its controlled player");
    }

    const loadedGame = loadGameFromSaveStateJson(JSON.stringify(document.coreState));
    invalidateTouchGestures();
    runtime.restore({
      game: loadedGame,
      controlledPlayerId,
      lastSequence: savedPlayer.lastSequence ?? 0,
      movement: [savedPlayer.movement?.[0] ?? 0, savedPlayer.movement?.[1] ?? 0],
      guardHeld: savedPlayer.guard?.held === true,
      drawHeld: savedPlayer.drawTicks != null,
      aimHeld: savedPlayer.aim != null,
    });
    initialRunSeedRef.current = null;
    leaveTrainingUrl();
    setSelectedCharacterId(
      persistSelectedCharacterId(undefined, document.presentation.characterId),
    );
    setInWorld(true);
    setOverlay(PLAY);
    let notice = t("status.restored", {
      source: sourceLabel,
      tick: String(document.coreState.tick),
      seed: String(document.coreState.runSeed),
    });
    if (persistImported) {
      // The imported game is already authoritative; keeping a quick-save copy is optional.
      if (persistSaveDocument(undefined, document)) setSavedGameAvailable(true);
      else notice += t("status.notKept");
    }
    setStatus(notice);
  };

  const saveGameLocally = () => {
    try {
      const document = captureSaveDocument();
      if (!persistSaveDocument(undefined, document)) {
        setStatus(t("status.saveNoStorage"));
        return;
      }
      setSavedGameAvailable(true);
      setStatus(t("status.saved", { tick: String(document.coreState.tick) }));
    } catch (error) {
      setStatus(t("status.saveFailed", { error: String(error) }));
    }
  };

  const loadSavedGame = () => {
    try {
      const document = loadPersistedSaveDocument();
      if (!document) {
        setSavedGameAvailable(false);
        setStatus(t("status.noSave"));
        return;
      }
      applySaveDocument(document, t("status.loadedSave"));
    } catch (error) {
      setStatus(t("status.loadFailed", { error: String(error) }));
    }
  };

  const exportSave = () => {
    try {
      const saveDocument = captureSaveDocument();
      downloadJson(serializeSaveDocument(saveDocument), saveFileName(saveDocument));
      setStatus(t("status.exported", { tick: String(saveDocument.coreState.tick) }));
    } catch (error) {
      setStatus(t("status.exportFailed", { error: String(error) }));
    }
  };

  // The training session's accepted commands and ticks, replayable natively with
  // `replay_reproduction` (#101). Local training only; the runtime returns null elsewhere.
  const exportReproduction = () => {
    try {
      const json = runtime.exportReproduction();
      if (json === null) throw new Error("only local training sessions record a reproduction");
      const { scenario: recorded, seed, ticks } = JSON.parse(json);
      downloadJson(json, `arpg-reproduction-${recorded}-seed-${seed}-tick-${ticks}.json`);
      setStatus(t("status.reproduction", { scenario: recorded, count: ticks }));
    } catch (error) {
      setStatus(t("status.reproductionFailed", { error: String(error) }));
    }
  };

  const requestSaveImport = () => {
    saveFileInputRef.current?.click();
  };

  const importSave = async (event) => {
    const [file] = event.target.files ?? [];
    event.target.value = "";
    if (!file) return;
    try {
      if (file.size > MAX_SAVE_FILE_BYTES) {
        throw new Error(`Save file exceeds ${MAX_SAVE_FILE_BYTES} bytes`);
      }
      const document = parseSaveDocument(await file.text());
      applySaveDocument(document, t("status.importedSave", { file: file.name }), true);
    } catch (error) {
      setStatus(t("status.importFailed", { error: String(error) }));
    }
  };

  const copyInvite = async () => {
    const url = new URL(location.pathname, location.origin);
    url.searchParams.set("join", lobbyCode);
    try {
      await navigator.clipboard.writeText(url.toString());
      setStatus(t("status.inviteCopied"));
    } catch (error) {
      setStatus(t("status.inviteFailed", { error: String(error) }));
    }
  };

  const modeLabel = t(
    scenario === "training"
      ? "mode.training"
      : mode === "local"
        ? "mode.local"
        : mode === "host"
          ? "mode.host"
          : mode === "guest"
            ? "mode.guest"
            : "mode.dedicated",
  );

  return (
    <>
      {!inWorld && (
        <main className="character-select-shell" aria-label={t("select.region")}>
          <div className="character-select-atmosphere" aria-hidden="true" />
          <header className="character-select-header">
            <div>
              <span className="character-select-eyebrow">{t("app.name")}</span>
              <h1 className="visually-hidden">{t("select.region")}</h1>
            </div>
          </header>

          <div className="character-select-layout">
            <section className="character-stage" aria-live="polite">
              <div className="character-stage-ground" aria-hidden="true" />
              <div
                className={`character-avatar character-avatar-${selectedCharacter.tone}`}
                style={{ "--character-accent": selectedCharacter.accent }}
                aria-hidden="true"
              >
                <span className="character-aura" />
                <span className="character-head" />
                <span className="character-torso" />
                <span className="character-arm character-arm-left" />
                <span className="character-arm character-arm-right" />
                <span className="character-leg character-leg-left" />
                <span className="character-leg character-leg-right" />
                <span className="character-weapon" />
              </div>
              <div className="character-stage-copy">
                <span>{t(selectedCharacter.role)}</span>
                <h2>{selectedCharacter.name}</h2>
                <p>
                  {t("select.appearance", {
                    appearance: t(selectedCharacter.appearance),
                    location: t(selectedCharacter.location),
                  })}
                </p>
              </div>
            </section>

            <aside className="character-roster" aria-label={t("select.characters")}>
              <div className="character-roster-heading">
                <span>{t("select.characters")}</span>
                <strong>{CHARACTER_PRESETS.length}</strong>
              </div>
              <div className="character-roster-list">
                {CHARACTER_PRESETS.map((character) => {
                  const selected = character.id === selectedCharacter.id;
                  return (
                    <button
                      key={character.id}
                      type="button"
                      className={`character-card ${selected ? "is-selected" : ""}`}
                      aria-pressed={selected}
                      onClick={() => selectCharacter(character.id)}
                    >
                      <span
                        className={`character-card-portrait character-card-portrait-${character.tone}`}
                        style={{ "--character-accent": character.accent }}
                        aria-hidden="true"
                      />
                      <span className="character-card-copy">
                        <strong>{character.name}</strong>
                        <small>{t(character.role)}</small>
                      </span>
                      <span className="character-card-meta">{t(character.appearance)}</span>
                    </button>
                  );
                })}
              </div>
              <p className="character-selection-note">{t("select.note")}</p>
            </aside>
          </div>

          <footer className="character-select-footer">
            <span className="character-select-status" role="status">
              {ready && status === t("status.selectCharacter") ? t("select.ready") : shownStatus}
            </span>
            {coreFailed && (
              <button type="button" className="core-retry-button" onClick={retryCoreLoad}>
                {t("app.retry")}
              </button>
            )}
            <div className="character-select-save-actions">
              <button
                type="button"
                onClick={loadSavedGame}
                disabled={!ready || !savedGameAvailable}
              >
                {t("select.loadSaved")}
              </button>
              <button type="button" onClick={requestSaveImport} disabled={!ready}>
                {t("select.importSave")}
              </button>
              <button
                type="button"
                className="training-entry-button"
                onClick={() => enterTrainingArena(trainingSeedDraft)}
                disabled={!ready}
              >
                {t("select.training")}
              </button>
              <button
                type="button"
                className="enter-world-button"
                onClick={enterWorld}
                disabled={!ready}
              >
                {t("select.enterWorld")}
              </button>
            </div>
            <input
              ref={saveFileInputRef}
              className="save-file-input"
              type="file"
              accept=".json,application/json"
              aria-label={t("settings.saveFile")}
              onChange={importSave}
            />
          </footer>
        </main>
      )}
      <main hidden={!inWorld} className="game-shell">
        <GameCanvas
          snapshots={snapshotStore}
          playerId={playerId}
          accent={selectedCharacter.accent}
          graphics={graphics}
          effects={presentationEffects}
          camera={followCamera}
          cameraSmoothing={!trainingPaused}
          onWheel={zoomWithWheel}
          onPointerMove={(event) => {
            pinchPointer(event, "move");
            aimWithPointer(event);
          }}
          onPointerDown={(event) => {
            pinchPointer(event, "down");
            beginMouseMovement(event);
          }}
          onPointerUp={(event) => {
            pinchPointer(event, "up");
            endMouseMovement(event);
          }}
          onPointerCancel={(event) => {
            pinchPointer(event, "up");
            endMouseMovement(event);
          }}
          onLostPointerCapture={endMouseMovement}
          onMouseUp={mouseUp}
          onContextMenu={(event) => event.preventDefault()}
        />
        {inWorld && (
          <>
            <header className="game-header">
              <div>
                <strong>{t("app.name")}</strong>
                <span>{modeLabel}</span>
              </div>
              <div className="game-header-actions">
                {coreFailed && (
                  <button type="button" className="core-retry-button" onClick={retryCoreLoad}>
                    {t("app.retry")}
                  </button>
                )}
                <button type="button" aria-haspopup="dialog" onClick={() => setOverlay(openMenu())}>
                  {t("header.menu")}
                </button>
                <button
                  ref={settingsTriggerRef}
                  type="button"
                  onClick={() => setOverlay(openSettings("play"))}
                >
                  {t("header.settings")}
                </button>
              </div>
            </header>

            {scenario === "training" && (
              <aside className="training-panel" aria-label="Training arena controls">
                <header>
                  <strong>Training arena</strong>
                  <TrainingTick store={snapshotStore} />
                </header>

                <div className="training-actions">
                  <button type="button" onClick={() => runtime.setTrainingPaused(!trainingPaused)}>
                    {trainingPaused ? "Resume" : "Pause"}
                  </button>
                  <button type="button" onClick={stepTrainingArena} disabled={!trainingPaused}>
                    Step
                  </button>
                  <label>
                    <span>Speed</span>
                    <select
                      value={trainingSpeed}
                      onChange={(event) => runtime.setTrainingSpeed(Number(event.target.value))}
                    >
                      {TRAINING_SPEEDS.map((speed) => (
                        <option key={speed} value={speed}>
                          {speed}×
                        </option>
                      ))}
                    </select>
                  </label>
                </div>

                <label className="training-scenario">
                  <span>Scenario</span>
                  <select
                    value={trainingFixture}
                    onChange={(event) => {
                      const seed = parseRunSeed(trainingSeedDraft) ?? DEFAULT_TRAINING_SEED;
                      startTraining(seed, event.target.value);
                      setStatus(
                        t("status.trainingScenario", {
                          scenario: event.target.value,
                          seed: String(seed),
                        }),
                      );
                    }}
                  >
                    {SCENARIOS.map((option) => (
                      <option key={option.id} value={option.id}>
                        {option.label}
                      </option>
                    ))}
                  </select>
                </label>

                <div className="training-seed">
                  <label>
                    <span>Seed</span>
                    <input
                      type="number"
                      min="0"
                      max="4294967295"
                      step="1"
                      inputMode="numeric"
                      value={trainingSeedDraft}
                      onChange={(event) => setTrainingSeedDraft(event.target.value)}
                    />
                  </label>
                  <button
                    type="button"
                    onClick={() => restartTrainingArena(trainingSeedDraft)}
                    disabled={parseRunSeed(trainingSeedDraft) === null}
                  >
                    Restart
                  </button>
                  <button
                    type="button"
                    onClick={() => {
                      const seed = freshRunSeed();
                      setTrainingSeedDraft(String(seed));
                      restartTrainingArena(seed);
                    }}
                  >
                    New seed
                  </button>
                </div>

                <div className="training-actions">
                  <button type="button" onClick={exportReproduction}>
                    Export reproduction
                  </button>
                </div>

                <WorkbenchControls
                  key={client.sourceGeneration}
                  runtime={runtime}
                  store={snapshotStore}
                  setStatus={setStatus}
                />

                <TrainingDiagnostics store={snapshotStore} playerId={playerId} />
                <StrikeTimeline store={snapshotStore} />
              </aside>
            )}

            <PlayerHud
              store={snapshotStore}
              playerId={playerId}
              status={shownStatus}
              interactKey={interactKey}
              keyLabels={keyLabels}
              onReturnToCheckpoint={() => runtime.returnToCheckpoint()}
              onReturnToEntrance={returnToCharacters}
            />

            {ready && !overlayOpen && (
              <section className="mobile-controls" aria-label={t("touch.region")}>
                <div
                  ref={touchStickRef}
                  className="virtual-stick"
                  role="group"
                  aria-label={t("touch.joystick")}
                  aria-disabled={!playerId}
                  onPointerDown={beginTouchStick}
                  onPointerMove={updateTouchStick}
                  onPointerUp={endTouchStick}
                  onPointerCancel={endTouchStick}
                  onLostPointerCapture={endTouchStick}
                  onContextMenu={(event) => event.preventDefault()}
                >
                  <span ref={touchKnobRef} className="virtual-stick-knob" />
                </div>
              </section>
            )}

            {ready && !overlayOpen && (
              <CombatActions
                store={snapshotStore}
                playerId={playerId}
                triggerCombatAction={triggerCombatAction}
                gestureEpochRef={touchGestureEpochRef}
                setTouchGuard={(held) => runtime.setGuard("touch", held)}
                setTouchDraw={(held, interrupted = false) =>
                  runtime.setBowDraw("touch", held, { interrupted })
                }
                switchWeapon={switchWeapon}
                keyLabels={keyLabels}
              />
            )}

            {overlay.layer === "menu" && (
              <SettingsDialog
                key="menu"
                className="settings-panel game-menu"
                labelledBy="game-menu-heading"
                onClose={() => stepOverlay(closeLayer)}
              >
                <header>
                  <div>
                    <h1 id="game-menu-heading">{t("menu.heading")}</h1>
                    <p className="game-menu-notice" role="status">
                      {menuWorldNotice(menuPausesWorld(mode))}
                    </p>
                  </div>
                </header>
                <div className="game-menu-actions">
                  <button
                    type="button"
                    data-autofocus={overlay.focus === "resume" ? "" : undefined}
                    onClick={() => setOverlay(PLAY)}
                  >
                    {t("menu.resume")}
                  </button>
                  <button
                    type="button"
                    data-autofocus={overlay.focus === "settings" ? "" : undefined}
                    onClick={() => setOverlay(openSettings("menu"))}
                  >
                    {t("menu.settings")}
                  </button>
                  <button
                    type="button"
                    data-autofocus={overlay.focus === "leave" ? "" : undefined}
                    onClick={requestLeave}
                  >
                    {t("menu.leave")}
                  </button>
                </div>
              </SettingsDialog>
            )}

            {overlay.layer === "confirmLeave" && (
              <SettingsDialog
                key="confirm-leave"
                className="settings-panel game-menu"
                labelledBy="leave-heading"
                onClose={() => stepOverlay(closeLayer)}
              >
                <header>
                  <div>
                    <h1 id="leave-heading">{t("leave.heading")}</h1>
                    <p className="game-menu-notice">{leaveConsequence(mode)}</p>
                  </div>
                </header>
                <div className="game-menu-actions">
                  <button type="button" data-autofocus="" onClick={() => stepOverlay(closeLayer)}>
                    {t("leave.cancel")}
                  </button>
                  <button type="button" className="danger-button" onClick={returnToCharacters}>
                    {t("leave.confirm")}
                  </button>
                </div>
              </SettingsDialog>
            )}

            {settingsOpen && (
              <SettingsDialog
                key="settings"
                onClose={() => stepOverlay(closeLayer)}
                returnFocusTo={overlay.settingsFrom === "menu" ? undefined : settingsTriggerRef}
              >
                <header>
                  <div>
                    <h1 id="settings-heading">{t("settings.heading")}</h1>
                  </div>
                  <button type="button" autoFocus onClick={() => stepOverlay(closeLayer)}>
                    {t("settings.close")}
                  </button>
                </header>

                <section>
                  <h2>{t("settings.game")}</h2>
                  <GameSummary store={snapshotStore} mode={mode} playerId={playerId} />
                  <button type="button" onClick={startLocal}>
                    {t("settings.startLocal")}
                  </button>
                </section>

                <section>
                  <h2>{t("settings.save")}</h2>
                  <div className="save-state-summary">
                    <strong>
                      {t(savedGameAvailable ? "settings.saveAvailable" : "settings.saveNone")}
                    </strong>
                    <span>
                      {mode === "local" ? t("settings.saveLocal") : t("settings.saveRemote")}
                    </span>
                  </div>
                  <div className="settings-actions save-state-actions">
                    <button
                      type="button"
                      onClick={saveGameLocally}
                      disabled={mode !== "local" || !playerId}
                    >
                      {t("settings.saveGame")}
                    </button>
                    <button
                      type="button"
                      onClick={loadSavedGame}
                      disabled={!ready || !savedGameAvailable}
                    >
                      {t("settings.loadGame")}
                    </button>
                    <button
                      type="button"
                      onClick={exportSave}
                      disabled={mode !== "local" || !playerId}
                    >
                      {t("settings.exportSave")}
                    </button>
                    <button type="button" onClick={requestSaveImport} disabled={!ready}>
                      {t("settings.importSave")}
                    </button>
                  </div>
                  <input
                    ref={saveFileInputRef}
                    className="save-file-input"
                    type="file"
                    accept=".json,application/json"
                    aria-label={t("settings.saveFile")}
                    onChange={importSave}
                  />
                  <p className="settings-note">{t("settings.saveNote")}</p>
                </section>

                <section>
                  <h2>{t("settings.dedicated")}</h2>
                  <label>
                    {t("settings.endpoint")}
                    <input
                      value={dedicatedUrl}
                      onChange={(event) => updateDedicatedUrl(event.target.value)}
                      placeholder="https://127.0.0.1:4433/arpg"
                    />
                  </label>
                  <button
                    type="button"
                    onClick={startDedicated}
                    disabled={!ready || !dedicatedUrl.trim()}
                  >
                    {t("settings.connectDedicated")}
                  </button>
                </section>

                <section>
                  <h2>{t("settings.peer")}</h2>
                  <label>
                    {t("settings.setupUrl")}
                    <input
                      value={setupUrl}
                      onChange={(event) => updateSetupUrl(event.target.value)}
                    />
                  </label>
                  <div className="settings-actions">
                    <button type="button" onClick={hostPeerGame} disabled={!ready}>
                      {t("settings.host")}
                    </button>
                    {lobbyCode && (
                      <button type="button" onClick={copyInvite}>
                        {t("settings.copyInvite", { code: lobbyCode })}
                      </button>
                    )}
                  </div>
                  <label>
                    {t("settings.lobbyCode")}
                    <input value={joinCode} onChange={(event) => setJoinCode(event.target.value)} />
                  </label>
                  <button type="button" onClick={joinPeerGame} disabled={!joinCode.trim()}>
                    {t("settings.join")}
                  </button>
                </section>

                <section>
                  <h2>{t("settings.graphics")}</h2>
                  <label className="checkbox-row">
                    <input
                      type="checkbox"
                      checked={graphics.shadows}
                      onChange={(event) => updateGraphics({ shadows: event.target.checked })}
                    />
                    {t("settings.shadows")}
                  </label>
                  <label>
                    {t("settings.pixelRatio")}
                    <select
                      value={graphics.pixelRatioLimit}
                      onChange={(event) =>
                        updateGraphics({ pixelRatioLimit: Number(event.target.value) })
                      }
                    >
                      <option value="1">1×</option>
                      <option value="1.5">1.5×</option>
                      <option value="2">2×</option>
                    </select>
                  </label>
                </section>

                <section aria-labelledby="accessibility-heading">
                  <h2 id="accessibility-heading">{t("settings.accessibility")}</h2>
                  <div className="settings-field">
                    <label htmlFor="motion-effects">{t("settings.motion")}</label>
                    <select
                      id="motion-effects"
                      value={accessibility.motion}
                      onChange={(event) => updateAccessibility({ motion: event.target.value })}
                    >
                      {MOTION_PREFERENCES.map((motion) => (
                        <option key={motion} value={motion}>
                          {t(MOTION_LABELS[motion])}
                        </option>
                      ))}
                    </select>
                  </div>
                  <label className="checkbox-row">
                    <input
                      type="checkbox"
                      checked={accessibility.reducedFlashes}
                      onChange={(event) =>
                        updateAccessibility({ reducedFlashes: event.target.checked })
                      }
                    />
                    {t("settings.flashes")}
                  </label>
                  <div className="settings-field">
                    <label htmlFor="text-size">{t("settings.textSize")}</label>
                    <select
                      id="text-size"
                      value={accessibility.textScale}
                      onChange={(event) =>
                        updateAccessibility({ textScale: Number(event.target.value) })
                      }
                    >
                      {TEXT_SCALES.map((scale) => (
                        <option key={scale} value={scale}>
                          {formatPercent(scale)}
                        </option>
                      ))}
                    </select>
                  </div>
                  <p className="settings-note">{t("settings.displayNote")}</p>
                  <div className="settings-actions">
                    <button type="button" onClick={() => updatePresentation(DEFAULT_PRESENTATION)}>
                      {t("settings.resetDisplay")}
                    </button>
                  </div>
                </section>

                <section className="keybindings-section">
                  <h2>{t("settings.controls")}</h2>
                  <label className="checkbox-row">
                    <input
                      type="checkbox"
                      checked={mouseAim}
                      onChange={(event) => updateMouseAim(event.target.checked)}
                    />
                    {t("settings.mouseAim")}
                  </label>
                  <KeybindingEditor
                    registry={inputRegistry}
                    profile={profile}
                    onProfileChange={updateProfile}
                  />
                </section>
              </SettingsDialog>
            )}
          </>
        )}
      </main>
    </>
  );
}

const root = document.getElementById("root");
if (!root) throw new Error("Missing #root");
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
