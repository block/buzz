import { TerminalSquare } from "lucide-react";

import { useAvailableAcpRuntimes } from "@/features/agents/hooks";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import type { SetupStepActions, SetupStepState } from "./types";

type SetupStepProps = {
  actions: SetupStepActions;
};

type SetupStepContentProps = {
  actions: SetupStepActions;
  state: SetupStepState;
};

function useSetupStepState(): SetupStepState {
  const providersQuery = useAvailableAcpRuntimes();
  const items = providersQuery.data ?? [];
  const isChecking = providersQuery.isLoading;
  const errorMessage =
    providersQuery.error instanceof Error ? providersQuery.error.message : null;

  return {
    runtimeProviders: {
      errorMessage,
      isChecking,
      items,
      showSetupLaterHint:
        errorMessage === null && !isChecking && items.length === 0,
    },
  };
}

function RuntimeProvidersSection({
  runtimeProviders,
}: {
  runtimeProviders: SetupStepState["runtimeProviders"];
}) {
  const { errorMessage, isChecking, items, showSetupLaterHint } =
    runtimeProviders;

  return (
    <div className="space-y-4 rounded-[28px] border border-border/70 bg-muted/20 p-5">
      <div className="space-y-1">
        <div className="flex items-center gap-2">
          <TerminalSquare className="h-4 w-4 text-primary" />
          <p className="text-sm font-medium">Detected runtimes</p>
        </div>
        <p className="text-sm text-muted-foreground">
          We only list runtimes the app can actually see on this machine.
        </p>
      </div>

      {items.length > 0 ? (
        <div className="grid gap-2">
          {items.map((provider) => (
            <div
              className="rounded-2xl border border-border/70 bg-background/85 px-4 py-3"
              data-testid={`onboarding-runtime-${provider.id}`}
              key={provider.id}
            >
              <div className="flex items-center justify-between gap-3">
                <p className="text-sm font-medium text-foreground">
                  {provider.label}
                </p>
                <Badge variant="outline">{provider.command}</Badge>
              </div>
            </div>
          ))}
        </div>
      ) : isChecking ? (
        <p className="text-sm text-muted-foreground">
          Looking for compatible runtimes...
        </p>
      ) : errorMessage ? null : (
        <p
          className="text-sm text-muted-foreground"
          data-testid="onboarding-acp-empty"
        >
          No compatible ACP runtimes detected yet.
        </p>
      )}

      {showSetupLaterHint ? (
        <p className="rounded-2xl border border-border/70 bg-background/85 px-4 py-3 text-sm text-muted-foreground">
          Nothing is installed yet. That&apos;s fine. You can finish setup now
          and come back later in Settings &gt; Doctor.
        </p>
      ) : null}

      {errorMessage ? (
        <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
          {errorMessage}
        </p>
      ) : null}
    </div>
  );
}

function SetupStepContent({ actions, state }: SetupStepContentProps) {
  const { runtimeProviders } = state;

  return (
    <div className="space-y-6" data-testid="onboarding-page-2">
      <div className="space-y-3">
        <Badge variant="info">First run</Badge>
        <div className="space-y-2">
          <h1 className="text-3xl font-semibold tracking-tight text-foreground">
            ACP runtimes
          </h1>
          <p className="max-w-xl text-sm leading-6 text-muted-foreground">
            ACP runtimes only matter when you want Sprout to launch local tools
            from this machine.
          </p>
        </div>
      </div>

      <RuntimeProvidersSection runtimeProviders={runtimeProviders} />

      <div className="flex flex-wrap items-center justify-end gap-2">
        <Button
          data-testid="onboarding-back"
          onClick={actions.back}
          type="button"
          variant="outline"
        >
          Back
        </Button>
        <Button
          data-testid="onboarding-finish"
          onClick={actions.complete}
          type="button"
        >
          Finish
        </Button>
      </div>
    </div>
  );
}

export function SetupStep({ actions }: SetupStepProps) {
  const state = useSetupStepState();

  return <SetupStepContent actions={actions} state={state} />;
}
