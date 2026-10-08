import * as React from "react";

import {
  extractSupportedLinkPreviews,
  type SupportedLinkPreview,
} from "@/shared/lib/linkPreview";
import { parseLinkPreviewSnapshots } from "@/shared/lib/linkPreviewSnapshot";
import type { ResolvedLinkPreview } from "@/shared/lib/useResolvedLinkPreviews";

/**
 * Resolve the link-preview cards for a rendered message.
 *
 * External URLs render exclusively from sender-authored
 * `["link-preview","snapshot",…]` tags (`parseLinkPreviewSnapshots`) — the
 * privacy model shipped in the rich-link-previews work: recipients never
 * contact external sites. Buzz-native entity links render as inline links or
 * chips with relay-backed metadata tooltips, not as standalone cards.
 */
export function mergeMessageLinkPreviews(
  candidates: SupportedLinkPreview[],
  snapshots: ResolvedLinkPreview[],
): ResolvedLinkPreview[] {
  const snapshotsByHref = new Map(
    snapshots.map((preview) => [preview.href, preview]),
  );

  return candidates.flatMap((candidate) => {
    const preview = snapshotsByHref.get(candidate.href);
    return preview ? [preview] : [];
  });
}

export function useMessageLinkPreviews({
  content,
  interactive,
  linkPreviewTags,
  linkPreviewsSuppressed,
  relayOrigin,
}: {
  content: string;
  interactive: boolean;
  linkPreviewTags?: readonly (readonly string[])[];
  linkPreviewsSuppressed: boolean;
  relayOrigin: string | null;
}): ResolvedLinkPreview[] {
  const linkPreviewCandidates = React.useMemo(
    () =>
      interactive && !linkPreviewsSuppressed
        ? extractSupportedLinkPreviews(content, relayOrigin)
        : [],
    [content, interactive, linkPreviewsSuppressed, relayOrigin],
  );
  return React.useMemo(() => {
    const snapshots =
      interactive && !linkPreviewsSuppressed
        ? parseLinkPreviewSnapshots(linkPreviewTags, content, relayOrigin)
        : [];
    return mergeMessageLinkPreviews(linkPreviewCandidates, snapshots);
  }, [
    content,
    interactive,
    linkPreviewCandidates,
    linkPreviewTags,
    linkPreviewsSuppressed,
    relayOrigin,
  ]);
}
