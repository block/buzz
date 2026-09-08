import { useCallback, useState } from "react";

export type ComposerDrafts = {
  readDraft: (destinationId: string) => string;
  writeDraft: (destinationId: string, value: string) => void;
  clearDraft: (destinationId: string) => void;
};

/**
 * Owns drafts by product destination rather than by a mounted text field.
 *
 * The AppShell creates this once for the current application composition, so a
 * conversation can unmount while its draft remains ready when the person
 * returns. Persisting across an application restart is a later product
 * decision; this controller deliberately does not invent it.
 */
export function useComposerDrafts(): ComposerDrafts {
  const [drafts, setDrafts] = useState<Map<string, string>>(() => new Map());

  const readDraft = useCallback(
    (destinationId: string) => drafts.get(destinationId) ?? "",
    [drafts],
  );

  const writeDraft = useCallback((destinationId: string, value: string) => {
    setDrafts((current) => {
      const next = new Map(current);
      if (value) next.set(destinationId, value);
      else next.delete(destinationId);
      return next;
    });
  }, []);

  const clearDraft = useCallback((destinationId: string) => {
    setDrafts((current) => {
      if (!current.has(destinationId)) return current;
      const next = new Map(current);
      next.delete(destinationId);
      return next;
    });
  }, []);

  return { readDraft, writeDraft, clearDraft };
}
