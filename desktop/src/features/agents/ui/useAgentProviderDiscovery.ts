import * as React from "react";

import { discoverAgentProviders } from "@/shared/api/agentModels";
import type {
  AcpRuntimeCatalogEntry,
  AgentProviderInfo,
} from "@/shared/api/types";

import type { EnvVarsValue } from "./EnvVarsEditor";

function stableEnvKey(envVars: EnvVarsValue): string {
  return JSON.stringify(
    Object.entries(envVars).sort(([left], [right]) =>
      left.localeCompare(right),
    ),
  );
}

/**
 * Fetch the selected harness's own LLM provider inventory.
 *
 * Only harnesses whose catalog entry sets `providerInventory` are asked:
 * everything else would pay a subprocess spawn to learn what the static catalog
 * already says. A failed probe degrades to the built-in rows — the provider
 * control never goes empty because discovery failed.
 */
export function useAgentProviderDiscovery({
  enabled,
  envVars,
  selectedRuntime,
}: {
  enabled: boolean;
  envVars: EnvVarsValue;
  selectedRuntime: AcpRuntimeCatalogEntry | undefined;
}) {
  const [providers, setProviders] = React.useState<AgentProviderInfo[]>([]);
  const [loading, setLoading] = React.useState(false);
  const [initializedFor, setInitializedFor] = React.useState<string | null>(
    null,
  );

  const agentCommand = selectedRuntime?.command?.trim() || null;
  const providerInventory = selectedRuntime?.providerInventory === true;
  const defaultArgs = selectedRuntime?.defaultArgs;
  const definitionEnv = selectedRuntime?.definitionEnv;
  const envKey = React.useMemo(() => stableEnvKey(envVars), [envVars]);
  const argsKey = JSON.stringify(defaultArgs ?? []);
  const requestRef = React.useRef(0);

  // `envVars`/`definitionEnv` are keyed by their stable serializations
  // (`envKey`/`argsKey`) so a refetch that produces an equal object does not
  // re-spawn the harness subprocess.
  // biome-ignore lint/correctness/useExhaustiveDependencies: envVars is keyed by envKey; argsKey stands in for defaultArgs
  React.useEffect(() => {
    const requestId = requestRef.current + 1;
    requestRef.current = requestId;

    if (!enabled || !providerInventory || agentCommand === null) {
      setProviders([]);
      setLoading(false);
      return;
    }

    setLoading(true);
    setInitializedFor(null);
    void discoverAgentProviders({
      agentCommand,
      agentArgs: JSON.parse(argsKey) as string[],
      envVars,
      definitionEnv: definitionEnv ?? {},
    })
      .then((response) => {
        if (requestRef.current !== requestId) return;
        setProviders(response.providers);
      })
      .catch(() => {
        if (requestRef.current !== requestId) return;
        setProviders([]);
      })
      .finally(() => {
        if (requestRef.current !== requestId) return;
        setInitializedFor(agentCommand);
        setLoading(false);
      });
  }, [
    agentCommand,
    argsKey,
    definitionEnv,
    enabled,
    envKey,
    providerInventory,
  ]);

  const ready = initializedFor === agentCommand;
  const discoveredProviders = ready ? providers : [];

  return { discoveredProviders, providerDiscoveryLoading: loading };
}
