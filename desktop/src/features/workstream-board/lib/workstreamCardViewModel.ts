import {
  parseWorkstreamCard,
  type WorkstreamCardV1,
} from "@/features/workstream-board/lib/workstreamCardParser";

export type WorkstreamCardViewModel =
  | { status: "loading" }
  | { status: "ready"; card: WorkstreamCardV1 }
  /** Canvas fetch failed, canvas is empty, or the card fence is missing/malformed. */
  | { status: "unavailable" };

/**
 * Bridges the per-channel canvas query state to a render-ready view model.
 * A card-local parse failure degrades to "unavailable" the same way a
 * failed/missing canvas fetch does — the caller renders channel metadata
 * plus an inline unavailable state either way, never a global error.
 */
export function buildWorkstreamCardViewModel(input: {
  canvasContent: string | null | undefined;
  isLoading: boolean;
  isError: boolean;
}): WorkstreamCardViewModel {
  if (input.isLoading) {
    return { status: "loading" };
  }
  if (input.isError) {
    return { status: "unavailable" };
  }

  const result = parseWorkstreamCard(input.canvasContent);
  if (!result.ok) {
    return { status: "unavailable" };
  }

  return { status: "ready", card: result.card };
}
