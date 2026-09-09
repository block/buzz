import { useCallback, useSyncExternalStore } from "react";

const STORAGE_KEY = "buzz-next-color-scheme";

export type ColorScheme = "light" | "dark";

function preferred(): ColorScheme {
  const stored = localStorage.getItem(STORAGE_KEY);
  if (stored === "light" || stored === "dark") return stored;
  return window.matchMedia("(prefers-color-scheme: dark)").matches
    ? "dark"
    : "light";
}

/**
 * The one answer to "light or dark".
 *
 * This is a fact, so it owns no policy — but a fact still has to have a *single*
 * value, and this one did not. The hook held `useState`, which gives every
 * caller an independent copy: the shell's toggle updated the shell's copy and
 * nothing else. `<html class="dark">` flipped, so anything reading the class
 * through CSS looked correct, and the defect stayed invisible until a consumer
 * needed the value in JavaScript. `DockWorkspace` is that consumer — dockview
 * takes its theme as a JS object, and it kept receiving the light one on a dark
 * page, which is the same class of bug as the vendor-white-sheet it replaced.
 * `ComponentAnatomy` had already hit this and worked around it locally by
 * observing the class with a `MutationObserver`.
 *
 * So the value lives in one module-level place and every caller subscribes to
 * it. Module-level state normally has to register teardown for a community
 * switch; appearance is a property of the person, not of the community, so it
 * deliberately survives one.
 *
 * The class on `<html>` is written where the value changes rather than in an
 * Effect per consumer, because the DOM is one external system with one owner
 * and N subscribers must not each try to write it.
 */
let current: ColorScheme | undefined;
const listeners = new Set<() => void>();

function read(): ColorScheme {
  if (current === undefined) current = preferred();
  return current;
}

function apply(scheme: ColorScheme) {
  document.documentElement.classList.toggle("dark", scheme === "dark");
}

function set(scheme: ColorScheme) {
  if (scheme === read()) return;
  current = scheme;
  localStorage.setItem(STORAGE_KEY, scheme);
  apply(scheme);
  for (const listener of listeners) listener();
}

function subscribe(listener: () => void) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

/** Paint the stored choice before React mounts, so there is no light flash. */
export function applyStoredColorScheme() {
  apply(read());
}

export function useColorScheme() {
  const scheme = useSyncExternalStore(subscribe, read);
  const toggle = useCallback(() => {
    set(read() === "light" ? "dark" : "light");
  }, []);
  return { scheme, toggle };
}
