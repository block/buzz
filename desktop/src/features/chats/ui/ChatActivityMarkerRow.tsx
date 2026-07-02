import type * as React from "react";
import { ChevronDown, Circle } from "lucide-react";

import { formatTranscriptTimestampTitle } from "@/features/agents/ui/agentSessionUtils";
import type { ActivityMarkerTone } from "@/features/chats/ui/chatActivityText";
import { cn } from "@/shared/lib/cn";
import { Marker, MarkerContent, MarkerIcon } from "@/shared/ui/marker";
import { Message, MessageContent } from "@/shared/ui/message";

export function ActivityMarkerRow({
  details,
  icon,
  label,
  loading = false,
  meta,
  timestamp,
  tone = "default",
}: {
  details?: React.ReactNode;
  icon?: React.ReactNode;
  label: React.ReactNode;
  loading?: boolean;
  meta?: string | null;
  timestamp?: string;
  tone?: ActivityMarkerTone;
}) {
  const title = timestamp
    ? formatTranscriptTimestampTitle(timestamp)
    : undefined;
  const statusProps = loading ? statusMarkerProps : {};

  return (
    <Message className="py-1.5" side="left">
      <MessageContent className="max-w-[min(42rem,78%)]">
        {details ? (
          <details className="group/activity-marker" title={title}>
            <summary className="list-none">
              <Marker
                className={cn("cursor-pointer", markerToneClass(tone))}
                {...statusProps}
              >
                <MarkerIcon>
                  {icon ?? <Circle className="size-3.5" />}
                </MarkerIcon>
                <MarkerContent
                  className={cn(loading && "shimmer")}
                  data-shimmer-text={
                    loading ? markerText(label, meta) : undefined
                  }
                >
                  <MarkerRowContent
                    label={label}
                    loading={loading}
                    meta={meta}
                    showChevron
                  />
                </MarkerContent>
              </Marker>
            </summary>
            <div className="mt-3 pl-6 text-sm text-muted-foreground">
              {details}
            </div>
          </details>
        ) : (
          <Marker
            className={markerToneClass(tone)}
            title={title}
            {...statusProps}
          >
            <MarkerIcon>{icon ?? <Circle className="size-3.5" />}</MarkerIcon>
            <MarkerContent
              className={cn(loading && "shimmer")}
              data-shimmer-text={loading ? markerText(label, meta) : undefined}
            >
              <MarkerRowContent label={label} loading={loading} meta={meta} />
            </MarkerContent>
          </Marker>
        )}
      </MessageContent>
    </Message>
  );
}

export function InlineActivityMarkerRow({
  details,
  icon,
  label,
  loading = false,
  timestamp,
  tone = "default",
}: {
  details?: React.ReactNode;
  icon?: React.ReactNode;
  label: React.ReactNode;
  loading?: boolean;
  timestamp?: string;
  tone?: ActivityMarkerTone;
}) {
  const title = timestamp
    ? formatTranscriptTimestampTitle(timestamp)
    : undefined;
  const statusProps = loading ? statusMarkerProps : {};

  return (
    <details className="group/inline-marker py-1" title={title}>
      <summary className="list-none">
        <Marker
          className={cn("cursor-pointer", markerToneClass(tone))}
          {...statusProps}
        >
          <MarkerIcon>{icon ?? <Circle className="size-3.5" />}</MarkerIcon>
          <MarkerContent
            className={cn(loading && "shimmer")}
            data-shimmer-text={loading ? markerText(label) : undefined}
          >
            <MarkerRowContent label={label} loading={loading} showChevron />
          </MarkerContent>
        </Marker>
      </summary>
      <div className="mt-3 pl-6 text-sm text-muted-foreground">
        {details ?? "No additional details."}
      </div>
    </details>
  );
}

const statusMarkerProps = {
  "aria-live": "polite",
  role: "status",
} as const;

function MarkerRowContent({
  label,
  loading,
  meta,
  showChevron = false,
}: {
  label: React.ReactNode;
  loading: boolean;
  meta?: string | null;
  showChevron?: boolean;
}) {
  return (
    <span className="flex min-w-0 items-center gap-1.5">
      <MarkerLabel label={label} loading={loading} />
      {meta ? <MarkerMeta loading={loading} meta={meta} /> : null}
      {showChevron ? (
        <span className="grid h-3.5 w-3.5 shrink-0 place-items-center">
          <ChevronDown className="h-3.5 w-3.5 text-muted-foreground/60 opacity-0 transition-[opacity,transform] group-hover/activity-marker:opacity-100 group-hover/inline-marker:opacity-100 group-open/activity-marker:rotate-180 group-open/activity-marker:opacity-100 group-open/inline-marker:rotate-180 group-open/inline-marker:opacity-100" />
        </span>
      ) : null}
    </span>
  );
}

function MarkerLabel({
  label,
  loading,
}: {
  label: React.ReactNode;
  loading: boolean;
}) {
  return (
    <span className="min-w-0 truncate" data-loading={loading || undefined}>
      {label}
    </span>
  );
}

function MarkerMeta({ loading, meta }: { loading: boolean; meta: string }) {
  return (
    <span
      className="shrink-0 text-2xs text-muted-foreground/70"
      data-loading={loading || undefined}
    >
      {meta}
    </span>
  );
}

function markerText(label: React.ReactNode, meta?: string | null) {
  return [typeof label === "string" ? label : null, meta]
    .filter(Boolean)
    .join(" ");
}

function markerToneClass(tone: ActivityMarkerTone) {
  if (tone === "danger") return "text-destructive";
  if (tone === "success" || tone === "warning" || tone === "muted") {
    return "text-muted-foreground";
  }
  return "text-muted-foreground";
}
