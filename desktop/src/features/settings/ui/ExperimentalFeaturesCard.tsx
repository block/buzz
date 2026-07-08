import * as React from "react";
import { toast } from "sonner";

import {
  useStartManagedAgentMutation,
  useStopManagedAgentMutation,
  useManagedAgentsQuery,
} from "@/features/agents/hooks";
import {
  applyExperimentAndRestartAgents,
  describeRestartOutcome,
  experimentRequiresAgentRestart,
  selectAgentsToRestart,
} from "@/features/settings/lib/experimentAgentRestart";
import { setDesktopExperiments } from "@/shared/api/tauri";
import {
  desktopFeatures,
  getOverrides,
  useFeatureToggle,
} from "@/shared/features";
import type { FeatureDefinition } from "@/shared/features";
import {
  AlertDialog,
  AlertDialogAction,
  AlertDialogCancel,
  AlertDialogContent,
  AlertDialogDescription,
  AlertDialogFooter,
  AlertDialogHeader,
  AlertDialogTitle,
} from "@/shared/ui/alert-dialog";
import { Button } from "@/shared/ui/button";
import { Switch } from "@/shared/ui/switch";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

function FeatureRow({ feature }: { feature: FeatureDefinition }) {
  const [enabled, toggle] = useFeatureToggle(feature.id);
  const [pendingValue, setPendingValue] = React.useState<boolean | null>(null);
  const requiresRestart = experimentRequiresAgentRestart(feature.id);
  // Only fetch agent state for rows that can trigger a restart.
  const agentsQuery = useManagedAgentsQuery({ enabled: requiresRestart });
  const startMutation = useStartManagedAgentMutation();
  const stopMutation = useStopManagedAgentMutation();
  const switchId = `feature-toggle-${feature.id}`;

  const runningAgents = selectAgentsToRestart(agentsQuery.data ?? []);

  const handleToggle = (value: boolean) => {
    // Spawn-env experiments confirm first: the toggle is applied only on
    // Confirm, then agents running at confirmation time are restarted so
    // their spawn-time env picks up the change. No running agents → nothing
    // to restart, apply directly.
    if (requiresRestart && runningAgents.length > 0) {
      setPendingValue(value);
      return;
    }
    toggle(value);
  };

  const handleConfirm = () => {
    if (pendingValue === null) {
      return;
    }
    const value = pendingValue;
    setPendingValue(null);
    // Snapshot at confirmation time: stopped agents stay stopped.
    const agentsToRestart = runningAgents;
    // Ordering matters: apply the toggle, await the explicit mirror write to
    // the Rust side, THEN restart — so respawned agents read the NEW env.
    // (The passive useDesktopExperimentsMirror effect also fires, but it is
    // unordered relative to the respawn; see applyExperimentAndRestartAgents.)
    void applyExperimentAndRestartAgents({
      applyToggle: () => toggle(value),
      mirrorExperiments: () => setDesktopExperiments(getOverrides()),
      agents: agentsToRestart,
      startAgent: (pubkey) => startMutation.mutateAsync(pubkey),
      stopAgent: (pubkey) => stopMutation.mutateAsync(pubkey),
    }).then(
      (outcome) => {
        const { kind, message } = describeRestartOutcome(outcome);
        if (kind === "success") {
          toast.success(message);
        } else {
          toast.error(message);
        }
      },
      (error) => {
        // Mirror write failed: the toggle stayed applied (best-effort mirror
        // convention), but agents were NOT restarted — tell the user.
        toast.error(
          `Setting applied, but syncing it to the agent runtime failed (${
            error instanceof Error ? error.message : String(error)
          }). Agents were not restarted — restart them manually to pick it up.`,
        );
      },
    );
  };

  const agentCountLabel =
    runningAgents.length === 1
      ? "1 running agent"
      : `${runningAgents.length} running agents`;

  return (
    <div className="flex items-center justify-between gap-3 rounded-lg border border-border/70 bg-background/70 px-4 py-3">
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium" id={`${switchId}-label`}>
          {feature.name}
        </p>
        <p className="text-xs text-muted-foreground">{feature.description}</p>
      </div>
      <Switch
        aria-labelledby={`${switchId}-label`}
        checked={enabled}
        data-testid={switchId}
        onCheckedChange={handleToggle}
      />
      <AlertDialog
        onOpenChange={(open) => {
          if (!open) {
            // Cancel / dismiss: leave the setting unchanged.
            setPendingValue(null);
          }
        }}
        open={pendingValue !== null}
      >
        <AlertDialogContent
          data-testid={`feature-restart-dialog-${feature.id}`}
        >
          <AlertDialogHeader>
            <AlertDialogTitle>
              {pendingValue
                ? `Enable ${feature.name}?`
                : `Disable ${feature.name}?`}
            </AlertDialogTitle>
            <AlertDialogDescription>
              This setting takes effect when agents start. Applying it will
              restart {agentCountLabel} so they pick up the change; stopped
              agents stay stopped. Cancel to leave the setting unchanged.
            </AlertDialogDescription>
          </AlertDialogHeader>
          <AlertDialogFooter>
            <AlertDialogCancel asChild>
              <Button type="button" variant="outline">
                Cancel
              </Button>
            </AlertDialogCancel>
            <AlertDialogAction asChild>
              <Button onClick={handleConfirm} type="button">
                Apply and restart agents
              </Button>
            </AlertDialogAction>
          </AlertDialogFooter>
        </AlertDialogContent>
      </AlertDialog>
    </div>
  );
}

export function ExperimentalFeaturesCard() {
  // Manifest is preview-only by definition; every desktop entry is a preview
  // feature.
  const previewFeatures = desktopFeatures;

  return (
    <section className="min-w-0" data-testid="settings-experimental">
      <SettingsSectionHeader
        title="Experiments"
        description={
          <>
            These features are functional but still being refined. Enable them
            to try new capabilities early.
          </>
        }
      />

      <div className="flex flex-col gap-2">
        {previewFeatures.map((f) => (
          <FeatureRow feature={f} key={f.id} />
        ))}
      </div>
    </section>
  );
}
