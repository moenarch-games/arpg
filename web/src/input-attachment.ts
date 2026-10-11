// Keyboard input attachment owner for the game client (#88). It owns one input-bindings
// controller and its browser keyboard attachment: the composition attaches it when the world
// is entered and detaches it when the world is left. Profile changes and the gameplay/menu
// context apply in place instead of rebuilding the attachment.
//
// Device semantics (key normalization, chords, text-entry filtering, gamepad/touch
// unification) stay in `input-bindings`; this owner manages lifetime and context only. The
// input-bindings constructors are injected by the composition.
//
// Held keys do not survive focus loss or a hidden document (the release may never arrive),
// leaving gameplay for a menu, or detaching: each of these calls `onRelease`.

export type InputContext = "gameplay" | "menu";

export interface InputDispatch {
  action: string;
  phase: string;
}

export interface InputControllerLike<Profile> {
  updateProfile(profile: Profile | undefined): void;
}

export interface VisibilityTargetLike extends EventTarget {
  visibilityState?: string;
}

export interface InputAttachmentOptions<Profile, Controller extends InputControllerLike<Profile>> {
  profile: Profile | undefined;
  createController(options: {
    profile: Profile | undefined;
    getActiveContexts: () => ReadonlySet<string>;
    onDispatch: (dispatch: InputDispatch) => void;
  }): Controller;
  attachKeyboard(controller: Controller): () => void;
  onDispatch(dispatch: InputDispatch): void;
  onRelease(): void;
  focusTarget: EventTarget;
  visibilityTarget: VisibilityTargetLike;
  // Actions whose bindings apply in every context (the settings toggle, which also closes
  // the menu). Their dispatches still reach `onDispatch` in "menu".
  alwaysActions?: Iterable<string>;
}

export function createInputAttachment<Profile, Controller extends InputControllerLike<Profile>>(
  options: InputAttachmentOptions<Profile, Controller>,
) {
  const alwaysActions = new Set(options.alwaysActions ?? []);
  let profile = options.profile;
  let context: InputContext = "gameplay";
  let controller: Controller | null = null;
  let detachKeyboard: (() => void) | null = null;

  const activeContexts = () => new Set([context]);

  const onBlur = () => options.onRelease();
  const onVisibility = () => {
    if (options.visibilityTarget.visibilityState === "hidden") options.onRelease();
  };

  return {
    attach() {
      if (controller) return;
      const owned = options.createController({
        profile,
        getActiveContexts: activeContexts,
        onDispatch: (dispatch) => {
          // A detached or replaced controller can no longer reach the runtime.
          if (controller !== owned) return;
          if (context !== "gameplay" && !alwaysActions.has(dispatch.action)) return;
          options.onDispatch(dispatch);
        },
      });
      controller = owned;
      detachKeyboard = options.attachKeyboard(owned);
      options.focusTarget.addEventListener("blur", onBlur);
      options.visibilityTarget.addEventListener("visibilitychange", onVisibility);
    },

    detach() {
      if (!controller) return;
      options.focusTarget.removeEventListener("blur", onBlur);
      options.visibilityTarget.removeEventListener("visibilitychange", onVisibility);
      const detach = detachKeyboard;
      detachKeyboard = null;
      controller = null;
      detach?.();
      options.onRelease();
    },

    setContext(next: InputContext) {
      if (next === context) return;
      const leavingGameplay = context === "gameplay";
      context = next;
      if (leavingGameplay && controller) options.onRelease();
    },

    getContext: () => context,

    setProfile(next: Profile | undefined) {
      if (Object.is(next, profile)) return;
      profile = next;
      controller?.updateProfile(next);
    },

    isAttached: () => controller !== null,
  };
}

export type InputAttachment = ReturnType<typeof createInputAttachment>;
