import { useCodexTaskHistoryQuery } from "@/features/agents/hooks";
import { cn } from "@/shared/lib/cn";
import { Markdown } from "@/shared/ui/markdown";

function formatTimestamp(timestamp: string | null) {
  if (!timestamp) return null;
  const date = new Date(timestamp);
  if (Number.isNaN(date.getTime())) return null;
  return new Intl.DateTimeFormat(undefined, {
    dateStyle: "medium",
    timeStyle: "short",
  }).format(date);
}

export function CodexTaskHistoryFocusedView({
  agentPubkey,
}: {
  agentPubkey: string;
}) {
  const historyQuery = useCodexTaskHistoryQuery(agentPubkey);

  if (historyQuery.isLoading) {
    return (
      <div className="px-4 py-8 text-center text-sm text-muted-foreground">
        Loading history...
      </div>
    );
  }

  if (historyQuery.error) {
    return (
      <div className="px-4 py-8 text-center text-sm text-destructive">
        {historyQuery.error instanceof Error
          ? historyQuery.error.message
          : "Could not load Codex history."}
      </div>
    );
  }

  const history = historyQuery.data;
  if (!history || history.messages.length === 0) {
    return (
      <div className="px-4 py-8 text-center text-sm text-muted-foreground">
        No conversation history.
      </div>
    );
  }

  return (
    <div data-testid="codex-task-history">
      {history.truncated ? (
        <div className="border-b border-border/55 px-4 py-2 text-xs text-muted-foreground">
          Showing the latest 200 messages
        </div>
      ) : null}
      <div className="divide-y divide-border/55">
        {history.messages.map((message) => {
          const timestamp = formatTimestamp(message.timestamp);
          return (
            <article
              className={cn(
                "px-4 py-4",
                message.role === "user" && "bg-muted/25",
              )}
              data-testid={`codex-history-${message.role}`}
              key={message.id}
            >
              <div className="mb-2 flex items-center justify-between gap-3">
                <span className="text-xs font-medium text-foreground">
                  {message.role === "user" ? "You" : "Codex"}
                </span>
                {timestamp ? (
                  <time className="text-2xs text-muted-foreground">
                    {timestamp}
                  </time>
                ) : null}
              </div>
              <Markdown
                className="text-sm leading-6"
                content={message.content}
                interactive={false}
              />
            </article>
          );
        })}
      </div>
    </div>
  );
}
