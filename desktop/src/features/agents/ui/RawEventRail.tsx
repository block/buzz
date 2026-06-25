import * as React from "react";

import { Button } from "@/shared/ui/button";
import type { ObserverEvent } from "./agentSessionTypes";
import { describeRawEvent } from "./agentSessionTranscript";

export function RawEventRail({ events }: { events: ObserverEvent[] }) {
  const [expanded, setExpanded] = React.useState(false);
  const visible = expanded ? events : events.slice(-18);

  return (
    <aside className="rounded-lg border border-border/70 bg-[#17171d] text-zinc-100">
      <div className="flex items-center justify-between border-b border-white/10 px-3 py-2">
        <div>
          <p className="text-2xs font-semibold uppercase tracking-[0.16em] text-zinc-400">
            Raw ACP
          </p>
          <p className="text-xs text-zinc-500">JSON-RPC payloads</p>
        </div>
        <Button
          className="h-7 text-zinc-300 hover:text-zinc-50"
          onClick={() => setExpanded((current) => !current)}
          size="sm"
          variant="ghost"
        >
          {expanded ? "Latest" : "All"}
        </Button>
      </div>
      <div className="max-h-[34rem] overflow-auto px-3 py-3">
        {visible.length === 0 ? (
          <p className="text-xs text-zinc-500">No raw events yet.</p>
        ) : (
          <div className="space-y-2">
            {visible.map((event) => (
              <details
                className="rounded-md border border-white/10 bg-white/[0.03] px-2 py-1.5"
                key={event.seq}
              >
                <summary className="cursor-pointer select-none text-xs text-zinc-300">
                  <span className="font-mono text-zinc-500">#{event.seq}</span>{" "}
                  {describeRawEvent(event)}
                </summary>
                <pre className="mt-2 max-h-72 overflow-auto whitespace-pre-wrap break-words text-2xs leading-5 text-zinc-300">
                  {JSON.stringify(event.payload, null, 2)}
                </pre>
              </details>
            ))}
          </div>
        )}
      </div>
    </aside>
  );
}
