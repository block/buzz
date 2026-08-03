import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  entityLinkProjectRouteId,
  isEntityLink,
  parseEntityLink,
  type ParsedEntityLink,
} from "@/shared/lib/entityLink";
import type { SupportedLinkPreview } from "@/shared/lib/linkPreview";

/**
 * Navigate to the project detail view for a `buzz://pr|issue|repo` link.
 * The link's (owner, d) coordinate is exactly the `/projects/$projectId`
 * route id, so no read-model resolution is needed.
 */
export function useOpenEntityLink(): (link: ParsedEntityLink) => void {
  const { goProject } = useAppNavigation();
  return React.useCallback(
    (link: ParsedEntityLink) => {
      void goProject(entityLinkProjectRouteId(link), {
        ...(link.type === "pr" ? { pullRequestId: link.id } : {}),
        ...(link.type === "issue" ? { issueId: link.id } : {}),
      });
    },
    [goProject],
  );
}

/**
 * In-app open handlers for `buzz://` entity preview cards, keyed by href.
 * External cards get no handler and keep their OS-opened anchor.
 */
export function useEntityCardOpenHandlers(
  previews: SupportedLinkPreview[],
  onOpenEntityLink: (link: ParsedEntityLink) => void,
): Map<string, () => void> {
  return React.useMemo(() => {
    const handlers = new Map<string, () => void>();
    for (const preview of previews) {
      if (!isEntityLink(preview.href)) continue;
      const parsed = parseEntityLink(preview.href);
      if (parsed.ok) {
        handlers.set(preview.href, () => onOpenEntityLink(parsed.value));
      }
    }
    return handlers;
  }, [onOpenEntityLink, previews]);
}

/**
 * Render an inline anchor for a `buzz://pr|issue|repo` entity link that
 * navigates in-app instead of handing the custom scheme to the OS (which
 * has no handler for it yet). Returns null when the href is not a valid
 * entity link so the caller can fall through to its default anchor.
 */
export function renderEntityLinkAnchor({
  anchorProps,
  children,
  href,
  onOpenEntityLink,
}: {
  anchorProps: React.ComponentPropsWithoutRef<"a">;
  children: React.ReactNode;
  href: string | undefined;
  onOpenEntityLink: (link: ParsedEntityLink) => void;
}): React.ReactElement | null {
  if (!href || !isEntityLink(href)) return null;

  const parsed = parseEntityLink(href);
  if (!parsed.ok) return null;

  return (
    <a
      {...anchorProps}
      className="font-medium text-primary underline underline-offset-4 transition-colors hover:text-primary/80 cursor-pointer"
      href={href}
      onClick={(event) => {
        event.preventDefault();
        onOpenEntityLink(parsed.value);
      }}
    >
      {children}
    </a>
  );
}
