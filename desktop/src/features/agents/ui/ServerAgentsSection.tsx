import { Check, CircleAlert, Play, Square, Trash2 } from "lucide-react";
import React from "react";
import { toast } from "sonner";

import type {
  SpawnerAnnouncement,
  SpawnPhase,
} from "@/shared/api/spawnerRelay";
import type { AgentPersona } from "@/shared/api/types";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/shared/ui/dropdown-menu";
import { Input } from "@/shared/ui/input";
import { SectionHeader } from "@/shared/ui/PageHeader";
import {
  sameLocation,
  setDefaultAgentLocation,
  useDefaultAgentLocation,
} from "../agentLocation";
import { useSpawnerDirectory } from "../spawnerDirectoryStore";
import { addSpawner, removeSpawner, slugFromName } from "../spawnerPreference";
import { useServerAgents, type ServerAgent } from "../useServerAgents";
import { shortenPubkey } from "./SpawnerAttestationDialog";
import { SpawnerCredentialCard } from "./SpawnerCredentialCard";

/** Human-readable label and tone for a reconciliation phase. */
export function phaseLabel(phase: SpawnPhase): {
  label: string;
  variant: "secondary" | "destructive" | "warning" | "success" | "info";
} {
  switch (phase) {
    case "running":
      return { label: "Running", variant: "success" };
    case "starting":
      return { label: "Starting", variant: "info" };
    case "pending_attestation":
      // Warning, not neutral: this state needs the user to do something, and
      // the agent stays dead until they do.
      return { label: "Awaiting approval", variant: "warning" };
    case "stopped":
      return { label: "Stopped", variant: "secondary" };
    case "failed":
      return { label: "Failed", variant: "destructive" };
  }
}

/**
 * Friendly name for an advertised agent runtime.
 *
 * Purely cosmetic: the value is self-reported by the host and controls nothing.
 * Unknown values pass through so a spawner running something we have never heard
 * of still reads sensibly.
 */
export function runtimeLabel(runtime: string | undefined): string | undefined {
  if (!runtime) return undefined;
  switch (runtime) {
    case "claude-agent-acp":
    case "claude-code-acp":
      return "Claude Code";
    case "buzz-agent":
      return "Buzz agent";
    case "goose":
      return "goose";
    case "codex-acp":
      return "Codex";
    default:
      return runtime;
  }
}

/** Display label for a spawner, preferring its announced name. */
export function spawnerLabel(
  pubkey: string,
  directory: ReadonlyMap<string, SpawnerAnnouncement>,
): string {
  return directory.get(pubkey)?.name ?? shortenPubkey(pubkey);
}

type ServerAgentsSectionProps = {
  personas: AgentPersona[];
};

/**
 * Server-hosted agents: the ones that keep running when this app is closed.
 *
 * Supports several spawners at once — a GPU box for agents that need it and a
 * cheap VPS for the rest — because a spawner is addressed by pubkey through the
 * relay and can live anywhere with outbound WebSocket.
 */
export function ServerAgentsSection({ personas }: ServerAgentsSectionProps) {
  const {
    spawners,
    agents,
    isPending,
    create,
    setEnabled,
    remove,
    hasServerAgent,
  } = useServerAgents();
  const directory = useSpawnerDirectory();

  // Keyed by the slug an agent's spec is published under, because that is the
  // only name a ServerAgent carries — keying by display name would miss every
  // persona whose name is not already its own slug ("Fizz" vs "fizz"), and a
  // miss republishes the spec without its personaId.
  const personaBySlug = React.useMemo(() => {
    const map = new Map<string, AgentPersona>();
    for (const persona of personas) {
      const slug = slugFromName(persona.displayName);
      if (slug) map.set(slug, persona);
    }
    return map;
  }, [personas]);

  // Announced spawners this device has not connected to yet — the discovery
  // path that replaces pasting 64 hex characters.
  const undiscovered = React.useMemo(
    () => [...directory.values()].filter((a) => !spawners.includes(a.pubkey)),
    [directory, spawners],
  );

  const handleDeploy = async (persona: AgentPersona, spawner: string) => {
    try {
      await create(persona, spawner);
      toast.success(
        `Deploying ${persona.displayName} to ${spawnerLabel(spawner, directory)}. Approve the key when prompted.`,
      );
    } catch (error) {
      toast.error(errorMessage(error, "Failed to deploy the agent."));
    }
  };

  return (
    <section className="space-y-3">
      <SectionHeader
        action={
          spawners.length > 0 ? (
            <DeployMenu
              directory={directory}
              disabled={isPending}
              hasServerAgent={hasServerAgent}
              onDeploy={handleDeploy}
              personas={personas}
              spawners={spawners}
            />
          ) : undefined
        }
        title="Server agents"
      />

      {spawners.length === 0 ? (
        <p className="text-sm text-muted-foreground">
          Run agents on a server so they keep working when Buzz is closed. A
          spawner can live anywhere that can reach this relay — it does not have
          to be the relay machine.
        </p>
      ) : null}

      {undiscovered.length > 0 ? (
        <DiscoveredSpawners spawners={undiscovered} />
      ) : null}

      {spawners.map((spawner) => {
        const spawnerAgents = agents.filter((a) => a.spawnerPubkey === spawner);
        return (
          <div className="space-y-2" key={spawner}>
            <div className="flex items-baseline justify-between gap-2">
              <p className="text-sm font-medium">
                {spawnerLabel(spawner, directory)}{" "}
                <span className="text-2xs font-normal text-muted-foreground">
                  {runtimeLabel(directory.get(spawner)?.runtime) ??
                    shortenPubkey(spawner)}
                </span>
              </p>
              <div className="flex shrink-0 items-center gap-3">
                <DefaultLocationToggle spawner={spawner} />
                <button
                  className="text-xs text-muted-foreground underline underline-offset-2"
                  onClick={() => removeSpawner(spawner)}
                  type="button"
                >
                  Disconnect
                </button>
              </div>
            </div>
            {spawnerAgents.length === 0 ? (
              <p className="text-sm text-muted-foreground">
                No agents here yet. Use "Deploy agent".
              </p>
            ) : (
              <ul className="space-y-2">
                {spawnerAgents.map((agent) => (
                  <ServerAgentRow
                    agent={agent}
                    isPending={isPending}
                    key={`${agent.spawnerPubkey}/${agent.slug}`}
                    onRemove={async () => {
                      try {
                        await remove(agent);
                        toast.success(`Removed "${agent.slug}".`);
                      } catch (error) {
                        toast.error(
                          errorMessage(error, "Failed to remove the agent."),
                        );
                      }
                    }}
                    onToggle={async (enabled) => {
                      try {
                        await setEnabled(
                          agent,
                          enabled,
                          personaBySlug.get(agent.slug),
                        );
                      } catch (error) {
                        toast.error(
                          errorMessage(error, "Failed to update the agent."),
                        );
                      }
                    }}
                  />
                ))}
              </ul>
            )}
            <SpawnerCredentialCard
              spawnerName={spawnerLabel(spawner, directory)}
              spawnerPubkey={spawner}
            />
          </div>
        );
      })}

      <SpawnerConnectCard />
    </section>
  );
}

/**
 * Spawners that announced themselves but are not connected yet.
 *
 * Deliberately worded as a claim rather than an endorsement: anyone can publish
 * a kind:10180, so this list is a phone book. Connecting is a user action, and
 * running an agent still needs a signed attestation.
 */
function DiscoveredSpawners({ spawners }: { spawners: SpawnerAnnouncement[] }) {
  return (
    <div className="space-y-2 rounded-md border border-border p-3">
      <p className="text-sm font-medium">Spawners on this relay</p>
      <p className="text-2xs text-muted-foreground">
        Anyone can advertise here. Connecting does not grant access — you still
        approve each agent's key.
      </p>
      <ul className="space-y-2">
        {spawners.map((spawner) => (
          <li className="flex items-center gap-3" key={spawner.pubkey}>
            <div className="min-w-0 flex-1">
              <p className="truncate text-sm">
                {spawner.name}{" "}
                <span className="text-2xs text-muted-foreground">
                  {shortenPubkey(spawner.pubkey)}
                </span>
              </p>
              <p className="text-2xs text-muted-foreground">
                {spawner.agentsRunning}/{spawner.maxAgents} agents
                {runtimeLabel(spawner.runtime)
                  ? ` · ${runtimeLabel(spawner.runtime)}`
                  : ""}
                {spawner.description ? ` · ${spawner.description}` : ""}
              </p>
            </div>
            <Button
              onClick={() => {
                if (!addSpawner(spawner.pubkey)) {
                  toast.error("That spawner advertised an invalid public key.");
                }
              }}
              size="sm"
              type="button"
              variant="outline"
            >
              Connect
            </Button>
          </li>
        ))}
      </ul>
    </div>
  );
}

function ServerAgentRow({
  agent,
  isPending,
  onToggle,
  onRemove,
}: {
  agent: ServerAgent;
  isPending: boolean;
  onToggle: (enabled: boolean) => void;
  onRemove: () => void;
}) {
  // The phase is `stopped`, but *why* is the useful part.
  const { label, variant } = agent.status.needsCredential
    ? { label: "Needs credential", variant: "warning" as const }
    : phaseLabel(agent.status.phase);
  const isStopped = agent.status.phase === "stopped";

  return (
    <li className="flex items-start gap-3 rounded-md border border-border p-3">
      <div className="min-w-0 flex-1 space-y-1">
        <div className="flex items-center gap-2">
          <span className="truncate text-sm font-medium">{agent.slug}</span>
          <Badge variant={variant}>{label}</Badge>
        </div>
        {agent.status.agentPubkey ? (
          <p className="text-2xs text-muted-foreground">
            {shortenPubkey(agent.status.agentPubkey)}
          </p>
        ) : null}
        {agent.status.error ? (
          <p className="flex items-start gap-1 text-xs text-destructive">
            <CircleAlert aria-hidden className="mt-0.5 size-3 shrink-0" />
            <span className="min-w-0 break-words">{agent.status.error}</span>
          </p>
        ) : null}
        {agent.status.needsCredential ? (
          <p className="text-2xs text-muted-foreground">
            Add your Claude credential below to start this agent.
          </p>
        ) : null}
        {agent.status.restartCount > 0 ? (
          <p className="text-2xs text-muted-foreground">
            {agent.status.restartCount} failed start
            {agent.status.restartCount === 1 ? "" : "s"}
          </p>
        ) : null}
      </div>
      <div className="flex shrink-0 items-center gap-1">
        <Button
          aria-label={isStopped ? "Start agent" : "Stop agent"}
          disabled={isPending}
          onClick={() => onToggle(isStopped)}
          size="icon"
          type="button"
          variant="ghost"
        >
          {isStopped ? (
            <Play className="size-4" />
          ) : (
            <Square className="size-4" />
          )}
        </Button>
        <Button
          aria-label="Remove agent"
          disabled={isPending}
          onClick={onRemove}
          size="icon"
          type="button"
          variant="ghost"
        >
          <Trash2 className="size-4" />
        </Button>
      </div>
    </li>
  );
}

/**
 * Picks a persona and a spawner to run it on.
 *
 * Grouped by spawner rather than asking twice: with several hosts connected,
 * "which agent, and where" is one decision. A persona already deployed to a
 * given host is omitted from that host's group, so a re-deploy cannot silently
 * overwrite a running agent's spec — but it stays available on other hosts,
 * since running the same persona in two places is legitimate.
 */
function DeployMenu({
  personas,
  spawners,
  directory,
  disabled,
  hasServerAgent,
  onDeploy,
}: {
  personas: AgentPersona[];
  spawners: readonly string[];
  directory: ReadonlyMap<string, SpawnerAnnouncement>;
  disabled: boolean;
  hasServerAgent: (persona: AgentPersona, spawner: string) => boolean;
  onDeploy: (persona: AgentPersona, spawner: string) => void;
}) {
  const groups = spawners.map((spawner) => ({
    spawner,
    personas: personas.filter((p) => !hasServerAgent(p, spawner)),
  }));
  const hasAnything = groups.some((g) => g.personas.length > 0);

  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <Button
          disabled={disabled || !hasAnything}
          size="sm"
          type="button"
          variant="outline"
        >
          Deploy agent
        </Button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end">
        {groups.map((group, index) => (
          <React.Fragment key={group.spawner}>
            {index > 0 ? <DropdownMenuSeparator /> : null}
            <DropdownMenuLabel>
              {spawnerLabel(group.spawner, directory)}
            </DropdownMenuLabel>
            {group.personas.length === 0 ? (
              <DropdownMenuItem disabled>All agents deployed</DropdownMenuItem>
            ) : (
              group.personas.map((persona) => (
                <DropdownMenuItem
                  key={persona.id}
                  onSelect={() => onDeploy(persona, group.spawner)}
                >
                  {persona.displayName}
                </DropdownMenuItem>
              ))
            )}
          </React.Fragment>
        ))}
      </DropdownMenuContent>
    </DropdownMenu>
  );
}

/**
 * Marks a spawner as the default location for new agents.
 *
 * Location is stored once and inherited, rather than copied onto each persona:
 * the built-in Fizz/Honey/Bumble carry no location of their own, so flipping
 * this moves every agent that has not been given an explicit one.
 */
function DefaultLocationToggle({ spawner }: { spawner: string }) {
  const current = useDefaultAgentLocation();
  const isDefault = sameLocation(current, {
    kind: "spawner",
    spawnerPubkey: spawner,
  });

  if (isDefault) {
    return (
      <span className="flex items-center gap-1 text-xs text-muted-foreground">
        <Check aria-hidden className="size-3" />
        Default for new agents
      </span>
    );
  }
  return (
    <button
      className="text-xs text-muted-foreground underline underline-offset-2"
      onClick={() => {
        if (
          !setDefaultAgentLocation({ kind: "spawner", spawnerPubkey: spawner })
        ) {
          toast.error("That spawner key is not valid.");
          return;
        }
        toast.success("New agents will run here by default.");
      }}
      type="button"
    >
      Make default
    </button>
  );
}

/** Connect to a spawner by pubkey, for one that has not announced itself. */
function SpawnerConnectCard() {
  const [value, setValue] = React.useState("");

  return (
    <div className="flex gap-2">
      <Input
        onChange={(event) => setValue(event.target.value)}
        placeholder="Spawner public key (64 hex characters)"
        value={value}
      />
      <Button
        disabled={value.trim().length === 0}
        onClick={() => {
          if (!addSpawner(value.trim())) {
            toast.error("That is not a valid 64-character hex public key.");
            return;
          }
          setValue("");
        }}
        type="button"
        variant="outline"
      >
        Connect
      </Button>
    </div>
  );
}

function errorMessage(error: unknown, fallback: string): string {
  return error instanceof Error ? error.message : fallback;
}
