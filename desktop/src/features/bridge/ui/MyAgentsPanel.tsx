// Read-only "My agents" view for the web build, fed by the relay bridge (docs/pwa-bridge-protocol.md).
import * as React from "react";

import { useIdentityQuery } from "@/shared/api/hooks";
import { nowS } from "../protocol";
import { hiddenAgentCount, NO_RUNNING_AGENTS, visibleRelayAgents } from "../relayAgentView";
import { useBridgeAgents } from "../useBridgeAgents";
import { AgentControls } from "./AgentControls";
import { AgentDetail } from "./AgentDetail";

function ago(s: number): string {
  if (s < 60) return `${s}s ago`;
  if (s < 3600) return `${Math.floor(s / 60)}m ago`;
  return `${Math.floor(s / 3600)}h ago`;
}

export function MyAgentsPanel() {
  const pubkey = useIdentityQuery().data?.pubkey;
  const b = useBridgeAgents(pubkey);
  const [, force] = React.useReducer((x: number) => x + 1, 0);
  React.useEffect(() => {
    const t = window.setInterval(force, 10_000);
    return () => window.clearInterval(t);
  }, []);
  const [selected, setSelected] = React.useState<string | null>(null);
  if (!b.available) return null;
  const age =
    b.lastHeardTs === null ? null : Math.max(0, nowS() - b.lastHeardTs);
  const offline = b.presence === "offline";
  // "Allow remote agent management" on the desktop (advertised in the encrypted heartbeat). Listing works without it; controls need it.
  const managementOn = b.heartbeat?.remote?.agents === true;
  const shownAgents = b.snapshot ? visibleRelayAgents(b.snapshot.agents) : []; // relay bridge: running agents only
  const hiddenCount = b.snapshot ? hiddenAgentCount(b.snapshot.agents) : 0;
  const selectedAgent = selected
    ? b.snapshot?.agents.find((a) => a.pubkey === selected)
    : undefined;
  if (selected && selectedAgent)
    return (
      <AgentDetail
        agent={selectedAgent}
        entries={b.transcripts.get(selectedAgent.pubkey.toLowerCase()) ?? []}
        offline={offline}
        onBack={() => setSelected(null)}
        controls={
          b.commander && managementOn ? (
            <AgentControls agent={selectedAgent} commander={b.commander} bridge={b} />
          ) : null
        }
      />
    );
  return (
    <section
      data-testid="my-agents"
      data-presence={b.presence}
      data-key-state={b.keyState}
      data-refreshing={b.refreshing ? "1" : "0"}
      style={{ padding: 16 }}
    >
      <h2 style={{ fontSize: 16, fontWeight: 600, margin: 0 }}>My agents</h2>
      <p
        data-testid="my-agents-presence"
        style={{ fontSize: 12, opacity: 0.75, margin: "4px 0 12px" }}
      >
        {b.presence === "online" &&
          `Desktop online${age !== null ? ` · updated ${ago(age)}` : ""}`}
        {offline &&
          `Desktop offline${age !== null ? ` · last seen ${ago(age)}` : ""}`}
        {b.presence === "unknown" &&
          (b.keyState === "decrypting"
            ? "Connecting to your desktop…"
            : "Waiting for your desktop…")}
        {b.refreshing && b.presence !== "unknown" && " · refreshing…"}
        {b.keyState === "failed" &&
          " · couldn't unlock the bridge key (approve the request in your signer)"}
      </p>
      {!managementOn && (
        <div
          data-testid="agents-management-info"
          style={{ border: "1px solid rgba(128,128,128,.3)", borderRadius: 8, padding: "8px 12px", margin: "0 0 12px", fontSize: 12 }}
        >
          <strong style={{ fontSize: 13 }}>Agent management is off</strong>
          <p style={{ margin: "4px 0 0", opacity: 0.85 }}>
            You can see your desktop's agents and what they are working on, but start, stop, message and create are not available.
            To enable them, turn on Settings → Agents → Allow remote agent management on your desktop, and make sure the desktop is online.
          </p>
          <ul style={{ margin: "6px 0 0", paddingLeft: 18, opacity: 0.85 }}>
            <li>Over the relay (works from anywhere, slower): list, start, stop and message agents.</li>
            <li>Over a direct tailnet connection (faster): everything above plus create, rename, change provider, model and persona, and delete.</li>
          </ul>
        </div>
      )}
      {b.snapshot && shownAgents.length === 0 && (
        <p data-testid="my-agents-empty" style={{ fontSize: 13, opacity: 0.7 }}>{NO_RUNNING_AGENTS}</p>
      )}
      {hiddenCount > 0 && (
        <p data-testid="my-agents-hidden-note" style={{ fontSize: 11, opacity: 0.6, margin: "0 0 8px" }}>
          {hiddenCount} stopped {hiddenCount === 1 ? "agent is" : "agents are"} hidden here; only running agents are listed over the relay.
        </p>
      )}
      <ul
        style={{
          listStyle: "none",
          padding: 0,
          margin: 0,
          display: "grid",
          gap: 8,
          opacity: offline ? 0.55 : 1,
        }}
      >
        {shownAgents.map((a) => (
          <li
            key={a.pubkey}
            data-testid="my-agent-row"
            data-status={a.status}
            role="button"
            tabIndex={0}
            onClick={() => setSelected(a.pubkey)}
            onKeyDown={(e) => {
              if (e.key === "Enter") setSelected(a.pubkey);
            }}
            style={{
              cursor: "pointer",
              border: "1px solid rgba(128,128,128,.3)",
              borderRadius: 8,
              padding: "8px 12px",
            }}
          >
            <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                gap: 8,
              }}
            >
              <strong style={{ fontSize: 14 }}>{a.name}</strong>
              <span style={{ fontSize: 12 }}>
                {a.status === "running"
                  ? "● running"
                  : a.status.replace("_", " ")}
              </span>
            </div>
            {a.currentTask && (
              <div style={{ fontSize: 12, opacity: 0.8 }}>{a.currentTask}</div>
            )}
            {(a.runtime || a.model) && (
              <div style={{ fontSize: 11, opacity: 0.6 }}>
                {[a.runtime, a.model].filter(Boolean).join(" · ")}
              </div>
            )}
            {a.lastError && a.status !== "running" && (
              <div style={{ fontSize: 11, color: "#c0392b" }}>
                {a.lastError}
              </div>
            )}
          </li>
        ))}
      </ul>
      <p data-testid="browsers-pointer" style={{ fontSize: 12, opacity: 0.7, marginTop: 12 }}>
        Your desktop's browsers (add, screenshot, remove, observe and drive) are on the Browsers page, and its terminal is on the Buzz term page, both in the left menu.
      </p>
      <p style={{ fontSize: 11, opacity: 0.5, marginTop: 12 }}>
        Tap an agent to see its recent activity.
      </p>
    </section>
  );
}
