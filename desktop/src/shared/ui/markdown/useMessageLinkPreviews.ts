import * as React from "react";

import {
  extractSupportedLinkPreviews,
  type SupportedLinkPreview,
} from "@/shared/lib/linkPreview";
import { parseLinkPreviewSnapshots } from "@/shared/lib/linkPreviewSnapshot";
import {
  isBuzzEntityPreview,
  type ResolvedLinkPreview,
  useResolvedLinkPreviews,
  withEntityFallbacks,
} from "@/shared/lib/useResolvedLinkPreviews";

/**
 * Resolve the link-preview cards for a rendered message.
 *
 * External URLs render exclusively from sender-authored
 * `["link-preview","snapshot",…]` tags (`parseLinkPreviewSnapshots`) — the
 * privacy model shipped in the rich-link-previews work: recipients never
 * contact external sites.
 *
 * `buzz://pr|issue|repo` entity links (and relay clone URLs, which normalize
 * onto `buzz://repo`) are the exception and are rendered recipient-side:
 * their metadata comes from the community relay itself via the entity metadata
 * loader, so the sender-snapshot privacy model does not apply — and senders
 * (CLI, agents, mobile) attach no snapshot tags for them. Recognized entity
 * cards win
 * over conflicting sender snapshots, and cards retain their first-seen order in
 * the message.
 */
export function mergeMessageLinkPreviews(
  candidates: SupportedLinkPreview[],
  snapshots: ResolvedLinkPreview[],
  entities: ResolvedLinkPreview[],
): ResolvedLinkPreview[] {
  const snapshotsByHref = new Map(
    snapshots.map((preview) => [preview.href, preview]),
  );
  const entitiesByHref = new Map(
    entities.map((preview) => [preview.href, preview]),
  );

  return candidates.flatMap((candidate) => {
    const preview = isBuzzEntityPreview(candidate)
      ? entitiesByHref.get(candidate.href)
      : snapshotsByHref.get(candidate.href);
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
  const entityLinkPreviews = React.useMemo(
    () => linkPreviewCandidates.filter(isBuzzEntityPreview),
    [linkPreviewCandidates],
  );
  const relayResolvedEntityLinkPreviews =
    useResolvedLinkPreviews(entityLinkPreviews);
  const resolvedEntityLinkPreviews = React.useMemo(
    () =>
      withEntityFallbacks(entityLinkPreviews, relayResolvedEntityLinkPreviews),
    [entityLinkPreviews, relayResolvedEntityLinkPreviews],
  );
  return React.useMemo(() => {
    const snapshots =
      interactive && !linkPreviewsSuppressed
        ? parseLinkPreviewSnapshots(linkPreviewTags, content, relayOrigin)
        : [];
    return mergeMessageLinkPreviews(
      linkPreviewCandidates,
      snapshots,
      resolvedEntityLinkPreviews,
    );
  }, [
    content,
    interactive,
    linkPreviewCandidates,
    linkPreviewTags,
    linkPreviewsSuppressed,
    relayOrigin,
    resolvedEntityLinkPreviews,
  ]);
}
