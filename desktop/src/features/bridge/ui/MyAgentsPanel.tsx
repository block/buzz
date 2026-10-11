// Read-only "My agents" view for the web build, fed by the relay bridge (docs/pwa-bridge-protocol.md).
import * as React from "react";

import { useIdentityQuery } from "@/shared/api/hooks";
import { nowS } from "../protocol";
import { useBridgeAgents } from "../useBridgeAgents";
import { AgentControls } from "./AgentControls";
import { AgentDetail } from "./AgentDetail";
import { BrowserPanel } from "./BrowserPanel";

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
          b.commander ? (
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
        {b.keyState === "failed" &&
          " · couldn't unlock the bridge key (approve the request in your signer)"}
      </p>
      {b.snapshot && b.snapshot.agents.length === 0 && (
        <p style={{ fontSize: 13, opacity: 0.7 }}>No agents on your desktop.</p>
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
        {b.snapshot?.agents.map((a) => (
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
      {b.commander && <BrowserPanel commander={b.commander} bridge={b} />}
      <p style={{ fontSize: 11, opacity: 0.5, marginTop: 12 }}>
        Tap an agent to see its recent activity.
      </p>
    </section>
  );
}
