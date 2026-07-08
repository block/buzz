import * as React from "react";
import type { LucideIcon } from "lucide-react";
import {
  Activity,
  Brain,
  ChevronDown,
  ChevronRight,
  Copy,
  Cpu,
  Hash,
  Layers,
  MessageSquare,
  PenOff,
  Server,
} from "lucide-react";
import { useAgentConfigSurface } from "../hooks";
import { cn } from "@/shared/lib/cn";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
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
  advancedMode?: "collapsed" | "flat";
};

function isReadOnlyField({
  origin,
  writeVia,
}: {
  origin: ConfigOrigin;
  writeVia: ConfigWriteMechanism;
}) {
  return writeVia.type === "readOnly" || origin === "harnessConstraint";
}

function ConfigFieldLabel({ label }: { label: string }) {
  return (
    <span className="inline-flex min-w-0 items-center gap-1.5 text-xs font-medium text-foreground">
      <span className="truncate">{label}</span>
    </span>
  );
}

function ProvenanceHint({
  locked,
  provenance,
}: {
  locked: boolean;
  provenance: string;
}) {
  return (
    <span className="mt-0.5 flex items-center gap-1 text-2xs text-muted-foreground/70">
      {locked ? (
        <PenOff aria-label="Read-only" className="h-3 w-3 shrink-0" />
      ) : null}
      <span className="min-w-0 truncate">{provenance}</span>
    </span>
  );
}

function shouldOfferCopy({
  fieldKey,
  origin,
  value,
}: {
  fieldKey?: keyof NormalizedConfig;
  origin: ConfigOrigin;
  value: string | null;
}) {
  if (!value) {
    return false;
  }

  if (
    fieldKey === "model" ||
    fieldKey === "provider" ||
    fieldKey === "maxOutputTokens" ||
    fieldKey === "contextLimit"
  ) {
    return true;
  }

  if (origin === "envVar") {
    return true;
  }

  // Heuristic for machine-y values worth copying: filesystem paths ("/" or
  // "~"), and URI-ish strings (scheme:rest). The colon rule requires the
  // value to be space-free so prose like "Extension: developer" doesn't
  // grow a surprising copy affordance.
  return (
    value.includes("/") ||
    value.startsWith("~") ||
    (value.includes(":") && !value.includes(" "))
  );
}

type RowVariant = "compact" | "profile";

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
      return "Inherited from template";
    case "runtimeOverride":
      return "Live override (this session only)";
    case "harnessConstraint":
      return "Locked by harness";
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

const NORMALIZED_ICONS: Record<keyof NormalizedConfig, LucideIcon> = {
  model: Cpu,
  provider: Server,
  mode: Activity,
  thinkingEffort: Brain,
  maxOutputTokens: Hash,
  contextLimit: Layers,
  systemPrompt: MessageSquare,
};

function NormalizedRow({
  fieldKey,
  label,
  field,
  isPreSpawn,
  configFilePath,
  variant = "compact",
}: {
  fieldKey: keyof NormalizedConfig;
  label: string;
  field: NormalizedField;
  isPreSpawn: boolean;
  configFilePath: string | null;
  variant?: RowVariant;
}) {
  const Icon = NORMALIZED_ICONS[fieldKey];
  // ACP-sourced origins only become meaningful post-spawn
  const isAcpOnly =
    field.origin === "acpNativeRead" || field.origin === "acpConfigOption";
  const displayValue =
    isPreSpawn && isAcpOnly
      ? "Available after agent starts"
      : (field.value ?? "—");
  const provenance = field.value
    ? provenanceSentence(field.origin, field.writeVia, configFilePath)
    : null;
  const locked = isReadOnlyField(field);
  const isCopyable =
    variant === "profile" &&
    shouldOfferCopy({
      fieldKey,
      origin: field.origin,
      value: field.value,
    });

  const content = (
    <>
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted/60">
        <Icon className="h-4 w-4 text-muted-foreground" />
      </span>
      <span className="min-w-0 flex-1 text-left">
        {variant === "profile" ? (
          <ConfigFieldLabel label={label} />
        ) : (
          <span className="block text-xs font-medium text-foreground">
            {label}
          </span>
        )}
        <span
          className="mt-0.5 block truncate text-sm text-muted-foreground"
          title={field.value ?? undefined}
        >
          {displayValue}
          {!(isPreSpawn && isAcpOnly) && field.overriddenValue ? (
            <span
              className={cn(
                "ml-2 text-xs text-muted-foreground/60",
                field.origin !== "runtimeOverride" && "line-through",
              )}
              title={field.overriddenValue ?? undefined}
            >
              {field.overriddenValue}
            </span>
          ) : null}
        </span>
        {provenance ? (
          <ProvenanceHint locked={locked} provenance={provenance} />
        ) : null}
      </span>
      {isCopyable ? (
        <Copy className="h-4 w-4 shrink-0 text-muted-foreground" />
      ) : null}
    </>
  );

  if (isCopyable && field.value) {
    return (
      <button
        aria-label={`Copy ${label}`}
        className="flex w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-muted/40"
        onClick={() =>
          copyTextToClipboard(field.value ?? "", `Copied ${label}`)
        }
        title={`Copy ${label}`}
        type="button"
      >
        {content}
      </button>
    );
  }

  return <div className="flex items-center gap-3 px-4 py-3">{content}</div>;
}

// ── Advanced row ──────────────────────────────────────────────────────────────

function AdvancedRow({
  field,
  configFilePath,
  variant = "compact",
}: {
  field: ConfigField;
  configFilePath: string | null;
  variant?: RowVariant;
}) {
  const provenance = field.value
    ? provenanceSentence(field.origin, field.writeVia, configFilePath)
    : null;
  const locked = isReadOnlyField(field);

  if (variant === "compact") {
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
        {provenance ? (
          <div className="mt-0.5 text-2xs text-muted-foreground/70">
            {provenance}
          </div>
        ) : null}
      </div>
    );
  }

  const isCopyable = shouldOfferCopy({
    origin: field.origin,
    value: field.value,
  });
  const content = (
    <>
      <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted/60">
        <Hash className="h-4 w-4 text-muted-foreground" />
      </span>
      <span className="min-w-0 flex-1 text-left">
        <ConfigFieldLabel label={field.label} />
        <span
          className="mt-0.5 block truncate text-sm text-muted-foreground"
          title={field.value ?? undefined}
        >
          {field.value ?? "—"}
        </span>
        {provenance ? (
          <ProvenanceHint locked={locked} provenance={provenance} />
        ) : null}
      </span>
      {isCopyable ? (
        <Copy className="h-4 w-4 shrink-0 text-muted-foreground" />
      ) : null}
    </>
  );

  if (isCopyable && field.value) {
    return (
      <button
        aria-label={`Copy ${field.label}`}
        className="flex w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-muted/40"
        onClick={() =>
          copyTextToClipboard(field.value ?? "", `Copied ${field.label}`)
        }
        title={`Copy ${field.label}`}
        type="button"
      >
        {content}
      </button>
    );
  }

  return <div className="flex items-center gap-3 px-4 py-3">{content}</div>;
}

// ── Main component ────────────────────────────────────────────────────────────

export function AgentConfigPanel({
  advancedMode = "collapsed",
  pubkey,
}: Props) {
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
  ).filter(([key, field]) => {
    if (field === null) {
      return false;
    }
    // Flat (profile) mode renders the record/persona system prompt in the
    // dedicated Instructions block above, so drop it here to avoid the
    // duplicate — but keep a config-file-sourced prompt, which has no other
    // home in the profile panel.
    if (
      advancedMode === "flat" &&
      key === "systemPrompt" &&
      field.origin !== "configFile"
    ) {
      return false;
    }
    return true;
  }) as [keyof NormalizedConfig, NormalizedField][];

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
              fieldKey={key}
              label={NORMALIZED_LABELS[key]}
              field={field}
              isPreSpawn={isPreSpawn}
              configFilePath={configFilePath}
              variant={advancedMode === "flat" ? "profile" : "compact"}
            />
          ))
        )}
      </div>

      {advanced.length > 0 && advancedMode === "flat" ? (
        <div className="divide-y divide-border/50 border-t border-border/50">
          {advanced.map((field) => (
            <AdvancedRow
              key={field.key}
              field={field}
              configFilePath={configFilePath}
              variant="profile"
            />
          ))}
        </div>
      ) : null}

      {advanced.length > 0 && advancedMode === "collapsed" ? (
        <div className="mt-3 border-t border-border/50 pt-2">
          <button
            className="flex items-center gap-1 text-xs text-muted-foreground hover:text-foreground"
            onClick={() => setAdvancedOpen((v) => !v)}
            type="button"
          >
            {advancedOpen ? (
              <ChevronDown className="h-3 w-3" />
            ) : (
              <ChevronRight className="h-3 w-3" />
            )}
            Advanced ({advanced.length})
          </button>

          {advancedOpen ? (
            <div className="mt-1 divide-y divide-border/50">
              {advanced.map((field) => (
                <AdvancedRow
                  key={field.key}
                  field={field}
                  configFilePath={configFilePath}
                />
              ))}
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
