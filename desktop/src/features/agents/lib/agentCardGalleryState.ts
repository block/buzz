/**
 * View-state selection for the minted-card gallery.
 *
 * Kept as a pure function so the error boundary is testable without a DOM:
 * a rejected `list_agent_cards` query MUST surface as an error state, never
 * as an empty archive — "No cards yet" on a permissions/IPC failure tells
 * someone their paid, persisted cards do not exist.
 */

export type AgentCardGalleryViewState =
  | { kind: "loading" }
  | { kind: "error"; message: string }
  | { kind: "empty" }
  | { kind: "cards" };

export function agentCardGalleryViewState(query: {
  isLoading: boolean;
  isError: boolean;
  error: unknown;
  data: readonly unknown[] | undefined;
}): AgentCardGalleryViewState {
  // Error wins over everything: `data` falls back to `[]` at the call site,
  // so checking emptiness first would render a failure as a false empty.
  if (query.isError) {
    const message =
      query.error instanceof Error
        ? query.error.message
        : String(query.error ?? "unknown error");
    return { kind: "error", message };
  }
  if (query.isLoading || query.data === undefined) {
    return { kind: "loading" };
  }
  return query.data.length === 0 ? { kind: "empty" } : { kind: "cards" };
}
