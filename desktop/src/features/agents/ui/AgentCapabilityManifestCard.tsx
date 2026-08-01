import * as React from "react";
import {
  AlertTriangle,
  Check,
  CircleHelp,
  Minus,
  RadioTower,
  ShieldCheck,
  Wrench,
} from "lucide-react";

import { useAcpRuntimesQuery } from "@/features/agents/hooks";
import { useManagedAgentRuntimesQuery } from "@/features/agents/managedAgentRuntimeHooks";
import { findManagedAgentRuntime } from "@/features/agents/managedAgentRuntimeStatus";
import {
  type AgentCapabilityManifest,
  type CapabilityEvidenceState,
  type ManifestOverallStatus,
  type ReadinessStatus,
  buildAgentCapabilityManifest,
  findManifestRuntime,
} from "@/features/agents/lib/capabilityManifest";
import { useObserverEvents } from "@/features/agents/ui/useObserverEvents";
import type { ManagedAgent } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";

type AgentCapabilityManifestCardProps = {
  agent: ManagedAgent;
  presenceStatus: "online" | "away" | "offline" | undefined;
};

const overallStatusPresentation: Record<
  ManifestOverallStatus,
  {
    label: string;
    variant: "destructive" | "outline" | "secondary";
    className?: string;
  }
> = {
  ready: {
    label: "Ready",
    variant: "outline",
    className:
      "border-emerald-500/30 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
  },
  attention: { label: "Needs attention", variant: "destructive" },
  stopped: { label: "Stopped", variant: "secondary" },
  unknown: { label: "Not fully verified", variant: "secondary" },
};

const readinessDotClass: Record<ReadinessStatus, string> = {
  ready: "bg-emerald-500",
  attention: "bg-destructive",
  pending: "bg-amber-500",
  unknown: "bg-muted-foreground/40",
};

const evidencePresentation: Record<
  CapabilityEvidenceState,
  { className: string; icon: typeof Check }
> = {
  reported: {
    className:
      "border-emerald-500/25 bg-emerald-500/10 text-emerald-700 dark:text-emerald-300",
    icon: Check,
  },
  unavailable: {
    className: "border-border/70 bg-muted/30 text-muted-foreground",
    icon: Minus,
  },
  unknown: {
    className:
      "border-amber-500/25 bg-amber-500/10 text-amber-700 dark:text-amber-300",
    icon: CircleHelp,
  },
};

function queryTimestamp(dataUpdatedAt: number): string | null {
  return dataUpdatedAt > 0 ? new Date(dataUpdatedAt).toISOString() : null;
}

export function AgentCapabilityManifestCard({
  agent,
  presenceStatus,
}: AgentCapabilityManifestCardProps) {
  const runtimeCatalogQuery = useAcpRuntimesQuery();
  const runtimeStatusesQuery = useManagedAgentRuntimesQuery();
  const observer = useObserverEvents(true, agent.pubkey);
  const runtime = React.useMemo(
    () => findManifestRuntime(agent, runtimeCatalogQuery.data ?? []),
    [agent, runtimeCatalogQuery.data],
  );
  const runtimeStatus = React.useMemo(
    () =>
      findManagedAgentRuntime(
        runtimeStatusesQuery.data ?? [],
        agent.pubkey,
        agent.relayUrl,
      ),
    [agent.pubkey, agent.relayUrl, runtimeStatusesQuery.data],
  );
  const manifest = React.useMemo(
    () =>
      buildAgentCapabilityManifest({
        agent,
        runtime,
        runtimeStatus,
        presenceStatus,
        observer,
        catalogObservedAt: queryTimestamp(runtimeCatalogQuery.dataUpdatedAt),
        runtimeObservedAt: queryTimestamp(runtimeStatusesQuery.dataUpdatedAt),
      }),
    [
      agent,
      observer,
      presenceStatus,
      runtime,
      runtimeCatalogQuery.dataUpdatedAt,
      runtimeStatus,
      runtimeStatusesQuery.dataUpdatedAt,
    ],
  );

  return <AgentCapabilityManifestView manifest={manifest} />;
}

export function AgentCapabilityManifestView({
  manifest,
}: {
  manifest: AgentCapabilityManifest;
}) {
  return (
    <section
      aria-label="Agent capability and readiness manifest"
      className="overflow-hidden rounded-2xl border border-border/70 bg-card/60 shadow-sm"
      data-testid="agent-capability-manifest"
    >
      <ManifestHeader manifest={manifest} />
      <div className="space-y-5 px-4 py-4">
        <div
          className="space-y-5"
          data-testid="agent-capability-readiness-evidence"
        >
          <ReadinessGrid checks={manifest.readiness} />
          <IdentityGrid manifest={manifest} />
        </div>
        <div
          className="space-y-5"
          data-testid="agent-capability-delegation-evidence"
        >
          <CapabilitySection manifest={manifest} />
          <PermissionSection manifest={manifest} />
          <ToolsSection manifest={manifest} />
        </div>
        {manifest.limitations.length > 0 ? (
          <div
            className="rounded-xl border border-dashed border-border/80 bg-muted/15 px-3 py-2.5"
            data-testid="agent-capability-limitations"
          >
            <p className="text-xs font-medium text-foreground">
              Unreported or limited
            </p>
            <ul className="mt-1.5 space-y-1 text-xs text-muted-foreground">
              {manifest.limitations.map((limitation) => (
                <li className="flex gap-2" key={limitation}>
                  <CircleHelp className="mt-0.5 h-3 w-3 shrink-0" />
                  <span>{limitation}</span>
                </li>
              ))}
            </ul>
          </div>
        ) : null}
      </div>
    </section>
  );
}

function ManifestHeader({ manifest }: { manifest: AgentCapabilityManifest }) {
  const status = overallStatusPresentation[manifest.overallStatus];
  const lastVerifiedLabel = manifest.lastVerifiedAt
    ? formatVerifiedTime(manifest.lastVerifiedAt)
    : "Never verified";

  return (
    <div className="flex items-start justify-between gap-3 border-b border-border/60 bg-muted/15 px-4 py-3.5">
      <div className="min-w-0">
        <div className="flex items-center gap-2">
          <ShieldCheck className="h-4 w-4 shrink-0 text-muted-foreground" />
          <h3 className="text-sm font-semibold text-foreground">
            Readiness & capabilities
          </h3>
        </div>
        <p className="mt-1 text-xs text-muted-foreground">
          Owner-only evidence from Buzz and the encrypted ACP observer feed.
        </p>
      </div>
      <div className="flex shrink-0 flex-col items-end gap-1">
        <Badge
          className={status.className}
          data-testid="agent-capability-overall-status"
          variant={status.variant}
        >
          {status.label}
        </Badge>
        <time
          className="text-2xs text-muted-foreground"
          dateTime={manifest.lastVerifiedAt ?? undefined}
          title={manifest.lastVerifiedAt ?? undefined}
        >
          {lastVerifiedLabel}
        </time>
        {manifest.freshness === "stale" ? (
          <span className="text-2xs font-medium text-amber-600 dark:text-amber-400">
            Live evidence is stale
          </span>
        ) : null}
      </div>
    </div>
  );
}

function ReadinessGrid({
  checks,
}: {
  checks: AgentCapabilityManifest["readiness"];
}) {
  return (
    <div>
      <SectionLabel icon={RadioTower} label="Readiness" />
      <div className="mt-2 grid grid-cols-2 gap-2">
        {checks.map((check) => (
          <div
            className="min-w-0 rounded-xl bg-muted/25 px-3 py-2"
            data-status={check.status}
            data-testid={`agent-readiness-${check.id}`}
            key={check.id}
          >
            <div className="flex items-center gap-1.5">
              <ReadinessDot status={check.status} />
              <span className="text-xs font-medium text-foreground">
                {check.label}
              </span>
            </div>
            <p
              className="mt-0.5 truncate pl-3.5 text-2xs capitalize text-muted-foreground"
              title={check.detail}
            >
              {check.detail}
            </p>
          </div>
        ))}
      </div>
    </div>
  );
}

function ReadinessDot({ status }: { status: ReadinessStatus }) {
  return (
    <span
      aria-label={status}
      className={cn("h-2 w-2 shrink-0 rounded-full", readinessDotClass[status])}
      role="img"
    />
  );
}

function IdentityGrid({ manifest }: { manifest: AgentCapabilityManifest }) {
  const rows = [
    {
      label: "Runtime",
      value: manifest.runtime.version
        ? `${manifest.runtime.label} ${manifest.runtime.version}`
        : manifest.runtime.label,
    },
    {
      label: "ACP protocol",
      value: manifest.protocolVersion ?? "Unknown",
    },
    {
      label: "Model",
      value: manifest.model.value
        ? `${manifest.model.value} (${manifest.model.source})`
        : "Unknown",
    },
    {
      label: "Provider",
      value: manifest.provider.value
        ? `${manifest.provider.value} (${manifest.provider.source})`
        : "Unknown",
    },
  ];
  return (
    <div className="grid grid-cols-2 gap-x-4 gap-y-3 border-y border-border/60 py-3">
      {rows.map((row) => (
        <div className="min-w-0" key={row.label}>
          <p className="text-2xs font-medium uppercase tracking-wide text-muted-foreground">
            {row.label}
          </p>
          <p
            className="mt-0.5 truncate text-xs font-medium text-foreground"
            title={row.value}
          >
            {row.value}
          </p>
        </div>
      ))}
    </div>
  );
}

function CapabilitySection({
  manifest,
}: {
  manifest: AgentCapabilityManifest;
}) {
  return (
    <div>
      <SectionLabel icon={Check} label="Features" />
      <div className="mt-2 flex flex-wrap gap-1.5">
        {manifest.features.map((feature) => (
          <EvidencePill
            key={feature.id}
            label={feature.label}
            source={feature.source === "runtime" ? "Runtime" : "Buzz catalog"}
            state={feature.state}
          />
        ))}
      </div>
    </div>
  );
}

function PermissionSection({
  manifest,
}: {
  manifest: AgentCapabilityManifest;
}) {
  const permission = manifest.permissionMode;
  const differs =
    permission.requested !== null &&
    permission.effective !== null &&
    permission.requested !== permission.effective;
  return (
    <div
      className={cn(
        "rounded-xl border px-3 py-2.5",
        differs
          ? "border-amber-500/30 bg-amber-500/10"
          : "border-border/70 bg-muted/15",
      )}
      data-testid="agent-capability-permission-mode"
    >
      <div className="flex items-center gap-2">
        {differs ? (
          <AlertTriangle className="h-3.5 w-3.5 text-amber-600 dark:text-amber-400" />
        ) : (
          <ShieldCheck className="h-3.5 w-3.5 text-muted-foreground" />
        )}
        <p className="text-xs font-medium text-foreground">Permission mode</p>
      </div>
      <div className="mt-1.5 grid grid-cols-2 gap-3 text-xs">
        <KeyValue label="Requested" value={permission.requested ?? "Unknown"} />
        <KeyValue label="Effective" value={permission.effective ?? "Unknown"} />
      </div>
      {differs ? (
        <p className="mt-2 text-2xs text-amber-700 dark:text-amber-300">
          Effective behavior differs from the requested runtime mode.
        </p>
      ) : null}
    </div>
  );
}

function ToolsSection({ manifest }: { manifest: AgentCapabilityManifest }) {
  return (
    <div>
      <SectionLabel icon={Wrench} label="Commands & tools" />
      <div className="mt-2 space-y-2">
        <TokenRow
          emptyLabel={
            manifest.toolSourcesState === "unavailable"
              ? "No MCP tool sources reported"
              : "Tool sources unknown"
          }
          label="Sources"
          tokens={manifest.toolSources}
        />
        <TokenRow
          emptyLabel={
            manifest.commandsState === "unavailable"
              ? "No runtime commands reported"
              : "Runtime commands unknown"
          }
          label="Commands"
          tokens={manifest.commands}
        />
        {manifest.tools.length > 0 ? (
          <div className="overflow-hidden rounded-xl border border-border/70">
            {manifest.tools.map((tool) => (
              <div
                className="flex items-center gap-2 border-b border-border/50 px-3 py-2 last:border-b-0"
                key={`${tool.source ?? "unknown"}:${tool.name}`}
              >
                <span className="min-w-0 flex-1 truncate text-xs font-medium text-foreground">
                  {tool.name}
                </span>
                <span className="text-2xs text-muted-foreground">
                  {tool.source ?? "Source unknown"}
                </span>
                <Badge variant="outline">{tool.riskClass}</Badge>
                <EvidenceIcon state={tool.availability} />
              </div>
            ))}
          </div>
        ) : null}
      </div>
    </div>
  );
}

function TokenRow({
  emptyLabel,
  label,
  tokens,
}: {
  emptyLabel: string;
  label: string;
  tokens: string[];
}) {
  return (
    <div className="flex items-start gap-3 rounded-xl bg-muted/20 px-3 py-2">
      <span className="w-16 shrink-0 text-2xs font-medium uppercase tracking-wide text-muted-foreground">
        {label}
      </span>
      <div className="flex min-w-0 flex-1 flex-wrap gap-1">
        {tokens.length > 0 ? (
          tokens.map((token) => (
            <span
              className="max-w-full truncate rounded-md bg-background/80 px-1.5 py-0.5 font-mono text-2xs text-foreground"
              key={token}
              title={token}
            >
              {token}
            </span>
          ))
        ) : (
          <span className="text-2xs text-muted-foreground">{emptyLabel}</span>
        )}
      </div>
    </div>
  );
}

function EvidencePill({
  label,
  source,
  state,
}: {
  label: string;
  source: string;
  state: CapabilityEvidenceState;
}) {
  const presentation = evidencePresentation[state];
  return (
    <span
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full border px-2 py-1 text-2xs",
        presentation.className,
      )}
      data-state={state}
      title={`${source}: ${state}`}
    >
      <EvidenceIcon state={state} />
      {label}
    </span>
  );
}

function EvidenceIcon({ state }: { state: CapabilityEvidenceState }) {
  const Icon = evidencePresentation[state].icon;
  return <Icon aria-hidden="true" className="h-3 w-3" />;
}

function KeyValue({ label, value }: { label: string; value: string }) {
  return (
    <div className="min-w-0">
      <p className="text-2xs uppercase tracking-wide text-muted-foreground">
        {label}
      </p>
      <p className="truncate font-mono text-xs text-foreground" title={value}>
        {value}
      </p>
    </div>
  );
}

function SectionLabel({
  icon: Icon,
  label,
}: {
  icon: typeof Check;
  label: string;
}) {
  return (
    <div className="flex items-center gap-1.5 text-xs font-medium text-foreground">
      <Icon className="h-3.5 w-3.5 text-muted-foreground" />
      <span>{label}</span>
    </div>
  );
}

function formatVerifiedTime(timestamp: string): string {
  const elapsedMs = Date.now() - Date.parse(timestamp);
  if (!Number.isFinite(elapsedMs) || elapsedMs < 0) return "Verified recently";
  const minutes = Math.floor(elapsedMs / 60_000);
  if (minutes < 1) return "Verified just now";
  if (minutes < 60) return `Verified ${minutes}m ago`;
  const hours = Math.floor(minutes / 60);
  if (hours < 24) return `Verified ${hours}h ago`;
  const days = Math.floor(hours / 24);
  return `Verified ${days}d ago`;
}
