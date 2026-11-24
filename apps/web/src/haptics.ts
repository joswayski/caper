/**
 * Caper's small set of haptics on phones, kept light so they confirm an action
 * rather than announce it. Android browsers use the Vibration API. iOS Safari
 * has none, but toggling a `switch` checkbox taps the Taptic Engine (iOS 18+)
 * during a tap, so the same calls work there except for arrivals, which happen
 * without a tap. Desktop pointers get nothing.
 */
const coarse = () => typeof window !== "undefined" && window.matchMedia("(pointer: coarse)").matches;

function buzz(ms: number) {
  if (!coarse()) return;
  try {
    if (typeof navigator.vibrate === "function") {
      navigator.vibrate(ms);
      return;
    }
    const label = document.createElement("label");
    label.ariaHidden = "true";
    label.style.display = "none";
    const input = document.createElement("input");
    input.type = "checkbox";
    input.setAttribute("switch", "");
    label.append(input);
    document.head.append(label);
    label.click();
    label.remove();
  } catch {
    /* Haptics are optional. */
  }
}

let lastArrival = 0;

export const haptics = {
  /** A send, a reaction, play or pause, a toggle, a copy. */
  tap: () => buzz(10),
  /** A long press taking hold, such as opening a message's actions. */
  hold: () => buzz(15),
  /** A refused action, such as attaching past the limit. */
  refuse: () => buzz(30),
  /**
   * Someone else's message arriving while the reader watches the
   * conversation: at most once every 1.5 seconds, and only while the page is visible.
   */
  arrival: () => {
    const now = performance.now();
    if (now - lastArrival < 1500 || document.visibilityState !== "visible") return;
    lastArrival = now;
    buzz(6);
  },
};
