import * as React from "react";

import { stickerCacheUrl, type StickerReference } from "@/shared/api/stickers";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";

export const StickerMessage = React.memo(function StickerMessage({
  fallback,
  reference,
}: {
  fallback: string;
  reference: StickerReference;
}) {
  const [failed, setFailed] = React.useState(false);
  if (failed) {
    return (
      <div className="inline-flex min-h-24 min-w-24 flex-col items-center justify-center rounded-2xl border border-dashed border-border/70 bg-muted/20 p-3">
        <span aria-hidden className="text-2xl">
          🧩
        </span>
        <span className="mt-1 text-xs text-muted-foreground">
          Sticker unavailable
        </span>
        <span className="mt-1 text-sm">{fallback}</span>
      </div>
    );
  }
  return (
    <img
      alt={fallback || `:${reference.shortcode}:`}
      className="max-h-48 max-w-48 object-contain"
      data-testid="sonar-sticker"
      loading="lazy"
      onError={() => setFailed(true)}
      src={rewriteRelayUrl(stickerCacheUrl(reference))}
    />
  );
});
