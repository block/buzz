import * as React from "react";

import { useCustomEmojiQuery } from "@/features/custom-emoji/hooks";
import { reactionEmojiUrl } from "@/shared/api/customEmoji";
import type { CustomEmoji } from "@/shared/lib/remarkCustomEmoji";
import {
  quickReactionStorageKey,
  readQuickReactionEntries,
  resolveQuickReactionEmojis,
} from "./useQuickReactionEmojis";

type QuickReactionItem = Readonly<{
  emoji: string;
  customEmojiUrl: string | undefined;
}>;
const EMPTY_PALETTE: readonly CustomEmoji[] = [];
const DEFAULT_ITEMS: readonly QuickReactionItem[] = resolveQuickReactionEmojis(
  [],
  3,
).map((emoji) => ({ emoji, customEmojiUrl: undefined }));
const QuickReactionContext =
  React.createContext<readonly QuickReactionItem[]>(DEFAULT_ITEMS);

/**
 * One quick-tray snapshot per community/identity session. Rows read context;
 * they do not observe the palette query, scan storage, or prepare emoji URLs.
 * Mount under AppReady's keyed community boundary so retained recents and the
 * single storage listener leave with that session, not with individual rows.
 */
export function QuickReactionProvider({
  communityScope,
  children,
}: {
  communityScope: string | null;
  children: React.ReactNode;
}) {
  const customEmoji = useCustomEmojiQuery().data ?? EMPTY_PALETTE;
  const storageKey = quickReactionStorageKey(communityScope);
  const [entries, setEntries] = React.useState(() =>
    readQuickReactionEntries(storageKey),
  );

  React.useEffect(() => {
    if (typeof window === "undefined") return;

    const handleStorage = (event: StorageEvent) => {
      if (event.key === storageKey) {
        setEntries(readQuickReactionEntries(storageKey));
      }
    };
    window.addEventListener("storage", handleStorage);
    return () => window.removeEventListener("storage", handleStorage);
  }, [storageKey]);

  // Same-window reactions only persist recents. Keep the tray steady until
  // reload or another window's storage event; palette updates still refresh
  // availability and URLs without replacing the frozen ranking inputs.
  const items = React.useMemo(
    () =>
      resolveQuickReactionEmojis(entries, 3, customEmoji)
        .map((emoji) => ({
          emoji,
          customEmojiUrl: reactionEmojiUrl(emoji, customEmoji),
        }))
        .filter(
          ({ emoji, customEmojiUrl }) =>
            !emoji.startsWith(":") || !emoji.endsWith(":") || customEmojiUrl,
        ),
    [customEmoji, entries],
  );
  const stableItems = React.useRef<readonly QuickReactionItem[]>(items);
  if (
    items.length !== stableItems.current.length ||
    items.some(
      (item, index) =>
        item.emoji !== stableItems.current[index].emoji ||
        item.customEmojiUrl !== stableItems.current[index].customEmojiUrl,
    )
  ) {
    stableItems.current = items;
  }

  return (
    <QuickReactionContext.Provider value={stableItems.current}>
      {children}
    </QuickReactionContext.Provider>
  );
}

/** Read shared, content-stable quick items without adding per-row observers. */
export function useQuickReactionItems(): readonly QuickReactionItem[] {
  return React.useContext(QuickReactionContext);
}
