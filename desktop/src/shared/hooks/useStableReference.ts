import * as React from "react";

/**
 * Returns `next` but preserves the previous reference whenever the two Maps
 * have identical entries (same size, same key→value pairs). Lets a value that
 * is recomputed into a fresh Map on an unrelated invalidation (e.g. a version
 * bump) keep a stable identity so downstream `React.memo` boundaries can bail.
 */
export function useStableMap<K, V>(next: Map<K, V>): Map<K, V> {
  const ref = React.useRef(next);
  const prev = ref.current;
  if (prev !== next && mapsEqual(prev, next)) {
    return prev;
  }
  ref.current = next;
  return next;
}

function mapsEqual<K, V>(a: Map<K, V>, b: Map<K, V>): boolean {
  if (a.size !== b.size) return false;
  for (const [key, value] of a) {
    if (!b.has(key) || !Object.is(b.get(key), value)) return false;
  }
  return true;
}

/**
 * Returns `next` but preserves the previous reference when the two arrays are
 * shallow-equal (same length, `Object.is` on each element). Same purpose as
 * `useStableMap` for array-valued derived state. Preserves the caller's exact
 * array type (mutable or readonly).
 */
export function useStableArrayShallow<T extends readonly unknown[]>(
  next: T,
): T {
  const ref = React.useRef(next);
  const prev = ref.current;
  if (prev !== next && arraysShallowEqual(prev, next)) {
    return prev;
  }
  ref.current = next;
  return next;
}

function arraysShallowEqual(
  a: readonly unknown[],
  b: readonly unknown[],
): boolean {
  if (a.length !== b.length) return false;
  for (let i = 0; i < a.length; i += 1) {
    if (!Object.is(a[i], b[i])) return false;
  }
  return true;
}

/**
 * Returns `next` but preserves the previous reference when the two Sets have
 * identical membership. Same purpose as `useStableMap` for Set-valued derived
 * state (e.g. pubkey sets rebuilt whenever a polling query re-materialises
 * its data without changing which pubkeys are in it).
 */
export function useStableSet<T>(next: ReadonlySet<T>): ReadonlySet<T> {
  const ref = React.useRef(next);
  const prev = ref.current;
  if (prev !== next && setsEqual(prev, next)) {
    return prev;
  }
  ref.current = next;
  return next;
}

function setsEqual<T>(a: ReadonlySet<T>, b: ReadonlySet<T>): boolean {
  if (a.size !== b.size) return false;
  for (const value of a) {
    if (!b.has(value)) return false;
  }
  return true;
}
