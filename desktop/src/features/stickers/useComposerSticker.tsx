import * as React from "react";

import type { StickerSelection } from "@/features/stickers/ui/ComposerStickerPicker";
import {
  stickerAssetCacheUrl,
  stickerReferenceTag,
} from "@/shared/api/stickers";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import { Button } from "@/shared/ui/button";
import { X } from "lucide-react";

export function useComposerSticker() {
  const [selection, setSelection] = React.useState<StickerSelection | null>(
    null,
  );
  const lastSelectionRef = React.useRef<StickerSelection | null>(null);
  const clear = React.useCallback(() => setSelection(null), []);
  const select = React.useCallback((next: StickerSelection) => {
    lastSelectionRef.current = next;
    setSelection(next);
  }, []);
  const setPendingTags = React.useCallback((tags: string[][]) => {
    if (tags.length === 0) {
      setSelection(null);
      return;
    }
    const saved = lastSelectionRef.current;
    if (
      saved &&
      tags[0]?.[3] === saved.sticker.sha256 &&
      tags[0]?.[1] === saved.pack.coordinate
    ) {
      setSelection(saved);
    }
  }, []);
  const tags = selection
    ? [stickerReferenceTag(selection.pack, selection.sticker)]
    : [];
  const fallback = selection ? `:${selection.sticker.shortcode}:` : "";
  return { clear, fallback, select, selection, setPendingTags, tags };
}

export const ComposerStickerPreview = React.memo(
  function ComposerStickerPreview({
    onRemove,
    selection,
  }: {
    onRemove: () => void;
    selection: StickerSelection;
  }) {
    return (
      <div
        className="mb-2 flex items-center gap-2 rounded-xl bg-muted/30 p-2"
        data-testid="pending-sticker"
      >
        <img
          alt={selection.sticker.alt ?? selection.sticker.shortcode}
          className="h-16 w-16 object-contain"
          src={rewriteRelayUrl(
            stickerAssetCacheUrl(selection.pack, selection.sticker),
          )}
        />
        <div className="min-w-0 flex-1">
          <p className="truncate text-sm font-medium">
            :{selection.sticker.shortcode}:
          </p>
          <p className="truncate text-xs text-muted-foreground">
            {selection.pack.title}
          </p>
        </div>
        <Button
          aria-label="Remove sticker"
          onClick={onRemove}
          size="icon"
          type="button"
          variant="ghost"
        >
          <X />
        </Button>
      </div>
    );
  },
);
