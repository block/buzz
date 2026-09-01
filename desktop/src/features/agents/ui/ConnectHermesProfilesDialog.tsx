import * as React from "react";

import {
  useAcpRuntimesQuery,
  useCreateManagedAgentMutation,
  useCreatePersonaMutation,
  useDeletePersonaMutation,
  useManagedAgentsQuery,
} from "@/features/agents/hooks";
import { buildInstanceInputForDefinition } from "@/features/agents/lib/instanceInputForDefinition";
import { connectHermesProfiles } from "@/features/agents/lib/connectHermesProfiles";
import { hermesProfileNameFromAgent } from "@/features/agents/lib/hermesProfileBinding";
import { useHermesProfilesQuery } from "@/features/agents/useHermesProfiles";
import { Button } from "@/shared/ui/button";
import { Checkbox } from "@/shared/ui/checkbox";
import type { AcpRuntime } from "@/shared/api/types";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

export function ConnectHermesProfilesDialog({
  open,
  onOpenChange,
}: {
  open: boolean;
  onOpenChange: (open: boolean) => void;
}) {
  const profilesQuery = useHermesProfilesQuery({ enabled: open });
  const runtimesQuery = useAcpRuntimesQuery({ enabled: open });
  const agentsQuery = useManagedAgentsQuery({ enabled: open });
  const createPersona = useCreatePersonaMutation();
  const createAgent = useCreateManagedAgentMutation();
  const deletePersona = useDeletePersonaMutation();
  const [selected, setSelected] = React.useState<Set<string>>(() => new Set());
  const [running, setRunning] = React.useState(false);
  const [summary, setSummary] = React.useState<string | null>(null);
  const initializedForOpenRef = React.useRef(false);

  const bound = React.useMemo(
    () =>
      new Set(
        (agentsQuery.data ?? [])
          .map(hermesProfileNameFromAgent)
          .filter((name): name is string => name !== null),
      ),
    [agentsQuery.data],
  );
  const eligible = React.useMemo(
    () =>
      (profilesQuery.data?.profiles ?? []).filter(
        (profile) => !profile.gatewayRunning && !bound.has(profile.name),
      ),
    [bound, profilesQuery.data?.profiles],
  );

  React.useEffect(() => {
    if (!open) {
      initializedForOpenRef.current = false;
      return;
    }
    if (
      initializedForOpenRef.current ||
      profilesQuery.isLoading ||
      agentsQuery.isLoading
    ) {
      return;
    }
    initializedForOpenRef.current = true;
    setSelected(new Set(eligible.map((profile) => profile.name)));
    setSummary(null);
  }, [agentsQuery.isLoading, eligible, open, profilesQuery.isLoading]);

  const hermesRuntime = (runtimesQuery.data ?? []).find(
    (runtime): runtime is AcpRuntime =>
      runtime.id === "hermes" &&
      runtime.availability === "available" &&
      typeof runtime.command === "string",
  );

  async function handleConnect() {
    if (!hermesRuntime || selected.size === 0 || running) return;
    setRunning(true);
    setSummary(null);
    const byName = new Map(
      (profilesQuery.data?.profiles ?? []).map((profile) => [
        profile.name,
        profile,
      ]),
    );
    const result = await connectHermesProfiles({
      profiles: [...selected],
      concurrency: 2,
      connect: async (profileName) => {
        const profile = byName.get(profileName);
        if (!profile) throw new Error("Profile disappeared during discovery.");
        const persona = await createPersona.mutateAsync({
          displayName: profile.displayName || profile.name,
          systemPrompt:
            profile.description ||
            `Run as the Hermes profile ${profile.name} and use its configured identity, memory, and tools.`,
          runtime: "hermes",
          envVars: {},
          behavior: { respondTo: "owner-only" },
        });
        try {
          const input = await buildInstanceInputForDefinition(
            persona,
            hermesRuntime,
            undefined,
            undefined,
            { type: "hermes_profile", profileName },
          );
          const created = await createAgent.mutateAsync(input);
          return {
            pubkey: created.agent.pubkey,
            startError: created.spawnError,
          };
        } catch (cause) {
          await deletePersona.mutateAsync(persona.id).catch(() => undefined);
          throw cause;
        }
      },
    });
    const startFailures = result.successes.filter(
      (entry) => entry.value.startError,
    ).length;
    setSummary(
      `${result.successes.length} connected${startFailures ? `, ${startFailures} did not start` : ""}${result.failures.length ? `, ${result.failures.length} failed` : ""}.`,
    );
    setSelected(new Set());
    setRunning(false);
  }

  const profiles = profilesQuery.data?.profiles ?? [];
  const selectedCount = selected.size;
  return (
    <Dialog onOpenChange={onOpenChange} open={open}>
      <DialogContent className="max-w-2xl overflow-hidden p-0">
        <div className="flex max-h-[85vh] flex-col">
          <DialogHeader className="border-b border-border/60 px-6 py-5 pr-14">
            <DialogTitle>Connect Hermes profiles</DialogTitle>
            <DialogDescription>
              Create normal Buzz agents from eligible local profiles. Starts at
              most two profiles at once.
            </DialogDescription>
          </DialogHeader>
          <div className="min-h-0 flex-1 space-y-3 overflow-y-auto px-6 py-5">
            {profilesQuery.isLoading || agentsQuery.isLoading ? (
              <p className="text-sm text-muted-foreground">
                Discovering profiles…
              </p>
            ) : null}
            {profiles.map((profile) => {
              const isBound = bound.has(profile.name);
              const disabled = running || profile.gatewayRunning || isBound;
              return (
                <label
                  className="flex items-start gap-3 rounded-xl border border-border/60 px-3 py-3"
                  htmlFor={`hermes-profile-connect-${profile.name}`}
                  key={profile.name}
                >
                  <Checkbox
                    checked={selected.has(profile.name)}
                    disabled={disabled}
                    id={`hermes-profile-connect-${profile.name}`}
                    onCheckedChange={(checked) => {
                      setSelected((current) => {
                        const next = new Set(current);
                        if (checked === true) next.add(profile.name);
                        else next.delete(profile.name);
                        return next;
                      });
                    }}
                  />
                  <span className="min-w-0 flex-1">
                    <span className="block text-sm font-medium">
                      {profile.displayName || profile.name}
                    </span>
                    <span className="block text-xs text-muted-foreground">
                      {profile.name}
                      {profile.model ? ` · ${profile.model}` : ""}
                      {profile.gatewayRunning ? " · gateway running" : ""}
                      {isBound ? " · already connected" : ""}
                    </span>
                  </span>
                </label>
              );
            })}
            {profilesQuery.isError ? (
              <p className="text-sm text-destructive">
                {profilesQuery.error instanceof Error
                  ? profilesQuery.error.message
                  : "Could not discover Hermes profiles."}
              </p>
            ) : null}
            {!runtimesQuery.isLoading && !hermesRuntime ? (
              <p className="text-sm text-destructive">
                Hermes Agent is not available on this computer.
              </p>
            ) : null}
            {summary ? (
              <p className="text-sm" data-testid="hermes-connect-summary">
                {summary}
              </p>
            ) : null}
          </div>
          <div className="flex items-center justify-between border-t border-border/60 px-6 py-4">
            <span className="text-xs text-muted-foreground">
              {eligible.length} eligible · {selectedCount} selected
            </span>
            <div className="flex gap-2">
              <Button
                onClick={() => onOpenChange(false)}
                type="button"
                variant="outline"
              >
                Close
              </Button>
              <Button
                data-testid="connect-hermes-profiles-submit"
                disabled={!hermesRuntime || selectedCount === 0 || running}
                onClick={() => void handleConnect()}
                type="button"
              >
                {running
                  ? "Connecting…"
                  : `Connect ${selectedCount || "selected"}`}
              </Button>
            </div>
          </div>
        </div>
      </DialogContent>
    </Dialog>
  );
}
