import * as React from "react";

import { relayClient } from "@/shared/api/relayClient";
import { PUBLISH_CANCELED } from "@/shared/api/relayEventPublisher";
import {
  nip44DecryptFromSelf,
  nip44EncryptToSelf,
  signRelayEvent,
} from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import { KIND_CHANNEL_SECTIONS } from "@/shared/constants/kinds";
import {
  decodeDoc,
  encodeDoc,
  LaneStore,
  type LaneCodec,
} from "./sidebarLaneStore";
import { canonical, isEmptyTree, type Tree } from "./sidebarLwwMap";
import { advanceWatermark, readWatermark } from "./sidebarSyncWatermark";
import { useStaleReaderRecovery } from "./useStaleReaderRecovery";

/** Relay event size ceiling shared by every sidebar blob. */
const MAX_CIPHERTEXT_BYTES = 65_535;
const DEBOUNCE_MS = 2_000;
const BACKOFF_MS = [5_000, 10_000, 30_000, 60_000, 60_000];

export type Lane = LaneCodec & {
  dTag: string;
  /** False when the projected payload exceeds today's count caps. */
  withinLimits: (projection: Record<string, unknown>) => boolean;
};

type Status = "unknown" | "empty" | "decoding" | "decoded" | "unreadable";
type Head = {
  id: string;
  createdAt: number;
  status: Status;
  digest: string | null;
};

/** `a` beats `b`: later `created_at`, then lower id (the relay's replaceable winner). */
const beats = (a: RelayEvent, b: Head) =>
  a.created_at > b.createdAt || (a.created_at === b.createdAt && a.id < b.id);

/**
 * Converges one lane's local tree with its relay head. Publishes only when the
 * decoded head's canonical bytes differ from the local tree's, never over an
 * undecoded or unreadable head, and never over an absence after a head has
 * been seen in this scope (the watermark). Every publish attempt waits for
 * `notBefore` (edit debounce or failure backoff); reads and merges do not.
 */
export class LaneReconciler {
  head: Head = { id: "", createdAt: 0, status: "unknown", digest: null };
  private lastHead: number;
  private attempt: object | null = null;
  private recheck = false;
  private failures = 0;
  private notBefore = 0;
  private observations = 0;
  private timer: ReturnType<typeof setTimeout> | null = null;
  private destroyed = false;

  private readonly lane: Lane;
  private readonly store: LaneStore;
  private readonly pubkey: string;
  private readonly relayUrl: string;

  constructor(lane: Lane, store: LaneStore, pubkey: string, relayUrl: string) {
    this.lane = lane;
    this.store = store;
    this.pubkey = pubkey;
    this.relayUrl = relayUrl;
    this.lastHead = readWatermark(pubkey, lane.dTag, relayUrl);
  }

  filter(limit: number) {
    return {
      kinds: [KIND_CHANNEL_SECTIONS],
      authors: [this.pubkey],
      "#d": [this.lane.dTag],
      limit,
    };
  }

  /** Records a raw head; one that beats the current head holds publishing until decoded. */
  private observe(event: RelayEvent): void {
    this.observations++;
    this.lastHead = Math.max(this.lastHead, event.created_at);
    advanceWatermark(
      this.pubkey,
      this.lane.dTag,
      this.relayUrl,
      event.created_at,
    );
    if (!beats(event, this.head)) return;
    this.head = {
      id: event.id,
      createdAt: event.created_at,
      status: "decoding",
      digest: null,
    };
  }

  /** Decodes and merges `event`; its status lands only while it is still the head. */
  async ingest(event: RelayEvent): Promise<void> {
    if (event.pubkey !== this.pubkey) return;
    this.observe(event);
    const mine = this.observations;
    let json: unknown = null;
    try {
      json = JSON.parse(await nip44DecryptFromSelf(event.content));
    } catch {
      json = null;
    }
    if (this.destroyed) return;
    const doc = decodeDoc(this.lane, json, event.created_at * 1_000);
    if (doc) this.store.merge(doc.tree, doc.legacy);
    // Only the newest observation (event or absence) may set eligibility; a
    // stale decode still merges its content above.
    if (this.head.id === event.id && this.observations === mine) {
      this.head = doc
        ? { ...this.head, status: "decoded", digest: canonical(json) }
        : { ...this.head, status: "unreadable", digest: null };
    }
    this.reconcile();
  }

  /** One relay read. Throws on transport failure. */
  async read(): Promise<void> {
    const seen = this.observations;
    const [event] = await relayClient.fetchEvents(this.filter(1));
    if (this.destroyed) return;
    if (event && event.pubkey === this.pubkey) return this.ingest(event);
    // Absence counts only if nothing was observed while it was in flight. It
    // permits a first copy for a genuinely new scope; after a head has been
    // seen it demotes the head and holds until a readable head returns.
    if (seen === this.observations) {
      this.observations++;
      const status = this.lastHead === 0 ? "empty" : "unknown";
      this.head = { ...this.head, status, digest: null };
    }
    this.reconcile();
  }

  reconcile(): void {
    if (this.destroyed) return;
    const wait = this.notBefore - Date.now();
    if (wait > 0) {
      this.wake(wait); // hold; re-read at the deadline
      return;
    }
    if (this.attempt) {
      this.recheck = true;
      return;
    }
    const { status, digest } = this.head;
    if (status !== "empty" && status !== "decoded") return; // hold; retried on cadence
    const tree = this.store.get();
    const settled =
      status === "empty"
        ? isEmptyTree(tree)
        : digest === canonical(encodeDoc(this.lane, tree));
    if (settled) {
      this.failures = 0;
      return;
    }
    void this.runAttempt();
  }

  /** Re-reads after `delay`. Never shortens a publish hold (`defer`). */
  wake(delay = 0): void {
    if (this.destroyed) return;
    if (this.timer !== null) clearTimeout(this.timer);
    this.timer = setTimeout(() => {
      this.timer = null;
      this.read().catch(() => this.backoff());
    }, delay);
  }

  /** Holds publishing for at least `delay`; local edits coalesce behind it. */
  defer(delay = DEBOUNCE_MS): void {
    this.notBefore = Math.max(this.notBefore, Date.now() + delay);
    this.wake(delay);
  }

  private backoff(): void {
    const delay = BACKOFF_MS[
      Math.min(this.failures, BACKOFF_MS.length - 1)
    ] as number;
    this.failures++;
    this.defer(delay);
  }

  private async runAttempt(): Promise<void> {
    const token = {};
    this.attempt = token;
    let outcome: "settled" | "acked" | "held" | "failed" = "failed";
    const held = () => Date.now() < this.notBefore;
    let head = this.head;
    let tree: Tree = this.store.get();
    try {
      await this.read(); // preflight: merge the current head first
      head = this.head;
      tree = this.store.get();
      if (held()) {
        outcome = "held"; // a deadline arrived during preflight
        return;
      }
      const doc = encodeDoc(this.lane, tree);
      const empty = head.status === "empty" && isEmptyTree(tree);
      if (head.status !== "empty" && head.status !== "decoded") {
        outcome = "settled"; // hold: the recovery cadence re-reads
        return;
      }
      if (
        empty ||
        head.digest === canonical(doc) ||
        !this.lane.withinLimits(doc)
      ) {
        outcome = "settled"; // equal, or over limit: kept locally only
        return;
      }
      const dropped = () =>
        this.destroyed ||
        held() ||
        this.store.get() !== tree ||
        this.head.id !== head.id ||
        this.head.status !== head.status;
      // A deliberate deadline cancel keeps the deadline; stale drops back off.
      const drop = () => {
        if (!dropped()) return false;
        if (held()) outcome = "held";
        return true;
      };
      const content = await nip44EncryptToSelf(JSON.stringify(doc));
      if (new TextEncoder().encode(content).length > MAX_CIPHERTEXT_BYTES) {
        outcome = "settled";
        return;
      }
      if (drop()) return;
      const event = await signRelayEvent({
        kind: KIND_CHANNEL_SECTIONS,
        content,
        createdAt: Math.max(Math.floor(Date.now() / 1_000), head.createdAt + 1),
        tags: [
          ["d", this.lane.dTag],
          ["t", this.lane.dTag], // relay discoverability; not used in our filters
        ],
      });
      if (drop()) return;
      try {
        await relayClient.publishEvent(
          event,
          `Timed out publishing ${this.lane.dTag}.`,
          `Failed to publish ${this.lane.dTag}.`,
          () => !dropped(), // checked again right before each socket send
        );
      } catch (error) {
        const message = String((error as Error)?.message);
        if (message === PUBLISH_CANCELED && held()) {
          outcome = "held";
          return;
        }
        if (!message.startsWith("duplicate:")) throw error;
      }
      outcome = "acked";
      this.lastHead = Math.max(this.lastHead, event.created_at);
      advanceWatermark(
        this.pubkey,
        this.lane.dTag,
        this.relayUrl,
        event.created_at,
      );
    } catch (error) {
      console.warn(`[${this.lane.dTag}] sync attempt failed:`, error);
    } finally {
      if (this.attempt === token) this.attempt = null;
      const recheck = this.recheck;
      this.recheck = false;
      // An ACK proves nothing about the head; verify with a read. Real
      // failures and stale drops retry behind the backoff, never immediately.
      if (outcome === "acked") this.wake();
      else if (outcome === "held")
        this.wake(this.notBefore - Date.now()); // deadline kept; no retry now
      else if (outcome === "failed") this.backoff();
      else if (recheck && (this.head !== head || this.store.get() !== tree)) {
        this.reconcile();
      }
    }
  }

  destroy(): void {
    this.destroyed = true;
    if (this.timer !== null) clearTimeout(this.timer);
  }
}

/**
 * Binds a lane to React for one pubkey + relay scope: the store's tree as
 * state, and a reconciler fed by bootstrap, live events, reconnects, local
 * edits, and the stale-reader recovery cadence.
 */
export function useLaneSync(
  lane: Lane,
  pubkey: string | undefined,
  relayUrl: string | undefined,
): { tree: Tree; edit: (fn: (tree: Tree) => Tree) => void } {
  const store = React.useMemo(
    () => (pubkey && relayUrl ? new LaneStore(lane, pubkey, relayUrl) : null),
    [lane, pubkey, relayUrl],
  );
  const [reconciler, setReconciler] = React.useState<LaneReconciler | null>(
    null,
  );
  const reconcilerRef = React.useRef<LaneReconciler | null>(null);
  reconcilerRef.current = reconciler;
  const tree = React.useSyncExternalStore(
    React.useCallback(
      (fn: () => void) => store?.subscribe(fn) ?? (() => {}),
      [store],
    ),
    () => store?.get() ?? EMPTY,
  );

  React.useEffect(() => {
    if (!store || !pubkey || !relayUrl) return;
    const rec = new LaneReconciler(lane, store, pubkey, relayUrl);
    setReconciler(rec);
    let disposed = false;
    let unsubLive: (() => Promise<void>) | null = null;
    const detachTabs = store.attachCrossTab();
    const unsubReconnect = relayClient.subscribeToReconnects(() => rec.wake());
    void relayClient
      .subscribeLive(rec.filter(0), (event) => {
        if (!disposed) void rec.ingest(event);
      })
      .then((dispose) => {
        if (disposed) void dispose();
        else unsubLive = dispose;
      })
      .catch(() => {});
    return () => {
      disposed = true;
      rec.destroy();
      detachTabs();
      unsubReconnect();
      if (unsubLive) void unsubLive();
      setReconciler(null);
    };
  }, [lane, store, pubkey, relayUrl]);

  // D2: the shared recovery cadence drives reads even while an edit is
  // pending; the reconciler consumes every result, so nothing is applied here.
  const recoveryRead = React.useCallback(async () => {
    store?.persist();
    await reconciler?.read().catch(() => {});
    return undefined;
  }, [store, reconciler]);
  useStaleReaderRecovery<never, never>({
    enabled: reconciler !== null,
    fetch: recoveryRead,
    hasPending: NEVER_PENDING,
    getRevision: ZERO,
    makeUpdater: IDENTITY,
    setStore: NOOP,
  });

  // A local edit persists immediately and publishes after the debounce; an
  // edit made before the reconciler exists is picked up by its first read.
  const edit = React.useCallback(
    (fn: (tree: Tree) => Tree) => {
      if (!store) return;
      const before = store.get();
      store.transact(fn);
      if (store.get() !== before) reconcilerRef.current?.defer();
    },
    [store],
  );

  return { tree, edit };
}

const EMPTY: Tree = Object.freeze({}) as Tree;
const NEVER_PENDING = () => false;
const ZERO = () => 0;
const IDENTITY = () => (prev: never) => prev;
const NOOP = () => {};
