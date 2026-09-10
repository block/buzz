import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import type { HarnessConnectionOption } from "./HarnessConnectionStep";

function previewRuntime({
  available = false,
  id,
  installInstructionsUrl = "https://github.com/block/buzz",
  label,
  source = "preset",
}: {
  available?: boolean;
  id: string;
  installInstructionsUrl?: string;
  label: string;
  source?: AcpRuntimeCatalogEntry["source"];
}): AcpRuntimeCatalogEntry {
  return {
    authStatus: available
      ? { status: "not_applicable" }
      : { status: "unknown" },
    availability: available ? "available" : "not_installed",
    avatarUrl: "",
    binaryPath: available ? `/usr/local/bin/${id}` : null,
    canAutoInstall: true,
    command: available ? id : null,
    contextLimitEnvVar: null,
    defaultArgs: [],
    id,
    installHint: `Install ${label} to connect it to Buzz.`,
    installInstructionsUrl,
    label,
    loginHint: null,
    maxRoundsEnvVar: null,
    maxTokensEnvVar: null,
    mcpCommand: null,
    modelEnvVar: null,
    nodeRequired: false,
    providerEnvVar: null,
    requiresExternalCli: id !== "buzz-agent",
    source,
    thinkingEnvVar: null,
    underlyingCliPath: null,
  };
}

/** Fixed workshop catalog. Availability changes only in React memory. */
export const HARNESS_CONNECTION_OPTIONS: readonly HarnessConnectionOption[] = [
  {
    methods: ["subscription"],
    runtime: previewRuntime({
      available: true,
      id: "claude",
      installInstructionsUrl: "https://code.claude.com/docs/en/getting-started",
      label: "Claude Code",
      source: "builtin",
    }),
  },
  {
    methods: ["subscription"],
    runtime: previewRuntime({
      id: "codex",
      installInstructionsUrl: "https://developers.openai.com/codex/cli/",
      label: "Codex",
      source: "builtin",
    }),
  },
  {
    methods: ["api"],
    runtime: previewRuntime({
      available: true,
      id: "goose",
      installInstructionsUrl:
        "https://goose-docs.ai/docs/getting-started/installation/",
      label: "Goose",
      source: "builtin",
    }),
  },
  {
    methods: ["api"],
    runtime: previewRuntime({
      available: true,
      id: "buzz-agent",
      label: "Buzz Agent",
      source: "builtin",
    }),
  },
  {
    methods: ["subscription"],
    runtime: previewRuntime({
      id: "cursor",
      installInstructionsUrl: "https://cursor.com/downloads",
      label: "Cursor",
    }),
  },
  {
    methods: ["subscription"],
    runtime: previewRuntime({
      id: "devin",
      installInstructionsUrl: "https://docs.devin.ai/cli",
      label: "Devin",
    }),
  },
  {
    methods: ["api"],
    runtime: previewRuntime({
      id: "omp",
      installInstructionsUrl: "https://omp.sh/",
      label: "Oh My Pi",
    }),
  },
  {
    methods: ["api"],
    runtime: previewRuntime({
      id: "grok",
      installInstructionsUrl: "https://build.x.ai/docs",
      label: "Grok Build",
    }),
  },
  {
    methods: ["api"],
    runtime: previewRuntime({
      id: "opencode",
      installInstructionsUrl: "https://opencode.ai/docs",
      label: "OpenCode",
    }),
  },
  {
    methods: ["api"],
    runtime: previewRuntime({
      id: "kimi",
      installInstructionsUrl: "https://kimi.ai/download",
      label: "Kimi Code",
    }),
  },
  {
    methods: ["subscription"],
    runtime: previewRuntime({
      id: "amp",
      installInstructionsUrl: "https://github.com/tao12345666333/amp-acp",
      label: "Amp",
    }),
  },
  {
    methods: ["api"],
    runtime: previewRuntime({
      id: "hermes",
      installInstructionsUrl: "https://hermes-agent.nousresearch.com",
      label: "Hermes Agent",
    }),
  },
  {
    methods: ["api"],
    runtime: previewRuntime({
      id: "openclaw",
      installInstructionsUrl: "https://docs.openclaw.ai/start/getting-started",
      label: "OpenClaw",
    }),
  },
];
