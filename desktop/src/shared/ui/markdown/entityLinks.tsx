import * as React from "react";

import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  entityLinkProjectRouteId,
  isEntityLink,
  parseEntityLink,
  type ParsedEntityLink,
} from "@/shared/lib/entityLink";
import {
  parseSupportedLinkPreview,
  type SupportedLinkPreview,
} from "@/shared/lib/linkPreview";

import {
  loadBuzzEntityMetadata,
  type LinkPreviewMetadata,
} from "@/shared/lib/useResolvedLinkPreviews";
import {
  Tooltip,
  TooltipContent,
  TooltipProvider,
  TooltipTrigger,
} from "@/shared/ui/tooltip";

import { BuzzInlineLink, BuzzLinkChip } from "./BuzzLinkChip";

function EntityMetadataTooltip({
  children,
  fallback,
  footer,
  href,
}: {
  children: (
    metadata: LinkPreviewMetadata | null | undefined,
  ) => React.ReactElement;
  fallback: string;
  footer: string;
  href: string;
}) {
  const [metadata, setMetadata] = React.useState<
    LinkPreviewMetadata | null | undefined
  >(undefined);
  React.useEffect(() => {
    let cancelled = false;
    void loadBuzzEntityMetadata(href).then((value) => {
      if (!cancelled) setMetadata(value);
    });
    return () => {
      cancelled = true;
    };
  }, [href]);
  const context = metadata?.title.trim() || fallback;
  const chip = children(metadata);
  return (
    <TooltipProvider delayDuration={500} skipDelayDuration={0}>
      <Tooltip>
        <TooltipTrigger asChild>{chip}</TooltipTrigger>
        <TooltipContent
          align="start"
          className="max-w-72 p-2 text-left"
          side="top"
        >
          <span className="line-clamp-2" data-buzz-tooltip-metadata-content="">
            {context}
            {metadata?.description ? ` · ${metadata.description}` : null}
          </span>
          <span
            className="mt-1 block text-2xs text-secondary-foreground/70"
            data-buzz-tooltip-metadata-type=""
          >
            {footer}
          </span>
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

function entityLinkPresentation(link: ParsedEntityLink) {
  switch (link.type) {
    case "repo":
      return {
        ariaLabel: link.commitHash
          ? `Open commit ${link.commitHash.slice(0, 8)} in repository ${link.dtag}`
          : `Open repository ${link.dtag}`,
        icon: "repo" as const,
        label: link.commitHash
          ? `${link.dtag} · ${link.commitHash.slice(0, 8)}`
          : link.dtag,
        tooltipFooter: "Repository",
      };
    case "pr":
      return {
        ariaLabel: `Open pull request ${link.id.slice(0, 8)} in repository ${link.dtag}`,
        icon: "pr" as const,
        label: `${link.dtag} · ${link.id.slice(0, 8)}`,
        tooltipFooter: `Pull request · ${link.dtag}`,
      };
    case "issue":
      return {
        ariaLabel: `Open issue ${link.id.slice(0, 8)} in repository ${link.dtag}`,
        icon: "issue" as const,
        label: `${link.dtag} · ${link.id.slice(0, 8)}`,
        tooltipFooter: `Issue · ${link.dtag}`,
      };
    case "project":
      return {
        ariaLabel: `Open project ${link.dtag}`,
        icon: "project" as const,
        label: link.dtag,
        tooltipFooter: "Project",
      };
  }
}

/**
 * Navigate to the project detail view for a `buzz://pr|issue|repo` link.
 * The link's (owner, d) coordinate is exactly the `/projects/$projectId`
 * route id, so no read-model resolution is needed.
 */
export function useOpenEntityLink(): (link: ParsedEntityLink) => void {
  const { goProject } = useAppNavigation();
  return React.useCallback(
    (link: ParsedEntityLink) => {
      const tab =
        (link.type === "repo" || link.type === "project") && link.tab
          ? link.tab
          : undefined;
      void goProject(entityLinkProjectRouteId(link), {
        entityNavigationId: crypto.randomUUID(),
        ...(tab
          ? {
              tab,
            }
          : {}),
        ...(link.type === "pr" ? { pullRequestId: link.id } : {}),
        ...(link.type === "issue" ? { issueId: link.id } : {}),
        ...(link.type === "repo" && link.commitHash
          ? { commitHash: link.commitHash }
          : {}),
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
 * Resolve an anchor href to a canonical `buzz://` entity link, accepting
 * both the deep-link scheme directly and HTTPS relay clone URLs (which the
 * preview parser normalizes onto `buzz://repo` only when the URL origin
 * matches the active relay origin).
 */
function resolveEntityHref(
  href: string,
  relayOrigin: string | null,
): string | null {
  if (isEntityLink(href)) return href;
  if (!/^https?:\/\//i.test(href)) return null;

  const preview = parseSupportedLinkPreview(href, relayOrigin);
  return preview && isEntityLink(preview.href) ? preview.href : null;
}

/**
 * Render an inline anchor for a Buzz entity link (`buzz://pr|issue|repo` or
 * an HTTPS relay clone URL whose origin matches the active relay) that
 * navigates in-app instead of handing the URL to the OS. Returns null when
 * the href is not a valid entity link so the caller can fall through to its
 * default anchor.
 */
export function renderEntityLinkAnchor({
  children,
  href,
  onOpenEntityLink,
  relayOrigin,
  interactive = true,
  asChip = true,
}: {
  children: React.ReactNode;
  href: string | undefined;
  onOpenEntityLink: (link: ParsedEntityLink) => void;
  relayOrigin: string | null;
  interactive?: boolean;
  asChip?: boolean;
}): React.ReactElement | null {
  if (!href) return null;

  const canonicalHref = resolveEntityHref(href, relayOrigin);
  if (!canonicalHref) return null;

  const parsed = parseEntityLink(canonicalHref);
  if (!parsed.ok) return null;
  const presentation = entityLinkPresentation(parsed.value);

  if (!asChip) {
    return (
      <BuzzInlineLink
        href={href}
        title={href}
        aria-label={presentation.ariaLabel}
        interactive={interactive}
        onOpenLink={() => onOpenEntityLink(parsed.value)}
      >
        {children}
      </BuzzInlineLink>
    );
  }

  const chip = (metadata?: LinkPreviewMetadata | null) => {
    const resolvedContext = metadata?.title.trim();
    const label =
      resolvedContext &&
      (parsed.value.type === "issue" || parsed.value.type === "pr")
        ? `${parsed.value.dtag} · ${resolvedContext}`
        : presentation.label;
    return (
      <BuzzLinkChip
        data-buzz-link-kind={parsed.value.type}
        href={href}
        icon={presentation.icon}
        aria-label={presentation.ariaLabel}
        className="max-w-64"
        interactive={interactive}
        onOpenLink={() => onOpenEntityLink(parsed.value)}
      >
        <span className="truncate">{label}</span>
      </BuzzLinkChip>
    );
  };
  return interactive ? (
    <EntityMetadataTooltip
      fallback={presentation.label}
      footer={presentation.tooltipFooter}
      href={canonicalHref}
    >
      {chip}
    </EntityMetadataTooltip>
  ) : (
    chip()
  );
}
