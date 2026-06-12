import * as React from "react";
import { ChevronDown, ChevronRight, Info } from "lucide-react";

import { useAgentConfigSurface } from "../hooks";
import { cn } from "@/shared/lib/cn";
import { Spinner } from "@/shared/ui/spinner";
import type {
  ConfigField,
  ConfigOrigin,
  NormalizedConfig,
  NormalizedField,
  ConfigSourceReport,
} from "@/shared/api/types";

type Props = {
  pubkey: string;
  isRunning: boolean;
};

// ── Origin badge ─────────────────────────────────────────────────────────────

function originLabel(
  origin: ConfigOrigin,
  configFilePath: string | null,
): string {
  switch (origin) {
    case "buzzExplicit":
      return "Buzz";
    case "acpConfigOption":
      return "ACP";
    case "acpNativeRead":
      return "ACP";
    case "envVar":
      return "Env";
    case "configFile": {
      if (configFilePath) {
        const parts = configFilePath.split(/[/\\]/);
        return parts[parts.length - 1] ?? configFilePath;
      }
      return "Config";
    }
    case "personaDefault":
      return "Persona";
  }
}

function originColorClass(origin: ConfigOrigin): string {
  switch (origin) {
    case "buzzExplicit":
      return "bg-blue-100 text-blue-700 dark:bg-blue-900/40 dark:text-blue-300";
    case "acpConfigOption":
    case "acpNativeRead":
      return "bg-green-100 text-green-700 dark:bg-green-900/40 dark:text-green-300";
    case "configFile":
    case "personaDefault":
      return "bg-muted text-muted-foreground";
    case "envVar":
      return "bg-amber-100 text-amber-700 dark:bg-amber-900/40 dark:text-amber-300";
  }
}

function OriginBadge({
  origin,
  configFilePath,
}: {
  origin: ConfigOrigin;
  configFilePath: string | null;
}) {
  return (
    <span
      className={cn(
        "inline-block max-w-[120px] truncate rounded px-1.5 py-0.5 text-[10px] font-medium leading-none",
        originColorClass(origin),
      )}
      title={originLabel(origin, configFilePath)}
    >
      {originLabel(origin, configFilePath)}
    </span>
  );
}

// ── Normalized row ────────────────────────────────────────────────────────────

const NORMALIZED_LABELS: Record<keyof NormalizedConfig, string> = {
  model: "Model",
  provider: "Provider",
  mode: "Mode",
  thinkingEffort: "Thinking / Effort",
  maxOutputTokens: "Max Output Tokens",
  contextLimit: "Context Limit",
  systemPrompt: "System Prompt",
};

function NormalizedRow({
  label,
  field,
  isPreSpawn,
  configFilePath,
}: {
  label: string;
  field: NormalizedField;
  isPreSpawn: boolean;
  configFilePath: string | null;
}) {
  // ACP-sourced origins only become meaningful post-spawn
  const isAcpOnly =
    field.origin === "acpNativeRead" || field.origin === "acpConfigOption";

  return (
    <div className="flex items-center gap-2 py-1 text-sm">
      <span className="w-36 shrink-0 text-xs text-muted-foreground">
        {label}
      </span>

      {/* Value area: effective value + strikethrough overridden value */}
      <span className="min-w-0 flex-1 truncate font-medium">
        {isPreSpawn && isAcpOnly ? (
          <em className="font-normal text-muted-foreground not-italic text-xs">
            Available after agent starts
          </em>
        ) : isPreSpawn && field.origin === "configFile" && !field.value ? (
          <span className="text-xs text-muted-foreground/70">—</span>
        ) : (
          <>
            {field.value ?? <span className="text-muted-foreground">—</span>}
            {field.overriddenValue && (
              <span className="ml-2 text-xs text-muted-foreground/60 line-through">
                {field.overriddenValue}
              </span>
            )}
          </>
        )}
      </span>

      {/* Badge area: effective badge + strikethrough overridden badge */}
      <span className="flex items-center gap-1 shrink-0">
        <OriginBadge origin={field.origin} configFilePath={configFilePath} />
        {field.overriddenOrigin && (
          <span className="opacity-50 line-through">
            <OriginBadge
              origin={field.overriddenOrigin}
              configFilePath={configFilePath}
            />
          </span>
        )}
      </span>

      {!field.isWritable && (
        <span title="Read-only — edit this field directly in the config file">
          <Info className="h-3 w-3 shrink-0 text-muted-foreground/50" />
        </span>
      )}
    </div>
  );
}

// ── Advanced row ──────────────────────────────────────────────────────────────

function AdvancedRow({
  field,
  configFilePath,
}: {
  field: ConfigField;
  configFilePath: string | null;
}) {
  return (
    <div className="flex items-center gap-2 py-1 text-sm">
      <span className="w-36 shrink-0 text-xs text-muted-foreground">
        {field.label}
      </span>
      <span className="min-w-0 flex-1 truncate font-medium font-mono text-xs">
        {field.value ?? (
          <span className="not-italic text-muted-foreground">—</span>
        )}
      </span>
      <OriginBadge origin={field.origin} configFilePath={configFilePath} />
      {!field.isWritable && (
        <span title="Read-only — edit this field directly in the config file">
          <Info className="h-3 w-3 shrink-0 text-muted-foreground/50" />
        </span>
      )}
    </div>
  );
}

// ── Sources footer ────────────────────────────────────────────────────────────

const STATUS_ICON: Record<string, string> = {
  available: "✓",
  pending: "⏳",
  notApplicable: "—",
};

function SourcesFooter({ sources }: { sources: ConfigSourceReport }) {
  const tiers = [
    { label: "Config file", status: sources.configFile },
    { label: "ACP native", status: sources.acpNative },
    { label: "ACP config", status: sources.acpConfigOptions },
    { label: "Env vars", status: sources.envVars },
  ] as const;

  return (
    <p className="mt-3 text-[10px] text-muted-foreground/70">
      {tiers.map((tier, i) => (
        <React.Fragment key={tier.label}>
          {i > 0 && <span className="mx-1.5">|</span>}
          {tier.label} <span>{STATUS_ICON[tier.status] ?? tier.status}</span>
        </React.Fragment>
      ))}
    </p>
  );
}

// ── Main component ────────────────────────────────────────────────────────────

export function AgentConfigPanel({ pubkey, isRunning: _isRunning }: Props) {
  const [advancedOpen, setAdvancedOpen] = React.useState(false);

  const { data, isLoading, error } = useAgentConfigSurface(pubkey);

  if (isLoading) {
    return (
      <div className="flex items-center gap-2 py-4 text-sm text-muted-foreground">
        <Spinner className="h-3.5 w-3.5" />
        Loading config…
      </div>
    );
  }

  if (error || !data) {
    return (
      <p className="py-3 text-sm text-destructive">
        {error instanceof Error
          ? error.message
          : "Failed to load agent config."}
      </p>
    );
  }

  const { normalized, advanced, sources, isPreSpawn } = data;
  const configFilePath = sources.configFilePath;

  const normalizedEntries = (
    Object.entries(normalized) as [
      keyof NormalizedConfig,
      NormalizedField | null,
    ][]
  ).filter(([, field]) => field !== null) as [
    keyof NormalizedConfig,
    NormalizedField,
  ][];

  return (
    <div className="space-y-0.5">
      {/* Normalized section */}
      <div
        className={cn("divide-y divide-border/50", isPreSpawn && "opacity-60")}
      >
        {normalizedEntries.length === 0 ? (
          <p className="py-2 text-xs text-muted-foreground">
            No config fields available.
          </p>
        ) : (
          normalizedEntries.map(([key, field]) => (
            <NormalizedRow
              key={key}
              label={NORMALIZED_LABELS[key]}
              field={field}
              isPreSpawn={isPreSpawn}
              configFilePath={configFilePath}
            />
          ))
        )}
      </div>

      {/* Advanced section */}
      {advanced.length > 0 && (
        <div className="mt-3 border-t border-border/50 pt-2">
          <button
            type="button"
            className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
            onClick={() => setAdvancedOpen((v) => !v)}
          >
            {advancedOpen ? (
              <ChevronDown className="h-3 w-3" />
            ) : (
              <ChevronRight className="h-3 w-3" />
            )}
            Advanced ({advanced.length})
          </button>

          {advancedOpen && (
            <div className="mt-1 divide-y divide-border/50">
              {advanced.map((field) => (
                <AdvancedRow
                  key={field.key}
                  field={field}
                  configFilePath={configFilePath}
                />
              ))}
            </div>
          )}
        </div>
      )}

      <SourcesFooter sources={sources} />
    </div>
  );
}
