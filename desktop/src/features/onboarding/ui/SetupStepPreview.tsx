import * as React from "react";

import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { TooltipProvider } from "@/shared/ui/tooltip";
import {
  SetupRuntimeAction,
  SetupRuntimeCardPresentation,
  SetupRuntimeInstallingStatus,
  SetupRuntimeReadyStatus,
} from "./SetupRuntimeCard";
import { SetupStepContent } from "./SetupStep";

const PREVIEW_RUNTIMES: AcpRuntimeCatalogEntry[] = [
  {
    id: "claude",
    label: "Claude Code",
    avatarUrl: "",
    availability: "available",
    command: "claude-agent-acp",
    binaryPath: "/usr/local/bin/claude-agent-acp",
    defaultArgs: [],
    mcpCommand: null,
    modelEnvVar: null,
    providerEnvVar: null,
    thinkingEnvVar: null,
    maxTokensEnvVar: null,
    contextLimitEnvVar: null,
    maxRoundsEnvVar: null,
    installHint: "Install the Claude Code ACP adapter via npm.",
    installInstructionsUrl:
      "https://www.npmjs.com/package/@anthropic-ai/claude-agent-acp",
    canAutoInstall: true,
    requiresExternalCli: true,
    underlyingCliPath: "/usr/local/bin/claude",
    nodeRequired: false,
    authStatus: { status: "logged_out" },
    loginHint: "Sign in with your Claude subscription.",
    source: "builtin",
  },
  {
    id: "codex",
    label: "Codex",
    avatarUrl: "",
    availability: "adapter_missing",
    command: null,
    binaryPath: null,
    defaultArgs: [],
    mcpCommand: null,
    modelEnvVar: null,
    providerEnvVar: null,
    thinkingEnvVar: null,
    maxTokensEnvVar: null,
    contextLimitEnvVar: null,
    maxRoundsEnvVar: null,
    installHint: "Install the Codex ACP adapter.",
    installInstructionsUrl: "https://github.com/openai/codex",
    canAutoInstall: true,
    requiresExternalCli: true,
    underlyingCliPath: "/usr/local/bin/codex",
    nodeRequired: false,
    authStatus: { status: "unknown" },
    loginHint: "Sign in with your ChatGPT subscription.",
    source: "builtin",
  },
  {
    id: "goose",
    label: "Goose",
    avatarUrl: "",
    availability: "available",
    command: "goose",
    binaryPath: "/usr/local/bin/goose",
    defaultArgs: ["acp"],
    mcpCommand: null,
    modelEnvVar: null,
    providerEnvVar: null,
    thinkingEnvVar: null,
    maxTokensEnvVar: null,
    contextLimitEnvVar: null,
    maxRoundsEnvVar: null,
    installHint: "Install Goose via the official install script.",
    installInstructionsUrl: "https://block.github.io/goose/",
    canAutoInstall: true,
    requiresExternalCli: true,
    underlyingCliPath: null,
    nodeRequired: false,
    authStatus: { status: "not_applicable" },
    loginHint: null,
    source: "builtin",
  },
  {
    id: "buzz-agent",
    label: "Buzz Agent",
    avatarUrl: "",
    availability: "available",
    command: "buzz-agent",
    binaryPath: "/Applications/Buzz.app/Contents/MacOS/buzz-agent",
    defaultArgs: [],
    mcpCommand: "buzz-dev-mcp",
    modelEnvVar: null,
    providerEnvVar: null,
    thinkingEnvVar: null,
    maxTokensEnvVar: null,
    contextLimitEnvVar: null,
    maxRoundsEnvVar: null,
    installHint: "Ships with the Buzz desktop app.",
    installInstructionsUrl: "https://github.com/block/buzz",
    canAutoInstall: false,
    requiresExternalCli: false,
    underlyingCliPath: null,
    nodeRequired: false,
    authStatus: { status: "not_applicable" },
    loginHint: null,
    source: "builtin",
  },
];

type PreviewActivity = "checking" | "idle" | "installing";

function PreviewRuntimeCard({
  onRuntimeChange,
  runtime,
}: {
  onRuntimeChange: (
    runtimeId: string,
    update: (runtime: AcpRuntimeCatalogEntry) => AcpRuntimeCatalogEntry,
  ) => void;
  runtime: AcpRuntimeCatalogEntry;
}) {
  const [activity, setActivity] = React.useState<PreviewActivity>("idle");
  const timeoutRef = React.useRef<number | null>(null);

  React.useEffect(
    () => () => {
      if (timeoutRef.current !== null) {
        window.clearTimeout(timeoutRef.current);
      }
    },
    [],
  );

  function finishAfter(
    activityValue: Exclude<PreviewActivity, "idle">,
    update: (current: AcpRuntimeCatalogEntry) => AcpRuntimeCatalogEntry,
  ) {
    if (timeoutRef.current !== null) {
      window.clearTimeout(timeoutRef.current);
    }
    setActivity(activityValue);
    timeoutRef.current = window.setTimeout(() => {
      onRuntimeChange(runtime.id, update);
      setActivity("idle");
      timeoutRef.current = null;
    }, 700);
  }

  let status: React.ReactNode;
  if (activity === "installing") {
    status = <SetupRuntimeInstallingStatus runtime={runtime} />;
  } else if (activity === "checking") {
    status = (
      <SetupRuntimeAction
        ariaLabel={`Sign in to ${runtime.label}`}
        disabled
        onClick={() => undefined}
        testId={`onboarding-runtime-instructions-${runtime.id}`}
      >
        CHECKING…
      </SetupRuntimeAction>
    );
  } else if (
    runtime.availability === "available" &&
    (runtime.authStatus.status === "logged_in" ||
      runtime.authStatus.status === "not_applicable")
  ) {
    status = <SetupRuntimeReadyStatus runtime={runtime} />;
  } else if (
    runtime.availability === "available" &&
    runtime.authStatus.status === "logged_out"
  ) {
    status = (
      <SetupRuntimeAction
        ariaLabel={`Sign in to ${runtime.label}`}
        onClick={() =>
          finishAfter("checking", (current) => ({
            ...current,
            authStatus: { status: "logged_in" },
          }))
        }
        testId={`onboarding-runtime-instructions-${runtime.id}`}
      >
        SIGN IN
      </SetupRuntimeAction>
    );
  } else {
    status = (
      <SetupRuntimeAction
        ariaLabel={`Install ${runtime.label}`}
        onClick={() =>
          finishAfter("installing", (current) => ({
            ...current,
            availability: "available",
            authStatus: { status: "logged_out" },
            binaryPath: `/usr/local/bin/${current.id}-acp`,
            command: `${current.id}-acp`,
          }))
        }
        testId={`onboarding-runtime-install-${runtime.id}`}
      >
        INSTALL
      </SetupRuntimeAction>
    );
  }

  return (
    <SetupRuntimeCardPresentation
      installError={null}
      isInstalling={activity === "installing"}
      runtime={runtime}
      status={status}
    />
  );
}

export function SetupStepPreview({
  onBack,
  onNext,
}: {
  onBack: () => void;
  onNext: () => void;
}) {
  const [runtimes, setRuntimes] = React.useState(PREVIEW_RUNTIMES);
  const updateRuntime = React.useCallback(
    (
      runtimeId: string,
      update: (runtime: AcpRuntimeCatalogEntry) => AcpRuntimeCatalogEntry,
    ) => {
      setRuntimes((current) =>
        current.map((runtime) =>
          runtime.id === runtimeId ? update(runtime) : runtime,
        ),
      );
    },
    [],
  );

  return (
    <TooltipProvider>
      <SetupStepContent
        actions={{
          back: onBack,
          next: onNext,
          navigateToAgentSettings: onNext,
        }}
        direction="forward"
        onReadyRuntimeIdsChange={() => undefined}
        renderRuntimeCard={(runtime) => (
          <PreviewRuntimeCard
            onRuntimeChange={updateRuntime}
            runtime={runtime}
          />
        )}
        state={{
          runtimeProviders: {
            errorMessage: null,
            isChecking: false,
            items: runtimes,
          },
        }}
      />
    </TooltipProvider>
  );
}
