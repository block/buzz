import {
  IconPlayerPlay,
  IconPlayerStop,
  IconPlus,
  IconRobot,
} from "@tabler/icons-react";
import { useCallback, useEffect, useMemo, useState } from "react";
import { runtime } from "@/shared/runtime/client";
import { agentRuntime } from "../runtime";
import type { AgentRuntime, ManagedAgent } from "../types";

function runtimeReady(runtime: AgentRuntime) {
  return (
    runtime.availability === "available" &&
    (runtime.authStatus.status === "logged_in" ||
      runtime.authStatus.status === "not_applicable")
  );
}

export function AgentsNavigator({
  agents,
  selectedPubkey,
  onSelect,
  onNew,
}: {
  agents: ManagedAgent[];
  selectedPubkey?: string;
  onSelect: (pubkey: string) => void;
  onNew: () => void;
}) {
  return (
    <aside className="workspace-navigator" aria-label="Agents">
      <header className="panel-heading panel-heading-between">
        <span className="text-heading text-primary">Agents</span>
        <button
          type="button"
          className="navigator-row-action always-visible"
          onClick={onNew}
          aria-label="New agent"
        >
          <IconPlus size={15} stroke={1.7} aria-hidden="true" />
        </button>
      </header>
      <div className="navigator-list">
        {agents.map((agent) => (
          <button
            type="button"
            className="agent-navigator-row"
            data-selected={selectedPubkey === agent.pubkey || undefined}
            key={agent.pubkey}
            onClick={() => onSelect(agent.pubkey)}
          >
            <span className="agent-avatar">
              <IconRobot size={15} stroke={1.5} aria-hidden="true" />
            </span>
            <span className="agent-row-copy">
              <span className="text-body text-primary">{agent.name}</span>
              <span className="text-body-sm text-tertiary">
                {agent.status === "running" ? "Running" : "Stopped"}
              </span>
            </span>
            <span
              className="agent-presence"
              data-running={agent.status === "running" || undefined}
              aria-hidden="true"
            />
          </button>
        ))}
      </div>
    </aside>
  );
}

function AgentCreation({
  runtimes,
  onCreated,
}: {
  runtimes: AgentRuntime[];
  onCreated: (agent: ManagedAgent) => void;
}) {
  const available = runtimes.filter(runtimeReady);
  const [name, setName] = useState("");
  const [instructions, setInstructions] = useState("");
  const [runtimeId, setRuntimeId] = useState(available[0]?.id ?? "");
  const [error, setError] = useState<string | null>(null);
  const [saving, setSaving] = useState(false);
  const selected = runtimes.find((runtime) => runtime.id === runtimeId);

  async function create() {
    if (!selected || !runtimeReady(selected)) return;
    setSaving(true);
    setError(null);
    try {
      const [relayUrl, identity] = await Promise.all([
        runtime.relayUrl(),
        runtime.identity(),
      ]);
      onCreated(
        await agentRuntime.create({
          name: name.trim(),
          instructions: instructions.trim(),
          runtime: selected,
          relayUrl,
          signerPubkey: identity.pubkey,
        }),
      );
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setSaving(false);
    }
  }

  return (
    <main className="agent-main-panel agent-create-panel">
      <header className="panel-heading">
        <h1 className="text-heading text-primary">New agent</h1>
      </header>
      <div className="agent-form">
        <label className="field-stack">
          <span className="text-body-sm text-secondary">Name</span>
          <input
            value={name}
            onChange={(event) => setName(event.target.value)}
            placeholder="Scout"
          />
        </label>
        <label className="field-stack">
          <span className="text-body-sm text-secondary">Instructions</span>
          <textarea
            value={instructions}
            onChange={(event) => setInstructions(event.target.value)}
            placeholder="Describe its role, boundaries, and how it should respond."
          />
        </label>
        <label className="field-stack">
          <span className="text-body-sm text-secondary">Runs with</span>
          <select
            value={runtimeId}
            onChange={(event) => setRuntimeId(event.target.value)}
          >
            {runtimes.map((runtime) => (
              <option
                key={runtime.id}
                value={runtime.id}
                disabled={!runtimeReady(runtime)}
              >
                {runtime.label}
                {runtimeReady(runtime) ? "" : " — needs setup"}
              </option>
            ))}
          </select>
        </label>
        {selected && !runtimeReady(selected) ? (
          <p className="setup-callout text-body text-secondary" role="status">
            {selected.loginHint ||
              selected.installHint ||
              "This runtime needs setup before it can start an agent."}
          </p>
        ) : null}
        {error ? (
          <p className="text-body text-danger" role="alert">
            {error}
          </p>
        ) : null}
        <button
          type="button"
          className="primary-button agent-create-action"
          disabled={
            !name.trim() ||
            !instructions.trim() ||
            !selected ||
            !runtimeReady(selected) ||
            saving
          }
          onClick={() => void create()}
        >
          {saving ? "Creating…" : "Create and start"}
        </button>
      </div>
    </main>
  );
}

function AgentDetail({
  agent,
  onChange,
}: {
  agent: ManagedAgent;
  onChange: (agent: ManagedAgent) => void;
}) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  async function toggle() {
    setBusy(true);
    setError(null);
    try {
      if (agent.status === "running") {
        onChange(await agentRuntime.stop(agent.pubkey));
      } else {
        const [relayUrl, identity] = await Promise.all([
          runtime.relayUrl(),
          runtime.identity(),
        ]);
        onChange(
          await agentRuntime.start(agent.pubkey, {
            relayUrl,
            signerPubkey: identity.pubkey,
          }),
        );
      }
    } catch (caught) {
      setError(caught instanceof Error ? caught.message : String(caught));
    } finally {
      setBusy(false);
    }
  }
  return (
    <main className="agent-main-panel">
      <header className="agent-profile-header">
        <span className="agent-profile-avatar">
          <IconRobot size={24} stroke={1.4} aria-hidden="true" />
        </span>
        <div className="agent-profile-title">
          <h1 className="text-heading text-primary">{agent.name}</h1>
          <span className="text-body-sm text-secondary">
            {agent.status === "running" ? "Running" : "Stopped"}
          </span>
        </div>
        <button
          type="button"
          className="quiet-button agent-lifecycle-action"
          onClick={() => void toggle()}
          disabled={busy}
        >
          {agent.status === "running" ? (
            <IconPlayerStop size={15} aria-hidden="true" />
          ) : (
            <IconPlayerPlay size={15} aria-hidden="true" />
          )}
          {busy ? "Working…" : agent.status === "running" ? "Stop" : "Start"}
        </button>
      </header>
      <div className="agent-profile-body">
        <section>
          <p className="text-body-sm text-tertiary">Instructions</p>
          <p className="text-body text-primary">
            {agent.systemPrompt || "No instructions yet."}
          </p>
        </section>
        <section className="agent-runtime-summary">
          <div>
            <p className="text-body-sm text-tertiary">Runs with</p>
            <p className="text-body text-primary">
              {agent.runtime || agent.agentCommand}
            </p>
          </div>
          <div>
            <p className="text-body-sm text-tertiary">Model</p>
            <p className="text-body text-secondary">
              {agent.model || "Runtime default"}
            </p>
          </div>
        </section>
        {error || agent.lastError ? (
          <p className="setup-callout text-body text-danger" role="alert">
            {error || agent.lastError}
          </p>
        ) : null}
      </div>
    </main>
  );
}

export function useAgentsWorkspace() {
  const [agents, setAgents] = useState<ManagedAgent[]>([]);
  const [runtimes, setRuntimes] = useState<AgentRuntime[]>([]);
  const [selectedPubkey, setSelectedPubkey] = useState<string>();
  const [creating, setCreating] = useState(false);
  const refresh = useCallback(async () => {
    const [nextAgents, nextRuntimes] = await Promise.all([
      agentRuntime.list(),
      agentRuntime.catalog(),
    ]);
    setAgents(nextAgents);
    setRuntimes(nextRuntimes);
    setSelectedPubkey((current) => current ?? nextAgents[0]?.pubkey);
  }, []);
  useEffect(() => {
    void refresh();
  }, [refresh]);
  const selected = useMemo(
    () => agents.find((agent) => agent.pubkey === selectedPubkey),
    [agents, selectedPubkey],
  );
  return {
    navigator: (
      <AgentsNavigator
        agents={agents}
        selectedPubkey={selectedPubkey}
        onSelect={(pubkey) => {
          setCreating(false);
          setSelectedPubkey(pubkey);
        }}
        onNew={() => setCreating(true)}
      />
    ),
    content: creating ? (
      <AgentCreation
        runtimes={runtimes}
        onCreated={(created) => {
          setAgents((current) => [created, ...current]);
          setSelectedPubkey(created.pubkey);
          setCreating(false);
        }}
      />
    ) : selected ? (
      <AgentDetail
        agent={selected}
        onChange={(changed) =>
          setAgents((current) =>
            current.map((item) =>
              item.pubkey === changed.pubkey ? changed : item,
            ),
          )
        }
      />
    ) : (
      <main className="empty-workspace-panel">
        <IconRobot size={22} aria-hidden="true" />
        <h1 className="text-heading text-primary">Create your first agent</h1>
        <button
          type="button"
          className="primary-button"
          onClick={() => setCreating(true)}
        >
          Create agent
        </button>
      </main>
    ),
  };
}
