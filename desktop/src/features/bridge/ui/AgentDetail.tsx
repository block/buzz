// Web agent detail: recent redacted transcript (live), fed by the relay bridge. Controls live in AgentControls (slice A, controls).
import * as React from "react";

import type { AgentSummary, TranscriptEntry } from "../protocol";

const ROLE_LABEL: Record<string, string> = {
  assistant: "Agent",
  user: "User",
  tool: "Tool",
  thought: "Thinking",
  plan: "Plan",
  status: "Status",
  permission: "Permission",
  error: "Error",
};

function clock(ms: number): string {
  if (!ms) return "";
  return new Date(ms).toLocaleTimeString([], {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
  });
}

export function AgentDetail(props: {
  agent: AgentSummary;
  entries: readonly TranscriptEntry[];
  offline: boolean;
  onBack: () => void;
  controls?: React.ReactNode;
}) {
  const { agent, entries, offline } = props;
  const endRef = React.useRef<HTMLDivElement>(null);
  const stick = React.useRef(true);
  const listRef = React.useRef<HTMLDivElement>(null);
  // biome-ignore lint/correctness/useExhaustiveDependencies: scroll on new entries only
  React.useEffect(() => {
    if (stick.current) endRef.current?.scrollIntoView({ block: "end" });
  }, [entries.length, entries[entries.length - 1]?.text]);
  return (
    <section
      data-testid="agent-detail"
      data-agent={agent.pubkey}
      style={{ padding: 16 }}
    >
      <button
        type="button"
        data-testid="agent-detail-back"
        onClick={props.onBack}
        style={{ fontSize: 12, marginBottom: 8 }}
      >
        ← My agents
      </button>
      <h2 style={{ fontSize: 16, fontWeight: 600, margin: 0 }}>{agent.name}</h2>
      <p
        data-testid="agent-detail-status"
        style={{ fontSize: 12, opacity: 0.75, margin: "4px 0 12px" }}
      >
        {agent.status === "running" ? "● running" : agent.status.replace("_", " ")}
        {agent.currentTask ? ` · ${agent.currentTask}` : ""}
        {offline ? " · desktop offline (showing last known)" : ""}
      </p>
      {props.controls}
      <h3 style={{ fontSize: 13, fontWeight: 600, margin: "12px 0 6px" }}>
        Recent activity
      </h3>
      <div
        ref={listRef}
        data-testid="agent-transcript"
        onScroll={(e) => {
          const el = e.currentTarget;
          stick.current = el.scrollHeight - el.scrollTop - el.clientHeight < 40;
        }}
        style={{
          maxHeight: 420,
          overflowY: "auto",
          display: "grid",
          gap: 6,
          opacity: offline ? 0.6 : 1,
        }}
      >
        {entries.length === 0 && (
          <p data-testid="agent-transcript-empty" style={{ fontSize: 12, opacity: 0.7 }}>
            No recent activity yet. Activity appears here while the agent is running on your desktop.
          </p>
        )}
        {entries.map((e) => (
          <div
            key={e.id}
            data-testid="transcript-entry"
            data-role={e.role}
            style={{
              fontSize: 12,
              borderLeft: "2px solid rgba(128,128,128,.4)",
              paddingLeft: 8,
              whiteSpace: "pre-wrap",
              wordBreak: "break-word",
              opacity: e.role === "thought" ? 0.65 : 1,
            }}
          >
            <span style={{ opacity: 0.6 }}>
              {clock(e.t)} {e.role === "tool" ? `Tool · ${e.tool?.name ?? ""}` : (ROLE_LABEL[e.role] ?? e.role)}
              {e.tool ? ` (${e.tool.status}${e.tool.error ? ", error" : ""})` : ""}
            </span>
            {e.text ? `\n${e.text}` : ""}
          </div>
        ))}
        <div ref={endRef} />
      </div>
    </section>
  );
}
