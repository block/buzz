/**
 * Per-field last-writer-wins registers for synced sidebar preferences.
 *
 * A document is a tree whose leaves are registers `[v, dev, value]`: `v` is a
 * millisecond version, `dev` a random per-install id. Merging keeps, for every
 * leaf, the register that sorts highest, so any two devices that have seen the
 * same registers hold byte-identical documents regardless of arrival order.
 */

export type Reg<T = unknown> = [v: number, dev: string, value: T];
export type Tree = { [key: string]: Tree | Reg | undefined };

/** Device id stamped on registers imported from meta-less (legacy) data. */
export const LEGACY_DEV = "0".repeat(16);

const CLOCK_KEY = "buzz-sidebar-clock.v1";
const DEV_RE = /^[0-9a-f]{16}$/;
const encoder = new TextEncoder();

function compareBytes(left: string, right: string): number {
  const a = encoder.encode(left);
  const b = encoder.encode(right);
  for (let i = 0; i < Math.min(a.length, b.length); i++) {
    if (a[i] !== b[i]) return (a[i] as number) - (b[i] as number);
  }
  return a.length - b.length;
}

const isTombstone = (value: unknown) => value === null || value === false;

/** Orders by `v`, then `dev` bytewise, then tombstone over value, then value JSON bytewise. */
export function compareRegs(a: Reg, b: Reg): number {
  if (a[0] !== b[0]) return a[0] - b[0];
  if (a[1] !== b[1]) return compareBytes(a[1], b[1]);
  const tombA = isTombstone(a[2]);
  if (tombA !== isTombstone(b[2])) return tombA ? 1 : -1;
  return compareBytes(JSON.stringify(a[2]), JSON.stringify(b[2]));
}

export const isReg = (node: unknown): node is Reg => Array.isArray(node);

/** Own-property read: dynamic keys such as `constructor` never hit the prototype. */
export const own = (node: Tree, key: string): Tree | Reg | undefined =>
  Object.hasOwn(node, key) ? node[key] : undefined;

/** True when no register (tombstones included) exists anywhere in `node`. */
export const isEmptyTree = (node: Tree): boolean =>
  Object.values(node).every(
    (child) => child === undefined || (!isReg(child) && isEmptyTree(child)),
  );

/** A register imported from a meta-less local cache (stamp 1); any remote beats it. */
const isCachePlaceholder = (reg: Reg) => reg[0] === 1 && reg[1] === LEGACY_DEV;

/**
 * Merges `b` into `a`. With `onlyMissing`, `b` only fills leaves `a` lacks or
 * holds as a cache placeholder (legacy import: omission never means deletion,
 * and authored registers win). Returns `a` itself when nothing changes.
 */
export function mergeTrees<T extends Tree>(a: T, b: T, onlyMissing = false): T {
  let out: Tree | null = null;
  for (const [key, bv] of Object.entries(b)) {
    const av = own(a, key);
    let next = av;
    if (av === undefined) next = bv;
    else if (bv === undefined) continue;
    else if (isReg(av) && isReg(bv)) {
      if ((!onlyMissing || isCachePlaceholder(av)) && compareRegs(bv, av) > 0)
        next = bv;
    } else if (!isReg(av) && !isReg(bv)) {
      next = mergeTrees(av, bv, onlyMissing);
    }
    if (next !== av) {
      out ??= { ...a };
      out[key] = next;
    }
  }
  return (out ?? a) as T;
}

function maxVersion(node: Tree): number {
  let max = 0;
  for (const child of Object.values(node)) {
    if (child === undefined) continue;
    max = Math.max(max, isReg(child) ? child[0] : maxVersion(child));
  }
  return max;
}

function randomDev(): string {
  const bytes = crypto.getRandomValues(new Uint8Array(8));
  return Array.from(bytes, (b) => b.toString(16).padStart(2, "0")).join("");
}

/** `v = max(now, lastIssued + 1, maxSeen + 1)`; `dev` and `lastIssued` persist per install. */
function nextStamp(maxSeen: number): [number, string] {
  let dev = randomDev();
  let last = 0;
  try {
    const saved = JSON.parse(window.localStorage.getItem(CLOCK_KEY) ?? "null");
    if (DEV_RE.test(saved?.dev) && Number.isSafeInteger(saved?.last)) {
      dev = saved.dev;
      last = saved.last;
    }
  } catch {
    // Fall back to a fresh clock; ordering still holds via `maxSeen`.
  }
  const v = Math.max(Date.now(), last + 1, maxSeen + 1);
  try {
    window.localStorage.setItem(CLOCK_KEY, JSON.stringify({ dev, last: v }));
  } catch {
    // Non-fatal: the next stamp still exceeds every version in the document.
  }
  return [v, dev];
}

function getAt(tree: Tree, path: string[]): Tree | Reg | undefined {
  let node: Tree | Reg | undefined = tree;
  for (const key of path) {
    if (node === undefined || isReg(node)) return undefined;
    node = own(node, key);
  }
  return node;
}

function setAt(tree: Tree, [key, ...rest]: string[], reg: Reg): Tree {
  const child = own(tree, key as string);
  const next =
    rest.length === 0
      ? reg
      : setAt(child && !isReg(child) ? child : {}, rest, reg);
  return { ...tree, [key as string]: next };
}

/**
 * Writes each `[path, value]` as a fresh register sharing one stamp. An edit
 * where every target already holds its value mints no version and returns
 * `tree` itself. Otherwise only changed leaves are written, or with `all`
 * every leaf (a whole-list operation such as a reorder).
 */
export function setRegs<T extends Tree>(
  tree: T,
  writes: Array<[path: string[], value: unknown]>,
  all = false,
): T {
  const changed = writes.filter(([path, value]) => {
    const current = getAt(tree, path);
    return !(
      isReg(current) && JSON.stringify(current[2]) === JSON.stringify(value)
    );
  });
  if (changed.length === 0) return tree;
  const [v, dev] = nextStamp(maxVersion(tree));
  let out: Tree = tree;
  for (const [path, value] of all ? writes : changed)
    out = setAt(out, path, [v, dev, value]);
  return out as T;
}

/** A leaf validator, or a node shape: fixed fields, or `{"*": child}` for maps. */
export type Shape = ((value: unknown) => boolean) | { [key: string]: Shape };

const isObject = (value: unknown): value is Record<string, unknown> =>
  typeof value === "object" && value !== null && !Array.isArray(value);

/**
 * Keeps only well-formed nodes of `shape`. An invalid register or record is
 * dropped while its valid siblings survive.
 */
export function validTree(node: unknown, shape: Shape): Tree | Reg | undefined {
  if (typeof shape === "function") {
    return isReg(node) &&
      node.length === 3 &&
      Number.isSafeInteger(node[0]) &&
      node[0] >= 0 &&
      typeof node[1] === "string" &&
      DEV_RE.test(node[1]) &&
      shape(node[2])
      ? node
      : undefined;
  }
  if (!isObject(node)) return undefined;
  const out: Tree = {};
  for (const [key, child] of Object.entries(node)) {
    if (key === "__proto__") continue;
    const sub = Object.hasOwn(shape, key) ? shape[key] : shape["*"];
    const valid = sub === undefined ? undefined : validTree(child, sub);
    if (valid !== undefined) out[key] = valid;
  }
  return out;
}

/** Stable key-sorted JSON; the digest and no-op comparison for documents. */
export function canonical(value: unknown): string {
  if (Array.isArray(value)) return `[${value.map(canonical).join(",")}]`;
  if (isObject(value)) {
    const keys = Object.keys(value)
      .filter((key) => value[key] !== undefined)
      .sort(compareBytes);
    return `{${keys.map((key) => `${JSON.stringify(key)}:${canonical(value[key])}`).join(",")}}`;
  }
  return JSON.stringify(value);
}
