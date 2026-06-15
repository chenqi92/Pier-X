import type { MouseEvent as ReactMouseEvent } from "react";

const SHAKE_CLASS = "is-dialog-shaking";
const SHAKE_TARGET_SELECTOR = ".dlg, .cmdp, .ws-newdialog, [data-dialog-shake-target]";
const SHAKE_FALLBACK_MS = 600;

type ActiveShake = {
  onAnimationEnd: (event: AnimationEvent) => void;
  timeoutId: number;
};

const activeShakes = new WeakMap<HTMLElement, ActiveShake>();

function cancelActiveShake(dialog: HTMLElement) {
  const active = activeShakes.get(dialog);
  if (!active) return;
  window.clearTimeout(active.timeoutId);
  dialog.removeEventListener("animationend", active.onAnimationEnd);
  activeShakes.delete(dialog);
}

export function shakeDialogOverlay(event: ReactMouseEvent<HTMLElement>) {
  if (event.target !== event.currentTarget) return;

  const overlay = event.currentTarget;
  const dialog = overlay.querySelector<HTMLElement>(SHAKE_TARGET_SELECTOR);
  if (!dialog) return;

  cancelActiveShake(dialog);
  dialog.classList.remove(SHAKE_CLASS);
  void dialog.offsetWidth;
  dialog.classList.add(SHAKE_CLASS);

  const finish = () => {
    cancelActiveShake(dialog);
    dialog.classList.remove(SHAKE_CLASS);
  };

  const onAnimationEnd = (animationEvent: AnimationEvent) => {
    if (animationEvent.target === dialog) finish();
  };
  const timeoutId = window.setTimeout(finish, SHAKE_FALLBACK_MS);

  dialog.addEventListener("animationend", onAnimationEnd);
  activeShakes.set(dialog, { onAnimationEnd, timeoutId });
}
