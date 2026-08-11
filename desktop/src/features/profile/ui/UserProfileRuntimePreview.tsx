import {
  AgentConfigSurfaceRows,
  type AgentConfigPanelSection,
} from "@/features/agents/ui/AgentConfigPanel";
import type { ProfileField } from "@/features/profile/ui/UserProfilePanelFields";
import { Badge } from "@/shared/ui/badge";
import type { NormalizedField, RuntimeConfigSurface } from "@/shared/api/types";

function previewField(value: string): NormalizedField {
  return {
    isRequired: false,
    origin: "harnessDefault",
    overriddenOrigin: null,
    overriddenValue: null,
    value,
    writeVia: { type: "readOnly" },
  };
}

const PROFILE_RUNTIME_PREVIEW: RuntimeConfigSurface = {
  advanced: [
    {
      key: "workingDirectory",
      label: "Working directory",
      origin: "buzzExplicit",
      schemaType: { type: "string" },
      value: "~/Development/buzz",
      writeVia: { type: "readOnly" },
    },
  ],
  extensions: [{ enabled: true, kind: "stdio", name: "Buzz developer tools" }],
  isPreSpawn: false,
  normalized: {
    contextLimit: previewField("200,000"),
    maxOutputTokens: previewField("8,192"),
    mode: previewField("Auto"),
    model: previewField("claude-sonnet-4-20250514"),
    provider: previewField("anthropic"),
    systemPrompt: null,
    thinkingEffort: previewField("High"),
  },
  runtimeId: "goose",
  runtimeLabel: "Goose",
  sources: {
    acpConfigOptions: "available",
    acpNative: "available",
    configFile: "available",
    configFilePath: "~/.config/goose/config.yaml",
    envVars: "notApplicable",
    mcpConfigFilePath: null,
  },
};

const PROFILE_RUNTIME_PREVIEW_FIELDS: ProfileField[] = [
  {
    copyValue: "goose",
    displayValue: "Goose",
    label: "Runtime",
    testId: "user-profile-runtime",
  },
  {
    copyValue: "goose acp",
    displayValue: "goose acp",
    label: "ACP command",
    testId: "user-profile-acp",
  },
  {
    copyValue: "goose mcp",
    displayValue: "goose mcp",
    label: "MCP command",
    testId: "user-profile-mcp",
  },
  {
    displayValue: "Yes",
    label: "Start on launch",
    testId: "user-profile-start-on-launch",
  },
  {
    displayValue: "Only the owner",
    label: "Who can send instructions",
    testId: "user-profile-respond-to",
  },
];

const PROFILE_RUNTIME_PREVIEW_DIAGNOSTICS: ProfileField[] = [
  {
    displayNode: (
      <Badge className="normal-case tracking-normal" variant="success">
        Running
      </Badge>
    ),
    displayValue: "Running",
    label: "Status",
    testId: "user-profile-agent-status",
  },
];

function appendMissingPreviewFields(
  fields: ProfileField[],
  previewFields: ProfileField[],
) {
  const existingLabels = new Set(fields.map((field) => field.label));
  return [
    ...fields,
    ...previewFields.filter((field) => !existingLabels.has(field.label)),
  ];
}

export function fillRuntimePreviewFields(fields: ProfileField[]) {
  return appendMissingPreviewFields(fields, PROFILE_RUNTIME_PREVIEW_FIELDS);
}

export function fillRuntimePreviewDiagnostics(fields: ProfileField[]) {
  return appendMissingPreviewFields(
    fields,
    PROFILE_RUNTIME_PREVIEW_DIAGNOSTICS,
  );
}

export function UserProfileRuntimePreviewNotice() {
  return (
    <div
      className="rounded-xl border border-dashed border-border/70 bg-muted/20 px-3 py-2.5"
      data-testid="user-profile-runtime-preview-notice"
    >
      <p className="text-sm font-medium text-foreground">
        Preview runtime data
      </p>
      <p className="mt-0.5 text-xs leading-5 text-muted-foreground">
        Staging doesn’t include every production runtime detail. Missing values
        below use examples.
      </p>
    </div>
  );
}

export function UserProfileConfigPreview({
  onEdit,
  sections,
}: {
  onEdit?: () => void;
  sections: readonly AgentConfigPanelSection[];
}) {
  return (
    <div data-testid="user-profile-runtime-preview">
      <AgentConfigSurfaceRows
        advancedMode="flat"
        data={PROFILE_RUNTIME_PREVIEW}
        onEdit={onEdit}
        sections={sections}
      />
    </div>
  );
}
