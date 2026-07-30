import * as React from "react";

import type { AgentLocation } from "../agentLocation";
import { useSpawnerDirectory } from "../spawnerDirectoryStore";
import { useSpawners } from "../spawnerPreference";
import {
  agentLocationOptions,
  agentLocationValue,
  parseAgentLocationValue,
} from "./agentLocationOptions";
import { runtimeLabel, spawnerLabel } from "./ServerAgentsSection";

/**
 * "Runs on" — which machine this agent lives on.
 *
 * Deliberately separate from the harness picker: a runtime says which binary
 * drives the agent, a location says whose computer runs it. Renders nothing
 * unless a spawner is connected, since "this Mac" is then the only answer.
 */
export function AgentRunsOnSection({
  isPending,
  location,
  onLocationChange,
}: {
  isPending: boolean;
  location: AgentLocation;
  onLocationChange: (next: AgentLocation) => void;
}) {
  const spawners = useSpawners();
  const directory = useSpawnerDirectory();
  const options = React.useMemo(
    () =>
      agentLocationOptions(spawners, (pubkey) => ({
        label: spawnerLabel(pubkey, directory),
        hint: runtimeLabel(directory.get(pubkey)?.runtime),
      })),
    [directory, spawners],
  );

  if (options.length === 0) return null;

  return (
    <div className="space-y-1.5">
      <label className="text-sm font-medium" htmlFor="agent-runs-on">
        Runs on
      </label>
      <select
        className="flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs"
        disabled={isPending}
        id="agent-runs-on"
        onChange={(event) =>
          onLocationChange(
            parseAgentLocationValue(event.target.value, spawners),
          )
        }
        value={agentLocationValue(location)}
      >
        {options.map((option) => (
          <option key={option.value} value={option.value}>
            {option.hint ? `${option.label} · ${option.hint}` : option.label}
          </option>
        ))}
      </select>
      <p className="text-2xs text-muted-foreground">
        {location.kind === "local"
          ? "This agent stops when Buzz is closed."
          : "The server runs this agent, so it keeps working when Buzz is closed. Approve its key when prompted."}
      </p>
    </div>
  );
}
