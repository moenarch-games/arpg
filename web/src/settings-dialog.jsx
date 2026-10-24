import { useEffect, useRef } from "react";

// A modal layer of the in-world shell (settings, game menu, leave confirmation). It owns
// focus while open and returns it to `returnFocusTo` when it closes.
export function SettingsDialog({
  children,
  onClose,
  returnFocusTo,
  labelledBy = "settings-heading",
  className = "settings-panel",
}) {
  const dialogRef = useRef(null);
  useEffect(() => {
    const dialog = dialogRef.current;
    // Without an explicit opener, focus returns to whatever held it when the layer opened.
    const focusTarget = returnFocusTo?.current ?? document.activeElement;
    dialog.showModal();
    dialog.querySelector("[data-autofocus]")?.focus();
    return () => {
      dialog.close();
      focusTarget?.focus();
    };
  }, [returnFocusTo]);

  return (
    <dialog
      ref={dialogRef}
      className={className}
      aria-labelledby={labelledBy}
      onCancel={(event) => {
        event.preventDefault();
        onClose();
      }}
      onClick={(event) => {
        if (event.target !== event.currentTarget) return;
        const rect = event.currentTarget.getBoundingClientRect();
        if (
          event.clientX < rect.left ||
          event.clientX > rect.right ||
          event.clientY < rect.top ||
          event.clientY > rect.bottom
        )
          onClose();
      }}
    >
      {children}
    </dialog>
  );
}
