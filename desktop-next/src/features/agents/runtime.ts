import { invoke, isTauri } from "@tauri-apps/api/core";
import type { AgentRuntime, ManagedAgent } from "./types";

type RawManagedAgent = {
  pubkey: string;
  name: string;
  runtime?: string | null;
  agent_command: string;
  system_prompt?: string | null;
  model?: string | null;
  provider?: string | null;
  status: ManagedAgent["status"];
  last_error?: string | null;
};

type RawRuntime = {
  id: string;
  label: string;
  availability: AgentRuntime["availability"];
  command: string | null;
  auth_status: AgentRuntime["authStatus"];
  install_hint: string;
  login_hint?: string | null;
};

const mockAgents: ManagedAgent[] = [
  {
    pubkey: "vogue-agent",
    name: "Vogue",
    runtime: "goose",
    agentCommand: "goose",
    systemPrompt: "Shape product interfaces with clarity and taste.",
    model: "Claude Fable 5",
    provider: "Databricks",
    status: "running",
    lastError: null,
  },
];

function agent(raw: RawManagedAgent): ManagedAgent {
  return {
    pubkey: raw.pubkey,
    name: raw.name,
    runtime: raw.runtime ?? null,
    agentCommand: raw.agent_command,
    systemPrompt: raw.system_prompt ?? null,
    model: raw.model ?? null,
    provider: raw.provider ?? null,
    status: raw.status,
    lastError: raw.last_error ?? null,
  };
}

function catalogEntry(raw: RawRuntime): AgentRuntime {
  return {
    id: raw.id,
    label: raw.label,
    availability: raw.availability,
    command: raw.command,
    authStatus: raw.auth_status,
    installHint: raw.install_hint,
    loginHint: raw.login_hint ?? null,
  };
}

export const agentRuntime = {
  async list(): Promise<ManagedAgent[]> {
    if (!isTauri()) return [...mockAgents];
    return (await invoke<RawManagedAgent[]>("list_managed_agents")).map(agent);
  },

  async catalog(): Promise<AgentRuntime[]> {
    if (!isTauri()) {
      return [
        {
          id: "goose",
          label: "Goose",
          availability: "available",
          command: "goose",
          authStatus: { status: "not_applicable" },
          installHint: "",
          loginHint: null,
        },
      ];
    }
    return (
      await invoke<RawRuntime[]>("discover_acp_providers", { force: false })
    ).map(catalogEntry);
  },

  async create(input: {
    name: string;
    instructions: string;
    runtime: AgentRuntime;
    relayUrl?: string;
    signerPubkey?: string;
  }): Promise<ManagedAgent> {
    if (!input.runtime.command) throw new Error("This runtime is not ready.");
    if (!isTauri()) {
      const created: ManagedAgent = {
        pubkey: crypto.randomUUID(),
        name: input.name,
        runtime: input.runtime.id,
        agentCommand: input.runtime.command,
        systemPrompt: input.instructions,
        model: null,
        provider: null,
        status: "running",
        lastError: null,
      };
      mockAgents.unshift(created);
      return created;
    }
    const response = await invoke<{
      agent: RawManagedAgent;
      spawn_error?: string | null;
    }>("create_managed_agent", {
      input: {
        name: input.name,
        relayUrl: null,
        expectedRelayUrl: input.relayUrl ?? null,
        expectedSignerPubkey: input.signerPubkey ?? null,
        acpCommand: "buzz-acp",
        agentCommand: input.runtime.command,
        harnessOverride: true,
        agentArgs: [],
        mcpCommand: null,
        turnTimeoutSeconds: null,
        idleTimeoutSeconds: null,
        maxTurnDurationSeconds: null,
        parallelism: null,
        systemPrompt: input.instructions,
        avatarUrl: null,
        model: null,
        provider: null,
        envVars: {},
        spawnAfterCreate: true,
        startOnAppLaunch: false,
        backend: { type: "local" },
        respondTo: "owner-only",
        respondToAllowlist: [],
        relayMesh: null,
      },
    });
    if (response.spawn_error) throw new Error(response.spawn_error);
    return agent(response.agent);
  },

  async start(
    pubkey: string,
    scope?: { relayUrl: string; signerPubkey: string },
  ): Promise<ManagedAgent> {
    if (!isTauri()) {
      const current = mockAgents.find((item) => item.pubkey === pubkey);
      if (!current) throw new Error("Agent not found.");
      current.status = "running";
      return { ...current };
    }
    return agent(
      await invoke<RawManagedAgent>("start_managed_agent", {
        pubkey,
        expectedRelayUrl: scope?.relayUrl ?? null,
        expectedSignerPubkey: scope?.signerPubkey ?? null,
        replayFloorUnix: null,
      }),
    );
  },

  async stop(pubkey: string): Promise<ManagedAgent> {
    if (!isTauri()) {
      const current = mockAgents.find((item) => item.pubkey === pubkey);
      if (!current) throw new Error("Agent not found.");
      current.status = "stopped";
      return { ...current };
    }
    return agent(
      await invoke<RawManagedAgent>("stop_managed_agent", { pubkey }),
    );
  },
};
