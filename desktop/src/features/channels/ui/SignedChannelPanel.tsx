import {
  AlertTriangle,
  ArrowUpRight,
  CheckCircle2,
  CircleDashed,
  Clock3,
  ExternalLink,
  Info,
  LoaderCircle,
  ShieldCheck,
  TriangleAlert,
  XCircle,
} from "lucide-react";
import type * as React from "react";

import {
  AuxiliaryPanelBody,
  AuxiliaryPanelHeader,
  AuxiliaryPanelHeaderActions,
  AuxiliaryPanelHeaderGroup,
  AuxiliaryPanelTitle,
  useAuxiliaryPanel,
  type AuxiliaryPanelMode,
} from "@/shared/layout/AuxiliaryPanel";
import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import type {
  PanelField,
  PanelLink,
  PanelManifest,
  PanelSection,
  PanelSourceEvent,
  PanelStatus,
  SignedChannelPanelState,
} from "./signedChannelPanelTypes";

type SignedChannelPanelProps = {
  channelName: string;
  state: SignedChannelPanelState;
  mode: AuxiliaryPanelMode;
  onOpenSourceEvent?: (eventId: string) => void;
};

const STATUS_LABELS: Record<PanelStatus, string> = {
  pending: "Pending",
  active: "Active",
  complete: "Complete",
  blocked: "Blocked",
  failed: "Failed",
  stale: "Stale",
  unavailable: "Unavailable",
};

const STATUS_STYLES: Record<PanelStatus, string> = {
  pending: "border-border bg-muted/60 text-muted-foreground",
  active: "border-blue-500/30 bg-blue-500/10 text-blue-700 dark:text-blue-300",
  complete:
    "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  blocked:
    "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  failed: "border-destructive/30 bg-destructive/10 text-destructive",
  stale:
    "border-amber-500/30 bg-amber-500/10 text-amber-700 dark:text-amber-300",
  unavailable: "border-border bg-muted/60 text-muted-foreground",
};

export function SignedChannelPanel({
  channelName,
  mode,
  onOpenSourceEvent,
  state,
}: SignedChannelPanelProps) {
  const panel = useAuxiliaryPanel();

  return (
    <div
      className="flex min-h-0 flex-1 flex-col bg-background"
      data-testid="signed-channel-panel"
    >
      <AuxiliaryPanelHeader
        bordered={mode === "panel"}
        density={mode === "panel" ? "compact" : "comfortable"}
        mode={mode}
        transparent={panel.transparentChrome}
      >
        <AuxiliaryPanelHeaderGroup>
          <AuxiliaryPanelTitle>Panel</AuxiliaryPanelTitle>
        </AuxiliaryPanelHeaderGroup>
        <AuxiliaryPanelHeaderActions />
      </AuxiliaryPanelHeader>

      <AuxiliaryPanelBody
        className="overflow-y-auto overflow-x-hidden overscroll-contain bg-background px-4 pb-8"
        mode={mode}
        panelPadding
      >
        <div className="space-y-5 pt-3">
          <div>
            <p className="text-sm text-muted-foreground">
              Signed channel workspace
            </p>
            <h3 className="mt-1 text-lg font-semibold tracking-tight">
              #{channelName}
            </h3>
          </div>

          {renderState({ onOpenSourceEvent, state })}
        </div>
      </AuxiliaryPanelBody>
    </div>
  );
}

function renderState({
  onOpenSourceEvent,
  state,
}: {
  onOpenSourceEvent?: (eventId: string) => void;
  state: SignedChannelPanelState;
}) {
  switch (state.kind) {
    case "loading":
      return <PanelLoadingState />;
    case "empty":
      return <PanelEmptyState message={state.message} />;
    case "unavailable":
      return <PanelUnavailableState message={state.message} />;
    case "invalid":
      return <PanelInvalidState message={state.message} />;
    case "ready":
    case "stale":
      return (
        <PanelManifestView
          manifest={state.manifest}
          onOpenSourceEvent={onOpenSourceEvent}
          stale={state.kind === "stale"}
        />
      );
  }
}

function PanelLoadingState() {
  return (
    <PanelNotice
      icon={<LoaderCircle className="animate-spin" />}
      label="Loading signed panel"
      message="The panel will remain here while its signed source is resolved."
      role="status"
      testId="signed-channel-panel-loading"
    />
  );
}

function PanelEmptyState({ message }: { message?: string }) {
  return (
    <PanelNotice
      icon={<CircleDashed />}
      label="No panel published"
      message={
        message ??
        "This channel has no signed panel projection yet. Source events remain available in the channel timeline."
      }
      role="status"
      testId="signed-channel-panel-empty"
    />
  );
}

function PanelUnavailableState({ message }: { message: string }) {
  return (
    <PanelNotice
      icon={<Info />}
      label="Panel unavailable"
      message={message}
      role="status"
      testId="signed-channel-panel-unavailable"
    />
  );
}

function PanelInvalidState({ message }: { message: string }) {
  return (
    <PanelNotice
      icon={<XCircle />}
      label="Panel could not be displayed"
      message={message}
      role="alert"
      testId="signed-channel-panel-invalid"
    />
  );
}

function PanelNotice({
  icon,
  label,
  message,
  role,
  testId,
}: {
  icon: React.ReactNode;
  label: string;
  message: string;
  role: "alert" | "status";
  testId: string;
}) {
  return (
    <div
      aria-live={role === "status" ? "polite" : undefined}
      className="rounded-2xl border border-border/70 bg-card/60 p-4"
      data-testid={testId}
      role={role}
    >
      <div className="flex items-start gap-3">
        <span className="mt-0.5 shrink-0 text-muted-foreground">{icon}</span>
        <div className="min-w-0 space-y-1">
          <p className="font-medium text-foreground">{label}</p>
          <p className="text-sm leading-5 text-muted-foreground">{message}</p>
        </div>
      </div>
    </div>
  );
}

function PanelManifestView({
  manifest,
  onOpenSourceEvent,
  stale,
}: {
  manifest: PanelManifest;
  onOpenSourceEvent?: (eventId: string) => void;
  stale: boolean;
}) {
  return (
    <div className="space-y-5" data-testid="signed-channel-panel-ready">
      {stale ? (
        <div
          className="flex items-start gap-2 rounded-xl border border-amber-500/30 bg-amber-500/10 px-3 py-2 text-sm text-amber-800 dark:text-amber-200"
          data-testid="signed-channel-panel-stale"
          role="status"
        >
          <TriangleAlert className="mt-0.5 h-4 w-4 shrink-0" />
          <span>
            This projection may no longer reflect current channel state.
          </span>
        </div>
      ) : null}

      <div className="space-y-2">
        <div className="flex flex-wrap items-center gap-2">
          <h4 className="min-w-0 flex-1 text-base font-semibold">
            {manifest.title}
          </h4>
          <PanelStatusBadge status={manifest.status} />
        </div>
        {manifest.description ? (
          <p className="text-sm leading-5 text-muted-foreground">
            {manifest.description}
          </p>
        ) : null}
        <div className="flex items-center gap-1.5 text-2xs text-muted-foreground">
          <Clock3 className="h-3.5 w-3.5" />
          <span>Updated {formatPanelTimestamp(manifest.updatedAt)}</span>
        </div>
      </div>

      <div className="space-y-4">
        {manifest.sections.map((section) => (
          <PanelSectionView
            key={section.id}
            onOpenSourceEvent={onOpenSourceEvent}
            section={section}
          />
        ))}
      </div>

      <PanelProvenance
        onOpenSourceEvent={onOpenSourceEvent}
        sourceEvents={manifest.sourceEvents}
      />
    </div>
  );
}

function PanelSectionView({
  onOpenSourceEvent,
  section,
}: {
  onOpenSourceEvent?: (eventId: string) => void;
  section: PanelSection;
}) {
  return (
    <section
      aria-labelledby={`signed-panel-section-${section.id}`}
      className="space-y-3 rounded-2xl border border-border/70 bg-card/40 p-4"
      data-testid={`signed-channel-panel-section-${section.id}`}
    >
      <div className="flex flex-wrap items-center gap-2">
        <h5
          className="min-w-0 flex-1 text-sm font-semibold"
          id={`signed-panel-section-${section.id}`}
        >
          {section.title}
        </h5>
        <PanelStatusBadge status={section.status} />
      </div>
      {section.fields.length > 0 ? (
        <dl className="divide-y divide-border/60 rounded-xl border border-border/60 bg-background/40">
          {section.fields.map((field) => (
            <PanelFieldView field={field} key={field.label} />
          ))}
        </dl>
      ) : null}
      {section.links.length > 0 ? (
        <div className="flex flex-wrap gap-2">
          {section.links.map((link) => (
            <PanelLinkView
              key={`${link.label}-${link.sourceEventId ?? link.uri ?? link.target}`}
              link={link}
              onOpenSourceEvent={onOpenSourceEvent}
            />
          ))}
        </div>
      ) : null}
    </section>
  );
}

function PanelFieldView({ field }: { field: PanelField }) {
  const isStatus = field.presentation === "status";
  return (
    <div className="grid grid-cols-[minmax(0,0.8fr)_minmax(0,1.2fr)] gap-3 px-3 py-2.5 text-sm">
      <dt className="text-muted-foreground">{field.label}</dt>
      <dd
        className={cn(
          "min-w-0 break-words text-right text-foreground",
          field.presentation === "monospace" && "font-mono text-xs",
          field.presentation === "timestamp" && "text-muted-foreground",
        )}
      >
        {isStatus ? (
          <span className="inline-flex justify-end">
            <PanelStatusBadge status={normalizeStatus(field.value)} />
          </span>
        ) : field.presentation === "timestamp" ? (
          formatPanelTimestamp(Number(field.value))
        ) : (
          field.value
        )}
      </dd>
    </div>
  );
}

function PanelLinkView({
  link,
  onOpenSourceEvent,
}: {
  link: PanelLink;
  onOpenSourceEvent?: (eventId: string) => void;
}) {
  if (link.target === "external" && link.uri?.startsWith("https://")) {
    return (
      <a
        className="inline-flex min-h-8 items-center gap-1.5 rounded-lg border border-border/70 px-2.5 text-xs font-medium text-foreground transition-colors hover:bg-muted focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
        href={link.uri}
        rel="noreferrer"
        target="_blank"
      >
        {link.label}
        <ExternalLink className="h-3.5 w-3.5" />
      </a>
    );
  }

  if (link.sourceEventId) {
    return (
      <Button
        className="h-8 gap-1.5 px-2.5 text-xs"
        onClick={() => onOpenSourceEvent?.(link.sourceEventId ?? "")}
        size="sm"
        type="button"
        variant="outline"
      >
        {link.label}
        <ArrowUpRight className="h-3.5 w-3.5" />
      </Button>
    );
  }

  return (
    <span className="inline-flex min-h-8 items-center gap-1.5 rounded-lg border border-border/50 px-2.5 text-xs text-muted-foreground">
      <AlertTriangle className="h-3.5 w-3.5" />
      {link.label}
    </span>
  );
}

function PanelProvenance({
  onOpenSourceEvent,
  sourceEvents,
}: {
  onOpenSourceEvent?: (eventId: string) => void;
  sourceEvents: PanelSourceEvent[];
}) {
  return (
    <div
      className="space-y-2 border-t border-border/60 pt-4"
      data-testid="signed-channel-panel-provenance"
    >
      <div className="flex items-center gap-2 text-xs font-medium text-foreground">
        <ShieldCheck className="h-4 w-4 text-muted-foreground" />
        Signed sources
      </div>
      <ul className="space-y-1.5">
        {sourceEvents.map((source) => (
          <li key={source.eventId}>
            <button
              className="flex w-full items-center justify-between gap-3 rounded-lg px-2 py-1.5 text-left text-2xs text-muted-foreground transition-colors hover:bg-muted focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-ring"
              onClick={() => onOpenSourceEvent?.(source.eventId)}
              title={source.eventId}
              type="button"
            >
              <span className="min-w-0 truncate">{source.label}</span>
              <span className="shrink-0 font-mono">
                {shortEventId(source.eventId)}
              </span>
            </button>
          </li>
        ))}
      </ul>
    </div>
  );
}

function PanelStatusBadge({ status }: { status: PanelStatus }) {
  const Icon =
    status === "complete"
      ? CheckCircle2
      : status === "failed"
        ? XCircle
        : status === "blocked" || status === "stale"
          ? TriangleAlert
          : status === "active"
            ? LoaderCircle
            : Info;
  return (
    <Badge
      className={cn("gap-1 border text-2xs", STATUS_STYLES[status])}
      data-testid={`signed-channel-panel-status-${status}`}
      variant="outline"
    >
      <Icon className={cn("h-3 w-3", status === "active" && "animate-spin")} />
      {STATUS_LABELS[status]}
    </Badge>
  );
}

function normalizeStatus(value: string): PanelStatus {
  return value.toLowerCase() in STATUS_LABELS
    ? (value.toLowerCase() as PanelStatus)
    : "pending";
}

function formatPanelTimestamp(unixSeconds: number) {
  if (!Number.isFinite(unixSeconds) || unixSeconds <= 0) {
    return "an unknown time";
  }
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(new Date(unixSeconds * 1_000));
}

function shortEventId(eventId: string) {
  return eventId.length > 16
    ? `${eventId.slice(0, 8)}…${eventId.slice(-8)}`
    : eventId;
}
