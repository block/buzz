import * as React from "react";

import { hasPrimaryShortcutModifier } from "@/shared/lib/platform";

export type ComposerAddressPrototype = "mention-button" | "send-button";

let prototype: ComposerAddressPrototype = "mention-button";
const listeners = new Set<() => void>();
let mountedConsumers = 0;

function emit() {
  for (const listener of listeners) listener();
}

function togglePrototype() {
  prototype = prototype === "mention-button" ? "send-button" : "mention-button";
  emit();
}

function handlePrototypeShortcut(event: KeyboardEvent) {
  if (
    event.defaultPrevented ||
    event.repeat ||
    event.key.toLowerCase() !== "m" ||
    !event.shiftKey ||
    !hasPrimaryShortcutModifier(event) ||
    event.altKey
  ) {
    return;
  }
  event.preventDefault();
  togglePrototype();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function getSnapshot() {
  return prototype;
}

export function useComposerAddressPrototype(): ComposerAddressPrototype {
  const value = React.useSyncExternalStore(subscribe, getSnapshot, getSnapshot);

  React.useEffect(() => {
    mountedConsumers += 1;
    if (mountedConsumers === 1) {
      window.addEventListener("keydown", handlePrototypeShortcut);
    }
    return () => {
      mountedConsumers -= 1;
      if (mountedConsumers === 0) {
        window.removeEventListener("keydown", handlePrototypeShortcut);
      }
    };
  }, []);

  return value;
}
