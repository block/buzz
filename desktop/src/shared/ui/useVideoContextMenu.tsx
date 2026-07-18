import * as React from "react";

import { invokeTauri } from "@/shared/api/tauri";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import {
  MediaContextMenu,
  type MediaContextMenuItem,
  type MediaContextMenuPosition,
  useDismissMediaContextMenu,
} from "@/shared/ui/markdown/MediaContextMenu";
import { resolveVideoDownloadFilename } from "@/shared/ui/videoDownload";
import { toast } from "sonner";

type UseVideoContextMenu = {
  /** `onContextMenuCapture` handler for the inline video surface. */
  onContextMenu: (event: React.MouseEvent) => void;
  /** The positioned menu element while open, or `null`. */
  menu: React.ReactNode;
};

/**
 * Owns the inline video right-click menu: open/close state, the pointer-anchor
 * handler, and the Download/Copy actions. Kept out of `VideoPlayer` so that
 * large component stays focused on playback, and out of the pure
 * `videoDownload.ts` helpers so they keep their DOM-free, Node-testable shape.
 *
 * `downloadUrl` is the original relay `/media/` URL (distinct from a rewritten
 * proxy `src`); when absent the menu omits Download and offers only Copy link,
 * so a non-relay video — which the download command's SSRF gate would reject —
 * never surfaces an action that could only error.
 */
export function useVideoContextMenu(
  src: string,
  downloadUrl?: string,
  filename?: string,
): UseVideoContextMenu {
  const [position, setPosition] =
    React.useState<MediaContextMenuPosition | null>(null);
  const close = React.useCallback(() => setPosition(null), []);
  useDismissMediaContextMenu(Boolean(position), close);

  const onContextMenu = React.useCallback((event: React.MouseEvent) => {
    event.preventDefault();
    setPosition({ x: event.clientX, y: event.clientY });
  }, []);

  const items = React.useMemo<MediaContextMenuItem[]>(() => {
    const entries: MediaContextMenuItem[] = [];
    if (downloadUrl) {
      entries.push({
        label: "Download video",
        onSelect: () => {
          close();
          invokeTauri("download_file", {
            url: downloadUrl,
            filename: resolveVideoDownloadFilename(filename),
          }).catch((err: unknown) => {
            toast.error(err instanceof Error ? err.message : "Download failed");
          });
        },
      });
    }
    entries.push({
      label: "Copy link",
      onSelect: () => {
        close();
        copyTextToClipboard(downloadUrl ?? src, "Link copied to clipboard");
      },
    });
    return entries;
  }, [close, downloadUrl, filename, src]);

  return {
    onContextMenu,
    menu: position ? (
      <MediaContextMenu
        dataAttributes={["data-video-context-menu"]}
        items={items}
        position={position}
      />
    ) : null,
  };
}
