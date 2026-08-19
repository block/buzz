import { Info } from "lucide-react";
import * as React from "react";

import { Button } from "@/shared/ui/button";

/**
 * Pre-send disclosure of the exact context payload appended to an outgoing
 * agent message. Showing the payload only in the sent message afterwards is
 * not a trust boundary — the payload embeds relay/git-controlled metadata
 * (names, titles, branches, paths) that an attacker can shape, and the agent
 * may act on it before the retrospective disclosure is even seen. Callers
 * must pass the same string they append at send time so what the user
 * inspects here is byte-identical to what gets signed under their key.
 */
export function AgentContextPayloadPreview({
  payload,
  triggerLabel,
}: {
  payload: string;
  triggerLabel: string;
}) {
  const [open, setOpen] = React.useState(false);
  const trimmed = payload.trim();
  if (!trimmed) return null;
  return (
    <div className="relative shrink-0">
      <Button
        aria-expanded={open}
        className="h-7 gap-1 rounded-full px-2 text-xs text-muted-foreground"
        data-testid="agent-context-preview-trigger"
        onClick={() => setOpen((value) => !value)}
        size="sm"
        title="Preview the exact context appended to your message"
        type="button"
        variant="ghost"
      >
        <Info className="h-3.5 w-3.5" />
        {triggerLabel}
      </Button>
      {open ? (
        <div
          className="absolute bottom-full right-0 z-50 mb-1 w-80 rounded-lg border border-border bg-popover p-3 shadow-md"
          data-testid="agent-context-preview"
        >
          <p className="mb-2 text-xs text-muted-foreground">
            This exact text is appended to your message before it is signed and
            sent. Quoted values are untrusted workspace metadata — Buzz does not
            verify or rewrite them.
          </p>
          <pre
            className="max-h-48 overflow-auto whitespace-pre-wrap break-words font-mono text-xs text-foreground"
            data-testid="agent-context-preview-payload"
          >
            {trimmed}
          </pre>
        </div>
      ) : null}
    </div>
  );
}
