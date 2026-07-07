import type * as React from "react";
import { ChevronDown, Circle } from "lucide-react";

import { formatTranscriptTimestampTitle } from "@/features/agents/ui/agentSessionUtils";
import type { ActivityMarkerTone } from "@/features/chats/ui/chatActivityText";
import { cn } from "@/shared/lib/cn";
import { Marker, MarkerContent, MarkerIcon } from "@/shared/ui/marker";
import { Message, MessageContent } from "@/shared/ui/message";

export function ActivityMarkerRow({
  details,
  entrance = false,
  icon,
  label,
  loading = false,
  meta,
  timestamp,
  tone = "default",
}: {
  details?: React.ReactNode;
  entrance?: boolean;
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
    <Message
      className={cn("py-1.5", entrance && "buzz-message-entrance")}
      side="left"
    >
      <MessageContent className="w-full max-w-full">
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
                <MarkerContent>
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
            <MarkerContent>
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
          <MarkerContent>
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
  // The shimmer ::before overlay repeats the text via data-shimmer-text, so
  // it can only be applied where the label is a plain string that exactly
  // matches the rendered text.
  const shimmer = loading && typeof label === "string";
  return (
    <span
      className={cn("min-w-0 truncate", shimmer && "shimmer")}
      data-loading={loading || undefined}
      data-shimmer-text={shimmer ? label : undefined}
    >
      {label}
    </span>
  );
}

function MarkerMeta({ loading, meta }: { loading: boolean; meta: string }) {
  return (
    <span
      className={cn(
        "shrink-0 text-2xs text-muted-foreground/70",
        loading && "shimmer",
      )}
      data-loading={loading || undefined}
      data-shimmer-text={loading ? meta : undefined}
    >
      {meta}
    </span>
  );
}

function markerToneClass(tone: ActivityMarkerTone) {
  if (tone === "danger") return "text-destructive";
  // Softer than plain muted-foreground: marker rows are plumbing, and at
  // full muted strength they still competed with the conversation text.
  return "text-muted-foreground/70";
}
