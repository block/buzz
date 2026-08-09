import { FileText } from "lucide-react";
import { toast } from "sonner";

import { invokeTauri } from "@/shared/api/tauri";
import { cn } from "@/shared/lib/cn";
import { fileLinkBasename, type ParsedFileLink } from "@/shared/lib/fileLink";
import {
  MENTION_CHIP_BASE_CLASSES,
  MENTION_CHIP_HOVER_CLASSES,
} from "@/shared/ui/mentionChip";

/**
 * Inline pill for a `buzz://file` deep link — click to open the live artifact
 * on disk, unlike a `FileCard`, which downloads the copy pinned at upload time.
 *
 * The open goes through the `open_workspace_file` command rather than the
 * frontend opener API so the containment check runs in Rust, where a caller
 * cannot skip it. A failure (deleted artifact, path outside its root) surfaces
 * as a toast: artifacts get regenerated, and a dead link must say so rather
 * than silently do nothing.
 */
export function FileLinkPill({
  interactive,
  link,
}: {
  interactive: boolean;
  link: ParsedFileLink;
}) {
  const label = fileLinkBasename(link);

  const content = (
    <>
      <FileText aria-hidden className="size-3.5 shrink-0" />
      {label}
    </>
  );

  // Non-interactive surfaces (the channel canvas) render every anchor inert.
  if (!interactive) {
    return (
      <span className="inline-flex items-center gap-1" data-file-link="">
        {content}
      </span>
    );
  }

  return (
    <button
      type="button"
      data-file-link=""
      aria-label={
        link.reveal ? `Show ${label} in the file manager` : `Open ${label}`
      }
      title={link.path}
      className={cn(
        "inline-flex cursor-pointer items-center gap-1",
        MENTION_CHIP_BASE_CLASSES,
        MENTION_CHIP_HOVER_CLASSES,
      )}
      onClick={() => {
        void invokeTauri("open_workspace_file", {
          path: link.path,
          root: link.root,
          reveal: link.reveal,
        }).catch((err: unknown) => {
          toast.error(
            err instanceof Error ? err.message : `Cannot open ${label}`,
          );
        });
      }}
    >
      {content}
    </button>
  );
}
