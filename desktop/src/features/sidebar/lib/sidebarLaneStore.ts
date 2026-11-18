import {
  canonical,
  isReg,
  LEGACY_DEV,
  mergeTrees,
  validTree,
  type Reg,
  type Shape,
  type Tree,
} from "./sidebarLwwMap";

/** How one sidebar preference maps between its register tree and its JSON. */
export type LaneCodec = {
  storageKey: (pubkey: string, relayUrl: string) => string;
  /** Pre-relay-scoping key migrated once on first read, then removed. */
  legacyStorageKey?: (pubkey: string) => string;
  /** Shape of `meta` (excluding `meta.v`). */
  shape: Shape;
  /** Registers for a meta-less payload, every value stamped by `stamp`. */
  fromLegacy: (json: unknown, stamp: <T>(value: T) => Reg<T>) => Tree | null;
  /** The legacy fields older clients read, projected from the registers. */
  project: (tree: Tree) => Record<string, unknown>;
};

export type DecodedDoc = { tree: Tree; legacy: boolean };

/**
 * Decodes a `version: 1` payload. A meta-less payload (older writer) is
 * imported at version `legacyV`; the caller merges it only into missing
 * leaves. Returns null when unreadable, including an unknown `meta.v`.
 */
export function decodeDoc(
  codec: LaneCodec,
  json: unknown,
  legacyV: number,
): DecodedDoc | null {
  if (typeof json !== "object" || json === null) return null;
  const { version, meta } = json as { version?: unknown; meta?: unknown };
  if (version !== 1) return null;
  if (meta === undefined) {
    // Validated like `meta`, so legacy keys obey the same own-key rules.
    const tree = validTree(
      codec.fromLegacy(json, (value) => [legacyV, LEGACY_DEV, value]),
      codec.shape,
    );
    return tree && !isReg(tree) ? { tree, legacy: true } : null;
  }
  if ((meta as { v?: unknown } | null)?.v !== 1) return null;
  const { v: _v, ...fields } = meta as Record<string, unknown>;
  const tree = validTree(fields, codec.shape);
  return tree && !isReg(tree) ? { tree, legacy: false } : null;
}

export function encodeDoc(codec: LaneCodec, tree: Tree) {
  return { version: 1, ...codec.project(tree), meta: { v: 1, ...tree } };
}

/**
 * The one authoritative register tree for a lane in a pubkey + relay scope.
 * Every read and edit goes through `transact`; the full tree persists at rest
 * (limits gate only publishing).
 */
export class LaneStore {
  private tree: Tree = {};
  private dirty = false;
  /** Pre-relay-scoping key to remove once the scoped copy is durable. */
  private legacyKey: string | undefined;
  private listeners = new Set<() => void>();
  private readonly key: string;

  private readonly codec: LaneCodec;

  constructor(codec: LaneCodec, pubkey: string, relayUrl: string) {
    this.codec = codec;
    this.key = codec.storageKey(pubkey, relayUrl);
    const legacyKey = codec.legacyStorageKey?.(pubkey);
    const raw = this.readRaw(this.key) ?? this.readRaw(legacyKey);
    // Meta-less local caches import at a stable stamp any remote beats.
    const doc = raw === undefined ? null : decodeDoc(codec, raw, 1);
    if (!doc) return;
    this.tree = doc.tree;
    this.dirty = doc.legacy;
    this.legacyKey = legacyKey;
    this.persist();
  }

  private readRaw(key: string | undefined): unknown {
    try {
      const raw = key ? window.localStorage.getItem(key) : null;
      return raw === null ? undefined : JSON.parse(raw);
    } catch {
      return undefined;
    }
  }

  get = (): Tree => this.tree;

  subscribe = (listener: () => void): (() => void) => {
    this.listeners.add(listener);
    return () => this.listeners.delete(listener);
  };

  /**
   * Applies `fn`. A tree with unchanged canonical bytes has no side effects
   * beyond retrying an earlier failed write.
   */
  transact(fn: (tree: Tree) => Tree): void {
    const next = fn(this.tree);
    if (next === this.tree || canonical(next) === canonical(this.tree)) {
      this.persist();
      return;
    }
    this.tree = next;
    this.dirty = true;
    this.persist();
    for (const listener of this.listeners) listener();
  }

  merge(remote: Tree, onlyMissing = false): void {
    this.transact((tree) => mergeTrees(tree, remote, onlyMissing));
  }

  /**
   * Writes the tree if an earlier write failed or is outstanding, then retires
   * the legacy key once the scoped copy is durable. Both retry on later calls.
   */
  persist(): boolean {
    try {
      if (this.dirty) {
        const raw = canonical(encodeDoc(this.codec, this.tree));
        window.localStorage.setItem(this.key, raw);
        this.dirty = false;
      }
      if (this.legacyKey && this.readRaw(this.key) !== undefined) {
        window.localStorage.removeItem(this.legacyKey);
        this.legacyKey = undefined;
      }
    } catch {
      // Stays pending; retried on the next transaction or recovery tick.
    }
    return !this.dirty;
  }

  /** Merges (never replaces) another tab's copy of this scope. */
  attachCrossTab(): () => void {
    const handler = (event: StorageEvent) => {
      if (event.key !== this.key || event.newValue === null) return;
      try {
        const doc = decodeDoc(this.codec, JSON.parse(event.newValue), 1);
        if (doc) this.merge(doc.tree, doc.legacy);
      } catch {
        // Ignore a malformed foreign write.
      }
    };
    window.addEventListener("storage", handler);
    return () => window.removeEventListener("storage", handler);
  }
}
