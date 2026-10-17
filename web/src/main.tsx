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
import { aimFromPointer } from "./aim-input.js";
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
import "./styles.css";

const gameplayContext = { op: "context", id: "gameplay" };

const MOTION_LABELS = { system: "Follow system", reduced: "Reduced", full: "Full" };

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
    ["game.moveForward", "Move forward", "KeyW", "allow", "Movement"],
    ["game.moveBackward", "Move backward", "KeyS", "allow", "Movement"],
    ["game.moveLeft", "Move left", "KeyA", "allow", "Movement"],
    ["game.moveRight", "Move right", "KeyD", "allow", "Movement"],
    // The standard scheme (#59): Space jumps and R attacks. Older stored profiles migrate
    // once (`input-profile.js`); customized ones keep their bindings.
    ["game.jump", "Jump", "Space", "never", "Movement"],
    ["game.primaryAttack", "Primary attack", "KeyR", "never", "Combat"],
    ["game.secondaryAttack", "Heavy attack", "KeyQ", "never", "Combat"],
    ["game.interact", "Interact / pick up", "KeyE", "never", "Interaction"],
    ["game.guard", "Raise shield (hold)", "KeyF", "never", "Combat"],
    ["game.switchWeapon", "Switch sword / bow", "KeyX", "never", "Combat"],
    ["game.cycleTarget", "Lock / cycle target", "KeyT", "never", "Combat"],
    ["game.clearTarget", "Clear target lock", "KeyG", "never", "Combat"],
  ]
    .map(([id, title, code, repeatPolicy, category]) => ({
      id,
      title,
      categoryPath: ["ARPG", category],
      repeatPolicy,
      allowedDevices: ["keyboard"],
      defaults: [physical(`${id}.default`, id, code)],
      provenance: { source: "arpg", version: "1" },
    }))
    .concat([
      {
        id: "game.settings",
        title: "Open or close settings",
        categoryPath: ["ARPG", "System"],
        repeatPolicy: "never",
        allowedDevices: ["keyboard"],
        defaults: [
          physical("game.settings.default", "game.settings", "Escape", {
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
const characterAssets = createCharacterAssets({
  bundle: RANGER_BUNDLE,
  baseUrl: `${import.meta.env.BASE_URL}characters/`,
  fetch: (url) => fetch(url),
  // The character, its equipment items and its sockets (#183) from the same bundle.
  adapt: async (source, options) => {
    const character = await adaptImportedCharacterBundle(source, options);
    const { equipment: data } = RANGER_BUNDLE;
    const items = Object.fromEntries(
      await Promise.all(
        Object.entries(data.items).map(async ([id, item]) => [
          id,
          await adaptImportedItemBundle({ ...source, key: item.key, variant: "default" }),
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

const megabytes = (bytes) => (bytes / 1_000_000).toFixed(1);

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

function GameCanvas({ snapshots, playerId, accent, graphics, effects, onPointerMove }) {
  const canvasRef = useRef(null);
  const presenterRef = useRef(null);
  const { shadows, pixelRatioLimit } = graphics;
  // The view the presenter is (or will be) created with; later changes apply in place.
  const viewRef = useRef({ playerId, accent, graphics: { shadows, pixelRatioLimit }, effects });
  useEffect(() => {
    const presenter = createFramePresenter({
      canvas: canvasRef.current,
      snapshots,
      createRenderer: createThreeSceneRenderer,
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
    };
  }, [snapshots]);

  useEffect(() => {
    viewRef.current = { playerId, accent, graphics: { shadows, pixelRatioLimit }, effects };
    presenterRef.current?.configure(viewRef.current);
  }, [playerId, accent, shadows, pixelRatioLimit, effects]);

  return (
    <canvas
      ref={canvasRef}
      className="game-canvas"
      aria-label="ARPG game world"
      onPointerMove={onPointerMove}
    />
  );
}

function App() {
  const settingsTriggerRef = useRef(null);
  const touchStickRef = useRef(null);
  const touchKnobRef = useRef(null);
  const touchPointerIdRef = useRef(null);
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
  const [status, setStatus] = useState("Select a character");
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
        ? `Loading game… ${megabytes(characterProgress.loadedBytes)} / ${megabytes(characterProgress.totalBytes)} MB`
        : "Loading game…";
  const scenario = client.training ? "training" : "adventure";
  const trainingPaused = client.training?.paused ?? false;
  const trainingSpeed = client.training?.speed ?? 1;
  const currentPlayer = () =>
    snapshotStore.getSnapshot()?.players.find((player) => player.id === playerId) ?? null;
  const [settingsOpen, setSettingsOpen] = useState(false);
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
      // The settings toggle also closes the menu, so it stays live in menu context.
      alwaysActions: ["game.settings"],
      onDispatch: (dispatch) => {
        if (dispatch.action === "game.settings") {
          if (dispatch.phase === "press") setSettingsOpen((open) => !open);
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
  }, [runtime, focusedPlayer, switchWeapon, triggerCombatAction, releaseInput]);

  const leaveTrainingUrl = () => {
    history.replaceState(null, "", withTrainingRequest(location.href, false, 0));
  };

  const startTraining = (seed, fixture = trainingFixture) => {
    invalidateTouchGestures();
    runtime.startLocal({ seed, training: true, scenario: fixture });
    initialRunSeedRef.current = null;
    setTrainingSeedDraft(String(seed));
    setTrainingFixture(fixture);
    history.replaceState(null, "", withTrainingRequest(location.href, true, seed, fixture));
  };

  const enterTrainingArena = (candidateSeed = trainingSeedDraft) => {
    if (!ready) return;
    const seed = parseRunSeed(candidateSeed);
    if (seed === null) {
      setStatus("Training seed must be an unsigned 32-bit integer");
      return;
    }
    const selectedId = persistSelectedCharacterId(undefined, selectedCharacterId);
    const selected = resolveCharacter(selectedId);
    startTraining(seed);
    setInWorld(true);
    setStatus(`Training arena · ${selected.name}`);
  };

  const restartTrainingArena = (candidateSeed = trainingSeedDraft) => {
    const seed = parseRunSeed(candidateSeed);
    if (seed === null || scenario !== "training") {
      setStatus("Training seed must be an unsigned 32-bit integer");
      return;
    }
    startTraining(seed);
    setStatus(`Training arena restarted · seed ${seed}`);
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
    setStatus(`Local game · ${selected.name}`);
  };

  const returnToCharacters = () => {
    runtime.stop();
    invalidateTouchGestures();
    setSettingsOpen(false);
    setInWorld(false);
    leaveTrainingUrl();
    setStatus("Select a character");
  };

  const startLocal = () => {
    invalidateTouchGestures();
    runtime.startLocal();
    leaveTrainingUrl();
    setStatus(`Local game · ${selectedCharacter.name}`);
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
      history.replaceState(null, "", withTrainingRequest(location.href, true, seed, fixture));
      setStatus(
        unknownFixture
          ? `Unknown scenario "${unknownFixture}"; using the generated dungeon`
          : "Training arena",
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
      setStatus("Training arena");
      return;
    }
    setStatus("Select a character");
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
    inputRef.current?.setContext(focusOwner ?? (settingsOpen ? "menu" : "gameplay"));
  }, [focusOwner, settingsOpen]);

  const updateProfile = (next) => {
    if (!persistStoredValue(PROFILE_KEY, JSON.stringify(next)))
      setStatus("Controls updated for this session; browser storage is unavailable");
    setProfile(next);
  };

  const updateMouseAim = (enabled) => {
    if (!persistStoredValue(MOUSE_AIM_KEY, enabled ? "on" : "off"))
      setStatus("Controls updated for this session; browser storage is unavailable");
    setMouseAim(enabled);
  };

  useEffect(() => {
    // Turning mouse aim off returns melee and the bow to committed facing.
    if (!mouseAim) runtime.setAim(null);
  }, [mouseAim, runtime]);

  // Mouse aim is semantic direction intent only; the authority validates and applies it.
  // The last pointed direction holds while the pointer crosses HUD overlays; focus loss,
  // menus and disabling mouse aim return to committed facing.
  const aimWithPointer = (event) => {
    if (!mouseAim || event.pointerType !== "mouse" || settingsOpen) return;
    const rect = event.currentTarget.getBoundingClientRect();
    const aim = aimFromPointer(
      event.clientX - rect.left,
      event.clientY - rect.top,
      rect.width,
      rect.height,
    );
    // `null` inside the dead zone over the player returns to committed facing.
    runtime.setAim(aim);
  };

  const updatePresentation = (next) => {
    const normalized = normalizePresentation(next);
    if (!persistPresentation(normalized))
      setStatus("Display settings updated for this session; browser storage is unavailable");
    setPresentation(normalized);
  };
  const updateGraphics = (patch) =>
    updatePresentation({ ...presentation, graphics: { ...graphics, ...patch } });
  const updateAccessibility = (patch) =>
    updatePresentation({ ...presentation, accessibility: { ...accessibility, ...patch } });

  const updateSetupUrl = (value) => {
    setSetupUrl(value);
    if (!persistStoredValue(SETUP_URL_KEY, value))
      setStatus("Endpoint updated for this session; browser storage is unavailable");
  };

  const updateDedicatedUrl = (value) => {
    setDedicatedUrl(value);
    if (!persistStoredValue(DEDICATED_URL_KEY, value))
      setStatus("Endpoint updated for this session; browser storage is unavailable");
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
    setSettingsOpen(false);
    let notice = `${sourceLabel} · tick ${document.coreState.tick} · seed ${document.coreState.runSeed}`;
    if (persistImported) {
      // The imported game is already authoritative; keeping a quick-save copy is optional.
      if (persistSaveDocument(undefined, document)) setSavedGameAvailable(true);
      else notice += " · not kept as the local save because browser storage is unavailable";
    }
    setStatus(notice);
  };

  const saveGameLocally = () => {
    try {
      const document = captureSaveDocument();
      if (!persistSaveDocument(undefined, document)) {
        setStatus("Could not save game: browser storage is unavailable; export the save instead");
        return;
      }
      setSavedGameAvailable(true);
      setStatus(`Game saved locally · tick ${document.coreState.tick}`);
    } catch (error) {
      setStatus(`Could not save game: ${error}`);
    }
  };

  const loadSavedGame = () => {
    try {
      const document = loadPersistedSaveDocument();
      if (!document) {
        setSavedGameAvailable(false);
        setStatus("No local save is available");
        return;
      }
      applySaveDocument(document, "Loaded local save");
    } catch (error) {
      setStatus(`Could not load saved game: ${error}`);
    }
  };

  const exportSave = () => {
    try {
      const saveDocument = captureSaveDocument();
      downloadJson(serializeSaveDocument(saveDocument), saveFileName(saveDocument));
      setStatus(`Save exported · tick ${saveDocument.coreState.tick}`);
    } catch (error) {
      setStatus(`Could not export save: ${error}`);
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
      setStatus(`Reproduction exported · ${recorded} · ${ticks} ticks`);
    } catch (error) {
      setStatus(`Could not export reproduction: ${error}`);
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
      applySaveDocument(document, `Imported ${file.name}`, true);
    } catch (error) {
      setStatus(`Could not import save: ${error}`);
    }
  };

  const copyInvite = async () => {
    const url = new URL(location.pathname, location.origin);
    url.searchParams.set("join", lobbyCode);
    try {
      await navigator.clipboard.writeText(url.toString());
      setStatus("Invite URL copied");
    } catch (error) {
      setStatus(`Could not copy invite: ${error}`);
    }
  };

  const modeLabel =
    scenario === "training"
      ? "Training arena"
      : mode === "local"
        ? "Local"
        : mode === "host"
          ? "Peer host"
          : mode === "guest"
            ? "Peer guest"
            : "Dedicated online";

  if (!inWorld) {
    return (
      <main className="character-select-shell" aria-label="Character selection">
        <div className="character-select-atmosphere" aria-hidden="true" />
        <header className="character-select-header">
          <div>
            <span className="character-select-eyebrow">ARPG</span>
            <h1 className="visually-hidden">Character selection</h1>
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
              <span>{selectedCharacter.role}</span>
              <h2>{selectedCharacter.name}</h2>
              <p>
                {selectedCharacter.appearance} appearance · {selectedCharacter.location}
              </p>
            </div>
          </section>

          <aside className="character-roster" aria-label="Characters">
            <div className="character-roster-heading">
              <span>Characters</span>
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
                      <small>{character.role}</small>
                    </span>
                    <span className="character-card-meta">{character.appearance}</span>
                  </button>
                );
              })}
            </div>
            <p className="character-selection-note">
              Appearance profiles currently share the same authoritative gameplay rules.
            </p>
          </aside>
        </div>

        <footer className="character-select-footer">
          <span className="character-select-status" role="status">
            {ready && status === "Select a character" ? "Ready" : shownStatus}
          </span>
          {coreFailed && (
            <button type="button" className="core-retry-button" onClick={retryCoreLoad}>
              Retry
            </button>
          )}
          <div className="character-select-save-actions">
            <button type="button" onClick={loadSavedGame} disabled={!ready || !savedGameAvailable}>
              Load Saved Game
            </button>
            <button type="button" onClick={requestSaveImport} disabled={!ready}>
              Import Save
            </button>
            <button
              type="button"
              className="training-entry-button"
              onClick={() => enterTrainingArena(trainingSeedDraft)}
              disabled={!ready}
            >
              Training Arena
            </button>
            <button
              type="button"
              className="enter-world-button"
              onClick={enterWorld}
              disabled={!ready}
            >
              Enter World
            </button>
          </div>
          <input
            ref={saveFileInputRef}
            className="save-file-input"
            type="file"
            accept=".json,application/json"
            aria-label="Save file"
            onChange={importSave}
          />
        </footer>
      </main>
    );
  }

  return (
    <main className="game-shell">
      <GameCanvas
        snapshots={snapshotStore}
        playerId={playerId}
        accent={selectedCharacter.accent}
        graphics={graphics}
        effects={presentationEffects}
        onPointerMove={aimWithPointer}
      />
      <header className="game-header">
        <div>
          <strong>ARPG</strong>
          <span>{modeLabel}</span>
        </div>
        <div className="game-header-actions">
          {coreFailed && (
            <button type="button" className="core-retry-button" onClick={retryCoreLoad}>
              Retry
            </button>
          )}
          <button type="button" onClick={returnToCharacters}>
            Characters
          </button>
          <button ref={settingsTriggerRef} type="button" onClick={() => setSettingsOpen(true)}>
            Settings
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
                setStatus(`Training scenario · ${event.target.value} · seed ${seed}`);
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
      />

      {ready && !settingsOpen && (
        <section className="mobile-controls" aria-label="Touch controls">
          <div
            ref={touchStickRef}
            className="virtual-stick"
            role="group"
            aria-label="Movement joystick"
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

      {ready && !settingsOpen && (
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

      {settingsOpen && (
        <SettingsDialog onClose={() => setSettingsOpen(false)} returnFocusTo={settingsTriggerRef}>
          <header>
            <div>
              <h1 id="settings-heading">Settings</h1>
            </div>
            <button type="button" autoFocus onClick={() => setSettingsOpen(false)}>
              Close
            </button>
          </header>

          <section>
            <h2>Game</h2>
            <GameSummary store={snapshotStore} mode={mode} playerId={playerId} />
            <button type="button" onClick={startLocal}>
              Start local game
            </button>
          </section>

          <section>
            <h2>Save & load</h2>
            <div className="save-state-summary">
              <strong>{savedGameAvailable ? "Local save available" : "No local save yet"}</strong>
              <span>
                {mode === "local"
                  ? "Current local authority can be saved or exported."
                  : "Loading a save returns to local play; network sessions are not overwritten."}
              </span>
            </div>
            <div className="settings-actions save-state-actions">
              <button
                type="button"
                onClick={saveGameLocally}
                disabled={mode !== "local" || !playerId}
              >
                Save game
              </button>
              <button
                type="button"
                onClick={loadSavedGame}
                disabled={!ready || !savedGameAvailable}
              >
                Load saved game
              </button>
              <button type="button" onClick={exportSave} disabled={mode !== "local" || !playerId}>
                Export save
              </button>
              <button type="button" onClick={requestSaveImport} disabled={!ready}>
                Import save
              </button>
            </div>
            <input
              ref={saveFileInputRef}
              className="save-file-input"
              type="file"
              accept=".json,application/json"
              aria-label="Save file"
              onChange={importSave}
            />
            <p className="settings-note">
              Saves are versioned and validated by the Rust authority before they replace the
              current game. Exported JSON can be imported on another browser or device.
            </p>
          </section>

          <section>
            <h2>Dedicated online</h2>
            <label>
              WebTransport endpoint
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
              Connect dedicated server
            </button>
          </section>

          <section>
            <h2>Peer co-op</h2>
            <label>
              Setup service URL
              <input value={setupUrl} onChange={(event) => updateSetupUrl(event.target.value)} />
            </label>
            <div className="settings-actions">
              <button type="button" onClick={hostPeerGame} disabled={!ready}>
                Host co-op
              </button>
              {lobbyCode && (
                <button type="button" onClick={copyInvite}>
                  Copy invite {lobbyCode}
                </button>
              )}
            </div>
            <label>
              Lobby code
              <input value={joinCode} onChange={(event) => setJoinCode(event.target.value)} />
            </label>
            <button type="button" onClick={joinPeerGame} disabled={!joinCode.trim()}>
              Join host
            </button>
          </section>

          <section>
            <h2>Graphics</h2>
            <label className="checkbox-row">
              <input
                type="checkbox"
                checked={graphics.shadows}
                onChange={(event) => updateGraphics({ shadows: event.target.checked })}
              />
              Enable shadows
            </label>
            <label>
              Pixel ratio limit
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
            <h2 id="accessibility-heading">Accessibility</h2>
            <div className="settings-field">
              <label htmlFor="motion-effects">Motion effects</label>
              <select
                id="motion-effects"
                value={accessibility.motion}
                onChange={(event) => updateAccessibility({ motion: event.target.value })}
              >
                {MOTION_PREFERENCES.map((motion) => (
                  <option key={motion} value={motion}>
                    {MOTION_LABELS[motion]}
                  </option>
                ))}
              </select>
            </div>
            <label className="checkbox-row">
              <input
                type="checkbox"
                checked={accessibility.reducedFlashes}
                onChange={(event) => updateAccessibility({ reducedFlashes: event.target.checked })}
              />
              Reduce bright flashes
            </label>
            <div className="settings-field">
              <label htmlFor="text-size">Text size</label>
              <select
                id="text-size"
                value={accessibility.textScale}
                onChange={(event) => updateAccessibility({ textScale: Number(event.target.value) })}
              >
                {TEXT_SCALES.map((scale) => (
                  <option key={scale} value={scale}>
                    {`${scale * 100}%`}
                  </option>
                ))}
              </select>
            </div>
            <p className="settings-note">
              Display settings stay in this browser. They never change saves or game rules.
            </p>
            <div className="settings-actions">
              <button type="button" onClick={() => updatePresentation(DEFAULT_PRESENTATION)}>
                Reset display settings
              </button>
            </div>
          </section>

          <section className="keybindings-section">
            <h2>Controls</h2>
            <label className="checkbox-row">
              <input
                type="checkbox"
                checked={mouseAim}
                onChange={(event) => updateMouseAim(event.target.checked)}
              />
              Aim melee and bow with the mouse
            </label>
            <KeybindingEditor
              registry={inputRegistry}
              profile={profile}
              onProfileChange={updateProfile}
            />
          </section>
        </SettingsDialog>
      )}
    </main>
  );
}

const root = document.getElementById("root");
if (!root) throw new Error("Missing #root");
createRoot(root).render(
  <StrictMode>
    <App />
  </StrictMode>,
);
