import * as React from "react";
import { ChevronDown } from "lucide-react";

import { cn } from "@/shared/lib/cn";
import { rewriteRelayUrl } from "@/shared/lib/mediaUrl";
import type { AgentActivityAction } from "../agentSessionTypes";
import type { CompactFileEditSummary } from "../agentSessionToolSummary";
import { isInlineImageData } from "../agentSessionUtils";
import {
  ActivityRowLabel,
  splitActivityRowLabel,
  type ActivityRowLabelParts,
} from "../activityRenderClasses/ActivityRow";

export function compactSummaryTone() {
  return "text-muted-foreground/60 group-open:text-foreground";
}

export function CompactToolSummaryRow({
  action,
  duration,
  fileEditSummary,
  label,
  preview,
  thumbnailSrc,
}: {
  action: AgentActivityAction | null;
  duration: string | null;
  fileEditSummary: CompactFileEditSummary | null;
  label: string;
  preview: string | null;
  thumbnailSrc: string | null;
}) {
  const [thumbnailFailed, setThumbnailFailed] = React.useState(false);
  const mutedTone = compactSummaryTone();
  const resolvedThumbnail = React.useMemo(() => {
    if (!thumbnailSrc || thumbnailFailed) return null;
    return resolveImageSrc(thumbnailSrc);
  }, [thumbnailFailed, thumbnailSrc]);
  const actionLabel = fileEditSummary
    ? null
    : getCompactToolActionLabel(action, label, preview);

  return (
    <>
      {fileEditSummary ? (
        <CompactFileEditSummaryView summary={fileEditSummary} />
      ) : actionLabel ? (
        <ActivityRowLabel
          object={actionLabel.object}
          openToneScope="tool"
          title={actionLabel.title}
          verb={actionLabel.verb}
        />
      ) : (
        <span className={cn("shrink-0 text-sm font-semibold", mutedTone)}>
          {label}
        </span>
      )}
      {!fileEditSummary && resolvedThumbnail ? (
        <img
          alt=""
          className="h-5 w-auto max-w-12 shrink-0 rounded-sm object-cover"
          decoding="async"
          loading="lazy"
          onError={() => setThumbnailFailed(true)}
          src={resolvedThumbnail}
          title={preview ?? undefined}
        />
      ) : !fileEditSummary && !actionLabel && preview ? (
        <span
          className={cn("min-w-0 max-w-48 truncate text-sm", mutedTone)}
          title={preview}
        >
          {preview}
        </span>
      ) : null}
      {duration ? (
        <span className={cn("shrink-0 text-xs", mutedTone)}>{duration}</span>
      ) : null}
      <ChevronDown
        className={cn(
          "h-3.5 w-3.5 shrink-0 transition-transform group-open:rotate-180",
          mutedTone,
        )}
      />
    </>
  );
}

function getCompactToolActionLabel(
  action: AgentActivityAction | null,
  label: string,
  preview: string | null,
): (ActivityRowLabelParts & { title?: string }) | null {
  if (action) {
    const object = action.object ?? preview ?? undefined;
    return {
      verb: action.verb,
      object,
      title: typeof object === "string" ? object : undefined,
    };
  }

  const parts = splitActivityRowLabel(label);
  if (!parts) return null;

  if (!preview) return parts;

  if (
    label === "Ran command" ||
    label === "Read file" ||
    label === "Updated todos" ||
    label === "Viewed image"
  ) {
    return { verb: parts.verb, object: preview, title: preview };
  }

  return parts;
}

function CompactFileEditSummaryView({
  summary,
}: {
  summary: CompactFileEditSummary;
}) {
  return (
    <ActivityRowLabel
      className="max-w-72"
      object={summary.filename}
      openToneScope="tool"
      stats={{
        additions: summary.additions,
        deletions: summary.deletions,
      }}
      title={summary.path}
      verb="Edited"
    />
  );
}

function resolveImageSrc(source: string): string {
  return isInlineImageData(source) ? source : rewriteRelayUrl(source);
}
