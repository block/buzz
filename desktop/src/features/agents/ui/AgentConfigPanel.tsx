import * as React from "react";
import { ChevronDown, ChevronRight } from "lucide-react";

import { useAgentConfigSurface } from "../hooks";
import { cn } from "@/shared/lib/cn";
import { Spinner } from "@/shared/ui/spinner";
import type {
  ConfigField,
  ConfigOrigin,
  ConfigWriteMechanism,
  NormalizedConfig,
  NormalizedField,
} from "@/shared/api/types";

type Props = {
  pubkey: string;
  isRunning: boolean;
};

// ── Provenance sentence ──────────────────────────────────────────────────────

function provenanceSentence(
  origin: ConfigOrigin,
  writeVia: ConfigWriteMechanism,
  configFilePath: string | null,
): string {
  switch (origin) {
    case "buzzExplicit":
      return "Set in Buzz";
    case "personaDefault":
      return "Inherited from persona";
    case "runtimeOverride":
      return "Live override (this session only)";
    case "envVar": {
      if (writeVia.type === "respawnWithEnvVar") {
        return `From environment variable (${writeVia.envKey})`;
      }
      return "From environment variable";
    }
    case "configFile":
      return configFilePath
        ? `From config file (${configFilePath})`
        : "From config file";
    case "acpConfigOption":
    case "acpNativeRead":
      return "From ACP session";
  }
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
    <div className="py-2">
      <div className="text-xs text-muted-foreground">{label}</div>
      <div
        className="mt-0.5 truncate text-sm font-medium"
        title={field.value ?? undefined}
      >
        {isPreSpawn && isAcpOnly ? (
          <span className="font-normal text-muted-foreground text-xs">
            Available after agent starts
          </span>
        ) : (
          <>
            {field.value ?? <span className="text-muted-foreground">—</span>}
            {field.overriddenValue && (
              <span
                className={cn(
                  "ml-2 text-xs text-muted-foreground/60",
                  // A runtime override rides over the persona baseline rather
                  // than superseding an explicit pick — show the persona as a
                  // secondary value, not struck through.
                  field.origin !== "runtimeOverride" && "line-through",
                )}
                title={field.overriddenValue ?? undefined}
              >
                {field.overriddenValue}
              </span>
            )}
          </>
        )}
      </div>
      {field.value && (
        <div className="mt-0.5 text-2xs text-muted-foreground/70">
          {provenanceSentence(field.origin, field.writeVia, configFilePath)}
        </div>
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
    <div className="py-2">
      <div className="text-xs text-muted-foreground">{field.label}</div>
      <div
        className="mt-0.5 truncate text-sm font-medium font-mono"
        title={field.value ?? undefined}
      >
        {field.value ?? (
          <span className="font-sans text-muted-foreground">—</span>
        )}
      </div>
      {field.value && (
        <div className="mt-0.5 text-2xs text-muted-foreground/70">
          {provenanceSentence(field.origin, field.writeVia, configFilePath)}
        </div>
      )}
    </div>
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
    </div>
  );
}
