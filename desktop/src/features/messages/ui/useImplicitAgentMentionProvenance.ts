import * as React from "react";

import { trimMapToSize } from "@/shared/lib/trimMapToSize";

type GeneratedMention = { pubkey: string; prefix: string };

/** Tracks the leading mentions that automatic addressing inserted per draft. */
export function useImplicitAgentMentionProvenance(
  effectiveDraftKey: string | null | undefined,
) {
  const byDraftRef = React.useRef(new Map<string, GeneratedMention[]>());

  const getPrefix = React.useCallback(() => {
    if (!effectiveDraftKey) return "";
    return (
      byDraftRef.current
        .get(effectiveDraftKey)
        ?.map((fragment) => fragment.prefix)
        .join("") ?? ""
    );
  }, [effectiveDraftKey]);

  const add = React.useCallback(
    (insertedFragments: readonly GeneratedMention[]) => {
      if (!effectiveDraftKey) return;
      const fragments = byDraftRef.current.get(effectiveDraftKey) ?? [];
      const insertedPubkeys = new Set(
        insertedFragments.map((fragment) => fragment.pubkey),
      );
      // Identity survives label resets, but generated text and prepend order do
      // not. The latest insertion owns that key's exact fragment for stripping.
      byDraftRef.current.set(effectiveDraftKey, [
        ...insertedFragments,
        ...fragments.filter(
          (fragment) => !insertedPubkeys.has(fragment.pubkey),
        ),
      ]);
      trimMapToSize(byDraftRef.current, 200);
    },
    [effectiveDraftKey],
  );

  const remove = React.useCallback(
    (pubkey: string) => {
      if (!effectiveDraftKey) return;
      const fragments = byDraftRef.current.get(effectiveDraftKey) ?? [];
      byDraftRef.current.set(
        effectiveDraftKey,
        fragments.filter((fragment) => fragment.pubkey !== pubkey),
      );
    },
    [effectiveDraftKey],
  );

  return { add, getPrefix, remove };
}
