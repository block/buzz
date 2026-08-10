import { nip44DecryptFromSelf } from "@/shared/api/tauri";
import type { RelayEvent } from "@/shared/api/types";
import {
  decodeContexts,
  isOverrideActive,
  isValidBlob,
  isValidReadStateDTag,
  mergeOverrideRegisters,
  type OverrideLiveness,
  type OverrideRegister,
  type ParsedContexts,
  type ReadStateBlob,
} from "@/features/channels/readState/readStateFormat";

export type ReadStateDecrypt = (ciphertext: string) => Promise<string>;

export type ParsedReadStateEvent = {
  dTag: string;
  blob: ReadStateBlob;
  /** Structured decode: frontiers keyed by raw ctx ID, overrides by raw ctx ID. */
  contexts: ParsedContexts;
  createdAt: number;
};

/**
 * The fully merged read-state across all events for a pubkey.
 * Both maps are keyed by raw context ID (unescaped).
 */
export type MergedReadState = {
  /** Componentwise max of all frontier values across events. */
  frontiers: Map<string, number>;
  /** Componentwise max of all override registers across events. */
  overrides: Map<string, OverrideRegister>;
};

export async function parseReadStateEvent(
  event: RelayEvent,
  pubkey: string,
  decrypt: ReadStateDecrypt = nip44DecryptFromSelf,
): Promise<ParsedReadStateEvent | null> {
  if (event.pubkey !== pubkey) return null;

  const dTags = event.tags.filter((tag) => tag[0] === "d");
  if (dTags.length !== 1) return null;
  const dTag = dTags[0]?.[1];
  if (!isValidReadStateDTag(dTag)) return null;

  const tTags = event.tags.filter(
    (tag) => tag[0] === "t" && tag[1] === "read-state",
  );
  if (tTags.length !== 1) return null;

  try {
    const plaintext = await decrypt(event.content);
    const parsed = JSON.parse(plaintext);
    if (!isValidBlob(parsed)) return null;
    const { wire, frontiers, overrides } = decodeContexts(parsed.contexts);
    return {
      dTag,
      blob: {
        v: 1,
        client_id: parsed.client_id,
        contexts: wire,
      },
      contexts: { frontiers, overrides },
      createdAt: event.created_at,
    };
  } catch (error) {
    console.debug(
      `[ReadStateSnapshot] decrypt/parse failed event=${event.id.substring(0, 8)}…:`,
      error,
    );
    return null;
  }
}

/**
 * Merge override registers from multiple register maps.
 *
 * Applies componentwise `max()` across all sources for each context suffix,
 * returning a single Map from raw context ID to merged `OverrideRegister`.
 * Used internally by `mergeReadStateEventsStructured`; also exported for
 * callers that collect registers independently.
 */
export function mergeOverrideRegisterMaps(
  ...maps: Array<ReadonlyMap<string, OverrideRegister>>
): Map<string, OverrideRegister> {
  const result = new Map<string, OverrideRegister>();
  for (const source of maps) {
    for (const [ctx, reg] of source) {
      const existing = result.get(ctx);
      result.set(ctx, existing ? mergeOverrideRegisters(existing, reg) : reg);
    }
  }
  return result;
}

/**
 * Merge N parsed read-state events into a single structured result.
 *
 * Returns merged `{ frontiers, overrides }` — both maps keyed by raw context
 * ID — using `mergeOverrideRegisterMaps` internally for the register join.
 * This is the primary entry point for slices 2–3; the frontier-only
 * `mergeReadStateEvents` is a thin legacy wrapper over this function.
 */
export async function mergeReadStateEventsStructured(
  events: RelayEvent[],
  pubkey: string,
  decrypt?: ReadStateDecrypt,
): Promise<MergedReadState> {
  const overrideMaps: Array<ReadonlyMap<string, OverrideRegister>> = [];
  const frontiers = new Map<string, number>();

  for (const event of events) {
    const parsed = await parseReadStateEvent(event, pubkey, decrypt);
    if (!parsed) continue;

    for (const [rawCtx, ts] of parsed.contexts.frontiers) {
      const current = frontiers.get(rawCtx) ?? 0;
      if (ts > current) frontiers.set(rawCtx, ts);
    }

    overrideMaps.push(parsed.contexts.overrides);
  }

  return { frontiers, overrides: mergeOverrideRegisterMaps(...overrideMaps) };
}

/**
 * Evaluate override liveness for one context after merging.
 *
 * Returns `OverrideLiveness` — the `active` predicate plus the effective
 * frontier used — so the caller does not have to apply `isOverrideActive`
 * manually.  Call after `mergeReadStateEventsStructured`.
 *
 * @param merged          Result of `mergeReadStateEventsStructured`.
 * @param rawCtxId        Raw (unescaped) context ID to evaluate.
 * @param effectiveFrontier
 *   The merged **effective** frontier for `rawCtxId`, after applying the
 *   hierarchical parent rule from NIP-RS `:169–196`/`:510–514`:
 *   ```
 *   effective(ctx) = max(merged[ctx], effective(parent(ctx)))
 *   ```
 *   For a channel context the effective frontier equals its own merged value.
 *   For a thread or message context the caller must fold in the parent channel
 *   frontier before passing the value here.  The caller owns this computation
 *   because only the caller knows the context hierarchy.
 */
export function computeOverrideLiveness(
  merged: MergedReadState,
  rawCtxId: string,
  effectiveFrontier: number,
): OverrideLiveness {
  const reg = merged.overrides.get(rawCtxId);
  const active = reg !== undefined && isOverrideActive(reg, effectiveFrontier);
  return { active, frontier: effectiveFrontier };
}

/**
 * Merge N read-state events into a frontier map (legacy wrapper).
 *
 * Thin wrapper over `mergeReadStateEventsStructured` that returns only the
 * merged frontiers — byte-identical behavior for `communityUnreadObserver`
 * and other existing callers that do not need override state.
 */
export async function mergeReadStateEvents(
  events: RelayEvent[],
  pubkey: string,
  decrypt?: ReadStateDecrypt,
): Promise<Map<string, number>> {
  const merged = await mergeReadStateEventsStructured(events, pubkey, decrypt);
  return merged.frontiers;
}

export function getSnapshotReadTimestamp(
  contexts: ReadonlyMap<string, number>,
  contextId: string,
): number | null {
  return contexts.get(contextId) ?? null;
}
