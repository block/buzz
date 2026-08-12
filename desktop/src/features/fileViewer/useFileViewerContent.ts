import { useQuery } from "@tanstack/react-query";

import { fetchMediaBytes } from "@/shared/api/tauriMedia";

import {
  type DecodedFileViewerContent,
  decodeFileViewerContent,
  type FileViewerContent,
  MAX_FILE_PREVIEW_BYTES,
} from "./fileViewerContent";

/**
 * Fetch and decode an attachment for the viewer panel.
 *
 * Bytes travel over the `fetch_media_bytes` IPC path (Rust reqwest through the
 * VPN tunnel) because a webview `fetch` to the relay is refused by Cloudflare
 * Access — see `mediaUrl.ts`. Blossom URLs are content-addressed, so a result
 * is never revalidated (`staleTime: Infinity`) and is dropped 5 minutes after
 * the last viewer closes (`gcTime`).
 */
export function useFileViewerContent(
  /** `null` disables the fetch — the panel calls this before its own guard. */
  url: string | null,
  declaredSize?: number,
): FileViewerContent {
  const oversized =
    declaredSize != null && declaredSize > MAX_FILE_PREVIEW_BYTES;
  const query = useQuery({
    enabled: url !== null && !oversized,
    gcTime: 5 * 60 * 1000,
    queryFn: async (): Promise<DecodedFileViewerContent> => {
      // Unreachable while `enabled` gates on a non-null URL; narrows the type
      // without an assertion.
      if (url === null) throw new Error("no file selected");
      return decodeFileViewerContent(await fetchMediaBytes(url));
    },
    queryKey: ["file-viewer-content", url],
    retry: 1,
    staleTime: Number.POSITIVE_INFINITY,
  });

  // Skip the fetch entirely when the imeta size already rules out a preview.
  if (oversized) return { status: "too-large" };
  if (query.isPending) return { status: "loading" };
  if (query.isError) {
    return {
      status: "error",
      message:
        query.error instanceof Error
          ? query.error.message
          : "Failed to load file",
    };
  }
  return query.data;
}
