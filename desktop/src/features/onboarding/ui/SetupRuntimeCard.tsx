import { Check } from "lucide-react";
import type * as React from "react";

import { describeResolvedCommand } from "@/features/agents/ui/agentUi";
import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import { cn } from "@/shared/lib/cn";
import { Button } from "@/shared/ui/button";
import { Card } from "@/shared/ui/card";
import { Spinner } from "@/shared/ui/spinner";
import { Tooltip, TooltipContent, TooltipTrigger } from "@/shared/ui/tooltip";
import { runtimeIsReadyForOnboarding } from "./onboardingRuntimeSelection";
import { getRuntimeDisplayLabel, RuntimeIcon } from "./RuntimeIcon";

function RuntimeReadinessIndicator({
  runtime,
  ready,
}: {
  runtime: AcpRuntimeCatalogEntry;
  ready: boolean;
}) {
  // Checkmark temporarily hidden; flip to true to restore it.
  const showReadinessCheckmark = false;
  if (!ready || !showReadinessCheckmark) return null;

  return (
    <span
      aria-hidden="true"
      className="pointer-events-none absolute right-8 top-8 flex h-8 w-8 items-center justify-center rounded-full border border-[var(--buzz-welcome-chartreuse)] bg-[var(--buzz-welcome-chartreuse)]"
      data-testid={`onboarding-runtime-check-${runtime.id}`}
    >
      <Check
        className="h-4 w-4 text-foreground"
        data-testid={`onboarding-runtime-checkmark-${runtime.id}`}
        strokeWidth={3}
      />
    </span>
  );
}

function RuntimeDetails({ runtime }: { runtime: AcpRuntimeCatalogEntry }) {
  if (
    runtime.availability === "available" &&
    runtime.command &&
    runtime.binaryPath
  ) {
    const description = describeResolvedCommand(
      runtime.command,
      runtime.binaryPath,
    );
    return (
      <>
        <p className="text-xs leading-4 text-white">
          {description.charAt(0).toUpperCase() + description.slice(1)}
        </p>
        {runtime.defaultArgs.length > 0 ? (
          <p className="mt-1 text-xs leading-4 text-white">
            Args:{" "}
            <code className="font-mono">{runtime.defaultArgs.join(", ")}</code>
          </p>
        ) : null}
      </>
    );
  }

  if (runtime.availability === "adapter_missing") {
    return (
      <>
        <p className="text-xs leading-4 text-white">
          CLI detected; ACP adapter missing.
        </p>
        <p className="mt-1 text-xs leading-4 text-white">
          {runtime.installHint}
        </p>
      </>
    );
  }

  if (runtime.availability === "adapter_outdated") {
    return (
      <>
        <p className="text-xs leading-4 text-white">
          ACP adapter detected but outdated — reinstall required.
        </p>
        <p className="mt-1 text-xs leading-4 text-white">
          This updates the machine-global{" "}
          <code className="rounded bg-white/10 px-0.5 font-mono text-xs text-white">
            codex-acp
          </code>{" "}
          adapter. Older Buzz releases using the legacy adapter contract may
          lose community access until{" "}
          <code className="rounded bg-white/10 px-0.5 font-mono text-xs text-white">
            @zed-industries/codex-acp@0.16.0
          </code>{" "}
          is restored.
        </p>
        <p className="mt-1 text-xs leading-4 text-white">
          {runtime.installHint}
        </p>
      </>
    );
  }

  if (runtime.availability === "cli_missing") {
    return (
      <>
        <p className="text-xs leading-4 text-white">
          ACP adapter detected; CLI missing.
        </p>
        <p className="mt-1 text-xs leading-4 text-white">
          {runtime.installHint}
        </p>
      </>
    );
  }

  return (
    <>
      <p className="text-xs leading-4 text-white">Not installed yet.</p>
      <p className="mt-1 text-xs leading-4 text-white">{runtime.installHint}</p>
    </>
  );
}

function runtimeDetailText(runtime: AcpRuntimeCatalogEntry): string {
  if (
    runtime.availability === "available" &&
    runtime.command &&
    runtime.binaryPath
  ) {
    const description = describeResolvedCommand(
      runtime.command,
      runtime.binaryPath,
    );
    return description.charAt(0).toUpperCase() + description.slice(1);
  }
  if (runtime.availability === "adapter_missing") {
    return "CLI detected; ACP adapter missing.";
  }
  if (runtime.availability === "adapter_outdated") {
    return "ACP adapter detected but outdated — reinstall required.";
  }
  if (
    runtime.availability === "cli_missing" ||
    runtime.availability === "not_installed"
  ) {
    return "CLI not detected.";
  }
  return "";
}

export function SetupRuntimeAction({
  ariaLabel,
  children,
  disabled,
  onClick,
  testId,
}: {
  ariaLabel: string;
  children: React.ReactNode;
  disabled?: boolean;
  onClick: () => void;
  testId?: string;
}) {
  return (
    <Button
      aria-label={ariaLabel}
      className="buzz-onboarding-runtime-setup h-5 rounded-full bg-[var(--buzz-welcome-chartreuse)]/30 px-2.5 font-mono !text-badge font-normal uppercase text-foreground hover:bg-[var(--buzz-welcome-chartreuse)]/40"
      data-testid={testId}
      disabled={disabled}
      onClick={onClick}
      type="button"
      variant="ghost"
    >
      {children}
    </Button>
  );
}

export function SetupRuntimeInstallingStatus({
  runtime,
}: {
  runtime: AcpRuntimeCatalogEntry;
}) {
  return (
    <div
      aria-label={`Installing ${runtime.label}`}
      className="flex h-5 items-center gap-2 rounded-full bg-white/60 px-2.5 font-mono text-badge font-normal uppercase text-foreground"
      role="status"
    >
      <Spinner className="h-3 w-3 border-2 text-foreground" />
      INSTALLING
    </div>
  );
}

export function SetupRuntimeCheckingStatus({
  runtime,
}: {
  runtime: AcpRuntimeCatalogEntry;
}) {
  return (
    <div
      aria-label={`Rechecking ${runtime.label}`}
      className="flex h-5 items-center gap-2 rounded-full bg-[#EBEFEF] px-2.5 font-mono text-badge font-normal uppercase text-foreground"
      data-testid={`onboarding-runtime-rechecking-${runtime.id}`}
      role="status"
    >
      <Spinner className="h-3 w-3 border-2 text-foreground" />
      CHECKING…
    </div>
  );
}

export function SetupRuntimeReadyStatus({
  runtime,
}: {
  runtime: AcpRuntimeCatalogEntry;
}) {
  return (
    <Tooltip>
      <TooltipTrigger asChild>
        <span
          className="inline-flex h-5 cursor-default items-center rounded-full bg-[#EBEFEF] px-2.5 font-mono text-badge font-normal uppercase text-foreground"
          data-testid={`onboarding-runtime-ready-${runtime.id}`}
        >
          READY
        </span>
      </TooltipTrigger>
      <TooltipContent
        className="max-w-80 bg-black text-left text-xs text-white shadow-sm"
        side="top"
      >
        <RuntimeDetails runtime={runtime} />
      </TooltipContent>
    </Tooltip>
  );
}

export function SetupRuntimeCardPresentation({
  footer,
  installError,
  installOutputLine,
  isInstalling,
  runtime,
  status,
}: {
  footer?: React.ReactNode;
  installError: string | null;
  installOutputLine?: string | null;
  isInstalling: boolean;
  runtime: AcpRuntimeCatalogEntry;
  status: React.ReactNode;
}) {
  const isAvailable = runtime.availability === "available";
  const isReady = runtimeIsReadyForOnboarding(runtime);

  return (
    <Card
      className={cn(
        "group h-[224px] w-full max-w-[288px] select-none items-center px-3 py-1.5 text-center",
        installError && "ring-1 ring-destructive/40",
        isReady && "brightness-[0.98]",
      )}
      data-ready={isReady ? "true" : "false"}
      data-testid={`onboarding-runtime-${runtime.id}`}
      variant="textured"
    >
      <RuntimeReadinessIndicator ready={isReady} runtime={runtime} />

      <div className="flex min-w-0 flex-col items-center gap-2.5">
        <div className="flex min-w-0 items-center justify-center gap-3">
          <RuntimeIcon className="h-7 w-7" runtime={runtime} />
          <h2 className="truncate text-sm font-normal leading-5 text-foreground">
            {getRuntimeDisplayLabel(runtime)}
          </h2>
        </div>
        {status}
        {isInstalling && installOutputLine ? (
          <p
            aria-live="polite"
            className="max-w-[13rem] truncate font-mono text-2xs leading-4 text-muted-foreground"
            data-testid={`onboarding-runtime-install-output-${runtime.id}`}
          >
            {installOutputLine}
          </p>
        ) : !isAvailable && runtimeDetailText(runtime) ? (
          <p
            aria-hidden={installError ? "true" : undefined}
            className={cn(
              "max-w-[13rem] text-2xs leading-4 text-muted-foreground",
              installError && "invisible",
            )}
          >
            {runtimeDetailText(runtime)}
          </p>
        ) : null}
      </div>
      {footer}
    </Card>
  );
}
