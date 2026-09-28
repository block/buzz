import type { BestieRhsKind } from "./bestieDmRhsHelpers";

const BESTIE_RHS_OPEN_EVENT = "buzz-bestie-open-rhs";

type BestieRhsOpenDetail = { kind: BestieRhsKind };

/**
 * Ask the Assistant DM / empty-popover RHS to drill into a category
 * (e.g. Reminders from the due nudge banner).
 */
export function requestBestieRhsOpen(kind: BestieRhsKind): void {
  if (typeof window === "undefined") return;
  window.dispatchEvent(
    new CustomEvent<BestieRhsOpenDetail>(BESTIE_RHS_OPEN_EVENT, {
      detail: { kind },
    }),
  );
}

/** Subscribe to {@link requestBestieRhsOpen} from RHS hosts. */
export function subscribeBestieRhsOpen(
  handler: (kind: BestieRhsKind) => void,
): () => void {
  if (typeof window === "undefined") return () => undefined;
  const listener = (event: Event) => {
    const detail = (event as CustomEvent<BestieRhsOpenDetail>).detail;
    if (!detail?.kind) return;
    handler(detail.kind);
  };
  window.addEventListener(BESTIE_RHS_OPEN_EVENT, listener);
  return () => window.removeEventListener(BESTIE_RHS_OPEN_EVENT, listener);
}
