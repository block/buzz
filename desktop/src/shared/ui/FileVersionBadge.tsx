import { AlertCircle, History } from "lucide-react";

import type { FileVersionStatus } from "@/shared/context/FileVersionContext";
import { cn } from "@/shared/lib/cn";

/**
 * "Outdated" / "New version" pill for a file's place in its version chain.
 *
 * Shared by all three surfaces that show it — the Files tab row
 * (`FilesPanel`), the attachment card in a chat bubble (`FileCard`), and the
 * preview modal header (`FilePreviewModal`) — so a file cannot appear
 * outdated in one place and current in another, and so the wording only has
 * to be changed once.
 *
 * When `onJumpToLatest` is supplied the Outdated pill becomes a button that
 * goes straight to the head of the chain, not one step along it: from v1 of
 * three you want the current file, not v2. Older versions stay reachable via
 * the disclosure on the newest file's card.
 *
 * Renders nothing when `status` is null (unknown — no channel context, or the
 * graph hasn't loaded) or when the file has no version links at all.
 */
export function FileVersionBadge({
  className,
  onJumpToLatest,
  status,
}: {
  className?: string;
  onJumpToLatest?: (() => void) | null;
  status: FileVersionStatus | null | undefined;
}) {
  if (status?.outdated) {
    const outdatedClass = cn(
      "flex shrink-0 items-center gap-1 rounded-full bg-amber-500/15 px-2 py-0.5 text-3xs font-medium text-amber-600 dark:text-amber-400",
      onJumpToLatest && "transition-colors hover:bg-amber-500/25",
      className,
    );

    if (onJumpToLatest) {
      return (
        <button
          className={outdatedClass}
          data-testid="file-version-badge-outdated"
          onClick={(event) => {
            // The card itself opens a preview; jumping is a different intent.
            event.stopPropagation();
            onJumpToLatest();
          }}
          title="Go to the latest version of this file"
          type="button"
        >
          <AlertCircle className="h-3 w-3" />
          Outdated — view latest
        </button>
      );
    }

    return (
      <span
        className={outdatedClass}
        data-testid="file-version-badge-outdated"
        title="A newer version of this file was shared later in this channel"
      >
        <AlertCircle className="h-3 w-3" />
        Outdated
      </span>
    );
  }

  if (status?.isNewVersion) {
    return (
      <span
        className={cn(
          "flex shrink-0 items-center gap-1 rounded-full bg-muted px-2 py-0.5 text-3xs font-medium text-muted-foreground",
          className,
        )}
        data-testid="file-version-badge-new"
        title="Tagged as a newer version of an earlier upload"
      >
        <History className="h-3 w-3" />
        New version
      </span>
    );
  }

  return null;
}
