import data from "@emoji-mart/data";
import { init } from "emoji-mart";

// emoji-mart treats a persisted empty object differently from a missing index:
// a missing index gets its default Frequent row, while `{}` removes the entire
// category from the module-global picker data. Normalize that poisoned state
// before the first picker initializes.
try {
  if (window.localStorage.getItem("emoji-mart.frequently") === "{}") {
    window.localStorage.removeItem("emoji-mart.frequently");
    window.localStorage.removeItem("emoji-mart.last");
  }
} catch {
  // emoji-mart also tolerates unavailable storage; picker selection still works.
}

// emoji-mart synchronously builds its search index inside `init`. Warm it at
// idle so the first picker open does not pay that cost. This module also owns
// the data passed to every Picker, making the prewarm part of that import path
// rather than a disconnected best-effort call.
const warm = () => void init({ data });
if (typeof window !== "undefined" && "requestIdleCallback" in window) {
  window.requestIdleCallback(warm, { timeout: 1_500 });
} else {
  globalThis.setTimeout(warm, 250);
}

export { data as emojiMartData };
