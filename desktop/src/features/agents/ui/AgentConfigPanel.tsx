import * as React from "react";
import type { LucideIcon } from "lucide-react";
import {
  Activity,
  Brain,
  ChevronDown,
  ChevronRight,
  Cpu,
  Hash,
  Layers,
  MessageSquare,
  Pencil,
  Server,
} from "lucide-react";
import { useAgentConfigSurface } from "../hooks";
import { cn } from "@/shared/lib/cn";
import { Spinner } from "@/shared/ui/spinner";
import { PanelSectionGroup } from "@/shared/ui/PanelSectionGroup";
import {
  HoverCopyIndicator,
  useCopyFeedback,
} from "@/shared/ui/HoverCopyIndicator";
import { McpServersSection, shouldRenderMcpServers } from "./McpServersSection";
import type {
  ConfigField,
  ConfigOrigin,
  NormalizedConfig,
  NormalizedField,
  RuntimeConfigSurface,
} from "@/shared/api/types";
import { providerDisplayLabel } from "./agentConfigOptions";

export type AgentConfigPanelSection = "model" | "mcp" | "advanced";

const ALL_AGENT_CONFIG_SECTIONS: readonly AgentConfigPanelSection[] = [
  "model",
  "mcp",
  "advanced",
];

type Props = {
  pubkey: string;
  advancedMode?: "collapsed" | "flat";
  onEdit?: () => void;
  sections?: readonly AgentConfigPanelSection[];
};

type AgentConfigSurfaceRowsProps = {
  advancedMode?: "collapsed" | "flat";
  data: RuntimeConfigSurface;
  onEdit?: () => void;
  sections?: readonly AgentConfigPanelSection[];
};

function ConfigFieldLabel({ label }: { label: string }) {
  return (
    <span className="inline-flex min-w-0 items-center gap-1.5 text-sm font-medium text-foreground">
      <span className="truncate">{label}</span>
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

// ── Normalized row ────────────────────────────────────────────────────────────

const NORMALIZED_LABELS: Record<keyof NormalizedConfig, string> = {
  model: "Model",
  provider: "Provider",
  mode: "Mode",
  thinkingEffort: "Thinking effort",
  maxOutputTokens: "Max output tokens",
  contextLimit: "Context limit",
  systemPrompt: "System prompt",
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
  onEdit,
  variant = "compact",
}: {
  fieldKey: keyof NormalizedConfig;
  label: string;
  field: NormalizedField;
  onEdit?: () => void;
  variant?: RowVariant;
}) {
  const Icon = NORMALIZED_ICONS[fieldKey];
  const rawDisplayValue = field.value ?? "—";
  const displayValue =
    fieldKey === "provider"
      ? providerDisplayLabel(rawDisplayValue)
      : rawDisplayValue;
  const isEditable = variant === "profile" && onEdit !== undefined;

  const content = (
    <>
      <Icon
        className="h-4 w-4 shrink-0 text-muted-foreground"
        data-slot="agent-config-field-icon"
      />
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
        </span>
      </span>
      {isEditable ? (
        <Pencil
          className="h-4 w-4 shrink-0 text-muted-foreground opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100"
          data-testid={`agent-config-${fieldKey}-edit-indicator`}
        />
      ) : null}
    </>
  );

  if (isEditable) {
    return (
      <button
        aria-label={`Edit ${label}`}
        className="group flex min-h-16 w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-muted/40 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        onClick={onEdit}
        title={`Edit ${label}`}
        type="button"
      >
        {content}
      </button>
    );
  }

  return (
    <div
      className={cn(
        "flex items-center gap-3 px-4 py-3",
        variant === "profile" && "min-h-16",
      )}
    >
      {content}
    </div>
  );
}

// ── Advanced row ──────────────────────────────────────────────────────────────

function AdvancedRow({
  field,
  variant = "compact",
}: {
  field: ConfigField;
  variant?: RowVariant;
}) {
  const { copied, copy } = useCopyFeedback({
    label: field.label,
    value: field.value ?? "",
  });

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
      </div>
    );
  }

  const isCopyable = shouldOfferCopy({
    origin: field.origin,
    value: field.value,
  });
  const content = (
    <>
      <span className="min-w-0 flex-1 text-left">
        <ConfigFieldLabel label={field.label} />
        <span
          className="mt-0.5 block truncate text-sm text-muted-foreground"
          title={field.value ?? undefined}
        >
          {field.value ?? "—"}
        </span>
      </span>
      {isCopyable ? (
        <HoverCopyIndicator
          copied={copied}
          testId={`agent-config-advanced-${field.key}-copy-status`}
        />
      ) : null}
    </>
  );

  if (isCopyable && field.value) {
    return (
      <button
        aria-label={`Copy ${field.label}`}
        className="group flex min-h-16 w-full items-center gap-3 px-4 py-3 text-left transition-colors hover:bg-muted/40 focus-visible:outline-hidden focus-visible:ring-2 focus-visible:ring-inset focus-visible:ring-ring"
        onClick={() => void copy()}
        title={`Copy ${field.label}`}
        type="button"
      >
        {content}
      </button>
    );
  }

  return (
    <div className="flex min-h-16 items-center gap-3 px-4 py-3">{content}</div>
  );
}

// ── Main component ────────────────────────────────────────────────────────────

function ProfileConfigSection({
  children,
  testId,
  title,
}: {
  children: React.ReactNode;
  testId?: string;
  title: string;
}) {
  return (
    <PanelSectionGroup testId={testId} title={title}>
      {children}
    </PanelSectionGroup>
  );
}

export function AgentConfigPanel({
  advancedMode = "collapsed",
  onEdit,
  pubkey,
  sections = ALL_AGENT_CONFIG_SECTIONS,
}: Props) {
  const { data, isLoading, error } = useAgentConfigSurface(pubkey);
  const flatStateTitle = sections.includes("model")
    ? "Model settings"
    : sections.includes("mcp")
      ? "MCP servers"
      : "Advanced";

  if (isLoading) {
    const loading = (
      <div className="flex items-center gap-2 px-4 py-4 text-sm text-muted-foreground">
        <Spinner className="h-3.5 w-3.5" />
        Loading configuration…
      </div>
    );
    if (advancedMode === "flat") {
      return (
        <ProfileConfigSection title={flatStateTitle}>
          {loading}
        </ProfileConfigSection>
      );
    }
    return loading;
  }

  if (error || !data) {
    const errorMessage = (
      <p className="px-4 py-3 text-sm text-destructive">
        {error instanceof Error
          ? error.message
          : "Couldn't load agent configuration."}
      </p>
    );
    if (advancedMode === "flat") {
      return (
        <ProfileConfigSection title={flatStateTitle}>
          {errorMessage}
        </ProfileConfigSection>
      );
    }
    return errorMessage;
  }

  return (
    <AgentConfigSurfaceRows
      advancedMode={advancedMode}
      data={data}
      onEdit={onEdit}
      sections={sections}
    />
  );
}

export function AgentConfigSurfaceRows({
  advancedMode = "collapsed",
  data,
  onEdit,
  sections = ALL_AGENT_CONFIG_SECTIONS,
}: AgentConfigSurfaceRowsProps) {
  const [advancedOpen, setAdvancedOpen] = React.useState(false);

  const { normalized, advanced, extensions, runtimeId } = data;

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
  const showMcpServers = shouldRenderMcpServers(runtimeId, extensions);
  const showModelSection = sections.includes("model");
  const showMcpSection = sections.includes("mcp");
  const showAdvancedSection = sections.includes("advanced");

  if (advancedMode === "flat") {
    return (
      <div className="space-y-4">
        {showModelSection && normalizedEntries.length > 0 ? (
          <ProfileConfigSection
            testId="user-profile-model-settings-section"
            title="Model settings"
          >
            <div className="divide-y divide-border/55">
              {normalizedEntries.map(([key, field]) => (
                <NormalizedRow
                  field={field}
                  fieldKey={key}
                  key={key}
                  label={NORMALIZED_LABELS[key]}
                  onEdit={onEdit}
                  variant="profile"
                />
              ))}
            </div>
          </ProfileConfigSection>
        ) : null}

        {showMcpSection && showMcpServers ? (
          <ProfileConfigSection
            testId="user-profile-mcp-servers-section"
            title="MCP servers"
          >
            <McpServersSection
              extensions={extensions}
              runtimeId={runtimeId}
              variant="profile"
            />
          </ProfileConfigSection>
        ) : null}

        {showAdvancedSection && advanced.length > 0 ? (
          <ProfileConfigSection
            testId="user-profile-advanced-section"
            title="Advanced"
          >
            {advanced.map((field) => (
              <AdvancedRow field={field} key={field.key} variant="profile" />
            ))}
          </ProfileConfigSection>
        ) : null}
      </div>
    );
  }

  return (
    <div className="space-y-0.5">
      {/* Normalized section */}
      <div className="divide-y divide-border/50">
        {normalizedEntries.length === 0 ? (
          <p className="py-2 text-xs text-muted-foreground">
            No config fields available.
          </p>
        ) : (
          normalizedEntries.map(([key, field]) => (
            <NormalizedRow
              fieldKey={key}
              label={NORMALIZED_LABELS[key]}
              field={field}
              key={key}
              variant="compact"
            />
          ))
        )}
      </div>

      <McpServersSection
        extensions={extensions}
        runtimeId={runtimeId}
        variant="compact"
      />

      {advanced.length > 0 ? (
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
                <AdvancedRow field={field} key={field.key} />
              ))}
            </div>
          ) : null}
        </div>
      ) : null}
    </div>
  );
}
