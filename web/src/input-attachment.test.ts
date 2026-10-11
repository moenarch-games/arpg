// Acceptance contract for the keyboard input attachment owned outside React (#88).
//
// `web/src/input-attachment.ts` owns one input-bindings controller and its browser keyboard
// attachment for the game client runtime. The composition attaches it when the world is
// entered and detaches it when the world is left; profile changes and the gameplay/menu
// context are applied in place instead of rebuilding the attachment in a React effect.
// Device semantics (key normalization, chords, text-entry filtering, gamepad/touch
// unification) stay in `input-bindings` and #57; this owner only manages lifetime and
// context.
//
// API (the input-bindings constructors are injected: the vendored packages are not
// resolvable in unit tests, and the composition supplies the real ones):
//
//   createInputAttachment({
//     profile,                          // initial input-bindings profile (may be undefined)
//     createController({ profile, getActiveContexts, onDispatch }),
//                                       // => controller with updateProfile(profile);
//                                       //    real: new InputRuntimeController({ registry,
//                                       //    consumePolicy: "matched", ...options })
//     attachKeyboard(controller),       // => detach(); real: attachKeyboardRuntime(controller,
//                                       //    { mode: "physical", ignoreTextEntry: true,
//                                       //      stopPropagation: true })
//     onDispatch(dispatch),             // gameplay dispatches, routed to the runtime
//     onRelease(),                      // clears held input (runtime.releaseInput + touch)
//     focusTarget,                      // EventTarget for "blur" (window)
//     visibilityTarget,                 // EventTarget for "visibilitychange" with
//                                       //   visibilityState (document)
//   }) => {
//     attach(),                         // idempotent
//     detach(),                         // idempotent; removes every listener
//     setContext("gameplay" | "menu"),
//     getContext(),
//     setProfile(profile),              // in place, attached or not
//     isAttached(),
//   }
//
// Behaviour:
// - The initial context is "gameplay". While attached, at most one keyboard attachment and
//   one blur / visibilitychange listener exist.
// - In "gameplay" the controller's active contexts contain "gameplay" and dispatches reach
//   `onDispatch`. In "menu" they do not contain "gameplay" and no dispatch reaches
//   `onDispatch`; switching gameplay → menu releases held input once.
// - Window blur and a hidden document release held input while attached.
// - `detach()` releases held input, detaches the keyboard and removes the blur and
//   visibility listeners; dispatches from the old controller are dropped afterwards.
import { expect, test } from "bun:test";
import { createInputAttachment } from "./input-attachment.ts";

class CountingTarget {
  listeners = new Map();
  visibilityState = "visible";

  addEventListener(type, listener) {
    if (!this.listeners.has(type)) this.listeners.set(type, new Set());
    this.listeners.get(type).add(listener);
  }

  removeEventListener(type, listener) {
    this.listeners.get(type)?.delete(listener);
  }

  count(type) {
    if (type) return this.listeners.get(type)?.size ?? 0;
    let total = 0;
    for (const set of this.listeners.values()) total += set.size;
    return total;
  }

  dispatch(type, event = {}) {
    for (const listener of [...(this.listeners.get(type) ?? [])]) listener({ type, ...event });
  }
}

function harness({ profile = { id: "initial" } } = {}) {
  const keyTarget = new CountingTarget();
  const focusTarget = new CountingTarget();
  const visibilityTarget = new CountingTarget();
  const controllers = [];
  const keyboardAttachments = [];
  const dispatched = [];
  let releases = 0;

  const options = {
    profile,
    createController(controllerOptions) {
      const controller = {
        options: controllerOptions,
        profile: controllerOptions.profile,
        profileUpdates: 0,
        updateProfile(next) {
          controller.profile = next;
          controller.profileUpdates += 1;
        },
        activeContexts: () => [...controllerOptions.getActiveContexts()],
        emit(dispatch) {
          controllerOptions.onDispatch(dispatch);
        },
      };
      controllers.push(controller);
      return controller;
    },
    attachKeyboard(controller) {
      const onKey = () => {};
      keyTarget.addEventListener("keydown", onKey);
      keyTarget.addEventListener("keyup", onKey);
      const attachment = { controller, detached: 0 };
      keyboardAttachments.push(attachment);
      return () => {
        attachment.detached += 1;
        keyTarget.removeEventListener("keydown", onKey);
        keyTarget.removeEventListener("keyup", onKey);
      };
    },
    onDispatch: (dispatch) => dispatched.push(dispatch),
    onRelease: () => {
      releases += 1;
    },
    focusTarget,
    visibilityTarget,
  };

  const listenerCounts = () => ({
    keyboard: keyTarget.count(),
    blur: focusTarget.count("blur"),
    visibility: visibilityTarget.count("visibilitychange"),
  });

  return {
    options,
    keyTarget,
    focusTarget,
    visibilityTarget,
    controllers,
    keyboardAttachments,
    dispatched,
    releases: () => releases,
    listenerCounts,
    liveKeyboardAttachments: () =>
      keyboardAttachments.filter((attachment) => attachment.detached === 0),
    liveController: () =>
      keyboardAttachments.find((attachment) => attachment.detached === 0)?.controller,
  };
}

const press = (action) => ({ action, phase: "press" });

test("attach creates one keyboard attachment with blur and visibility listeners", () => {
  const h = harness();
  const input = createInputAttachment(h.options);
  expect(input.isAttached()).toBe(false);
  expect(h.listenerCounts()).toEqual({ keyboard: 0, blur: 0, visibility: 0 });

  input.attach();
  input.attach();

  expect(input.isAttached()).toBe(true);
  expect(h.liveKeyboardAttachments()).toHaveLength(1);
  expect(h.listenerCounts()).toEqual({ keyboard: 2, blur: 1, visibility: 1 });
  expect(h.liveController().profile).toEqual({ id: "initial" });
});

test("gameplay context activates gameplay bindings and forwards dispatches", () => {
  const h = harness();
  const input = createInputAttachment(h.options);
  input.attach();

  expect(input.getContext()).toBe("gameplay");
  expect(h.liveController().activeContexts()).toContain("gameplay");
  h.liveController().emit(press("game.moveForward"));

  expect(h.dispatched).toEqual([press("game.moveForward")]);
});

test("menu context suppresses gameplay commands and releases held input once", () => {
  const h = harness();
  const input = createInputAttachment(h.options);
  input.attach();
  const releasesBefore = h.releases();

  input.setContext("menu");
  input.setContext("menu");

  expect(input.getContext()).toBe("menu");
  expect(h.releases() - releasesBefore).toBe(1);
  expect(h.liveController().activeContexts()).not.toContain("gameplay");
  h.liveController().emit(press("game.primaryAttack"));
  h.liveController().emit({ action: "game.moveForward", phase: "release" });
  expect(h.dispatched).toEqual([]);

  input.setContext("gameplay");
  expect(h.liveController().activeContexts()).toContain("gameplay");
  h.liveController().emit(press("game.primaryAttack"));
  expect(h.dispatched).toEqual([press("game.primaryAttack")]);
});

test("context changes keep the same attachment and listener counts", () => {
  const h = harness();
  const input = createInputAttachment(h.options);
  input.attach();
  const counts = h.listenerCounts();

  for (let round = 0; round < 5; round += 1) {
    input.setContext("menu");
    input.setContext("gameplay");
  }

  expect(h.keyboardAttachments).toHaveLength(1);
  expect(h.controllers).toHaveLength(1);
  expect(h.listenerCounts()).toEqual(counts);
});

test("a profile change while attached updates the live controller in place", () => {
  const h = harness();
  const input = createInputAttachment(h.options);
  input.attach();
  const counts = h.listenerCounts();
  const controller = h.liveController();

  input.setProfile({ id: "rebound" });
  input.setProfile({ id: "rebound-again" });

  expect(h.controllers).toHaveLength(1);
  expect(h.keyboardAttachments).toHaveLength(1);
  expect(h.liveController()).toBe(controller);
  expect(controller.profile).toEqual({ id: "rebound-again" });
  expect(controller.profileUpdates).toBe(2);
  expect(h.listenerCounts()).toEqual(counts);
});

test("a profile change while detached applies on the next attach", () => {
  const h = harness();
  const input = createInputAttachment(h.options);
  input.attach();
  input.detach();

  input.setProfile({ id: "changed-in-menu" });
  input.attach();

  expect(h.liveController().profile).toEqual({ id: "changed-in-menu" });
});

test("focus loss and a hidden document release held input while attached", () => {
  const h = harness();
  const input = createInputAttachment(h.options);
  input.attach();
  const releasesBefore = h.releases();

  h.focusTarget.dispatch("blur");
  expect(h.releases() - releasesBefore).toBe(1);

  h.visibilityTarget.visibilityState = "visible";
  h.visibilityTarget.dispatch("visibilitychange");
  expect(h.releases() - releasesBefore).toBe(1);

  h.visibilityTarget.visibilityState = "hidden";
  h.visibilityTarget.dispatch("visibilitychange");
  expect(h.releases() - releasesBefore).toBe(2);
});

test("detach removes every listener, releases held input and drops late dispatches", () => {
  const h = harness();
  const input = createInputAttachment(h.options);
  input.attach();
  const controller = h.liveController();
  const releasesBefore = h.releases();

  input.detach();

  expect(input.isAttached()).toBe(false);
  expect(h.listenerCounts()).toEqual({ keyboard: 0, blur: 0, visibility: 0 });
  expect(h.keyboardAttachments[0].detached).toBe(1);
  expect(h.releases() - releasesBefore).toBe(1);

  controller.emit(press("game.primaryAttack"));
  expect(h.dispatched).toEqual([]);

  h.focusTarget.dispatch("blur");
  expect(h.releases() - releasesBefore).toBe(1);
});

test("detach is idempotent and safe before attach", () => {
  const h = harness();
  const input = createInputAttachment(h.options);
  input.detach();
  expect(h.releases()).toBe(0);

  input.attach();
  input.detach();
  input.detach();

  expect(h.keyboardAttachments[0].detached).toBe(1);
  expect(h.releases()).toBe(1);
  expect(h.listenerCounts()).toEqual({ keyboard: 0, blur: 0, visibility: 0 });
});

test("repeated enter and leave keeps listener counts flat", () => {
  const h = harness();
  const input = createInputAttachment(h.options);
  for (let round = 0; round < 20; round += 1) {
    input.attach();
    expect(h.liveKeyboardAttachments()).toHaveLength(1);
    expect(h.listenerCounts()).toEqual({ keyboard: 2, blur: 1, visibility: 1 });
    input.setProfile({ id: `round-${round}` });
    input.setContext(round % 2 === 0 ? "menu" : "gameplay");
    input.detach();
    expect(h.listenerCounts()).toEqual({ keyboard: 0, blur: 0, visibility: 0 });
  }

  expect(h.liveKeyboardAttachments()).toHaveLength(0);
  expect(h.keyboardAttachments.every((attachment) => attachment.detached === 1)).toBe(true);
  // Old controllers can no longer reach the runtime.
  for (const controller of h.controllers) controller.emit(press("game.interact"));
  expect(h.dispatched).toEqual([]);
});
