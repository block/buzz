import * as React from "react";
import { ArrowRightLeft, BookOpen, Send } from "lucide-react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";

import { useRelayAgentsQuery } from "@/features/agents/hooks";
import { useUsersBatchQuery } from "@/features/profile/hooks";
import { truncatePubkey } from "@/shared/lib/pubkey";
import type { ManagedAgent } from "@/shared/api/types";
import {
  getAgentHandoff,
  listAgentHandoffs,
  sendAgentHandoff,
  type AgentHandoffRecord,
} from "@/shared/api/handoffs";
import { cn } from "@/shared/lib/cn";
import { Badge } from "@/shared/ui/badge";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";

type AgentHandoffDialogProps = {
  agent: Pick<ManagedAgent, "pubkey" | "name">;
  history: string;
  initialMode?: "send" | "received";
  trigger?: React.ReactNode;
  open?: boolean;
  onOpenChange?: (open: boolean) => void;
};

export function AgentHandoffDialog({
  agent,
  history,
  initialMode = "send",
  trigger,
  open: controlledOpen,
  onOpenChange,
}: AgentHandoffDialogProps) {
  const [internalOpen, setInternalOpen] = React.useState(false);
  const open = controlledOpen ?? internalOpen;
  const setOpen = React.useCallback(
    (nextOpen: boolean) => {
      if (controlledOpen === undefined) {
        setInternalOpen(nextOpen);
      }
      onOpenChange?.(nextOpen);
    },
    [controlledOpen, onOpenChange],
  );
  const [mode, setMode] = React.useState<"send" | "received">(initialMode);
  const [selectedEvent, setSelectedEvent] = React.useState<string | null>(null);
  const [recipient, setRecipient] = React.useState("");
  const [title, setTitle] = React.useState("");
  const [summary, setSummary] = React.useState("");
  const [body, setBody] = React.useState(history);
  const relayAgents = useRelayAgentsQuery({ enabled: open }).data ?? [];
  const ownerPubkeys = React.useMemo(
    () =>
      relayAgents.flatMap((agent) =>
        agent.ownerPubkey ? [agent.ownerPubkey] : [],
      ),
    [relayAgents],
  );
  const ownerProfiles = useUsersBatchQuery(ownerPubkeys, { enabled: open }).data
    ?.profiles;
  const queryClient = useQueryClient();
  const received = useQuery({
    queryKey: ["agent-handoffs"],
    queryFn: () => listAgentHandoffs(),
    enabled: open && mode === "received",
  });
  const detail = useQuery({
    queryKey: ["agent-handoff", selectedEvent],
    queryFn: () => getAgentHandoff(selectedEvent as string),
    enabled: Boolean(selectedEvent),
  });
  const send = useMutation({
    mutationFn: () =>
      sendAgentHandoff({
        recipientPubkey: recipient,
        title,
        summary: summary.trim() || undefined,
        history: body,
      }),
    onSuccess: () => {
      setOpen(false);
      setRecipient("");
      setTitle("");
      setSummary("");
      void queryClient.invalidateQueries({ queryKey: ["agent-handoffs"] });
    },
  });

  function openSend() {
    setMode("send");
    setBody(history);
    setOpen(true);
  }

  function openReceived() {
    setMode("received");
    setSelectedEvent(null);
    setOpen(true);
  }

  function openDialog() {
    if (initialMode === "received") {
      openReceived();
      return;
    }
    openSend();
  }

  const availableAgents = relayAgents.filter(
    (candidate) => candidate.pubkey !== agent.pubkey,
  );

  return (
    <>
      <div className="flex items-center gap-1">
        {trigger ? (
          React.cloneElement(
            trigger as React.ReactElement<{ onSelect?: () => void }>,
            { onSelect: openDialog },
          )
        ) : (
          <>
            <Button
              aria-label={
                initialMode === "received"
                  ? "View agent handoff history"
                  : "Send agent handoff"
              }
              className="h-8 gap-1.5 px-2 text-xs"
              onClick={openDialog}
              size="sm"
              variant="outline"
            >
              {initialMode === "received" ? (
                <BookOpen className="h-3.5 w-3.5" />
              ) : (
                <Send className="h-3.5 w-3.5" />
              )}
              {initialMode === "received" ? "History" : "Handoff"}
            </Button>
            <Button
              aria-label="View received agent handoffs"
              className="h-8 w-8 p-0"
              onClick={openReceived}
              size="sm"
              title="View received handoffs"
              variant="ghost"
            >
              <BookOpen className="h-4 w-4" />
            </Button>
          </>
        )}
      </div>

      <Dialog onOpenChange={setOpen} open={open}>
        <DialogContent className="max-w-3xl overflow-hidden p-0">
          <DialogHeader className="border-b px-6 pb-4 pt-5 pr-14">
            <DialogTitle className="flex items-center gap-2">
              <ArrowRightLeft className="h-4 w-4" />
              Agent handoff
            </DialogTitle>
            <DialogDescription>
              Share a curated task snapshot with another Agent. Hidden reasoning
              and credentials are excluded.
            </DialogDescription>
          </DialogHeader>

          <div className="flex gap-2 border-b px-6 py-3">
            <Button
              onClick={() => setMode("send")}
              size="sm"
              variant={mode === "send" ? "default" : "outline"}
            >
              Send history
            </Button>
            <Button
              onClick={() => setMode("received")}
              size="sm"
              variant={mode === "received" ? "default" : "outline"}
            >
              Received history
            </Button>
          </div>

          {mode === "send" ? (
            <div className="grid min-h-0 gap-4 overflow-y-auto px-6 pb-6 pt-4">
              <label className="grid gap-1.5 text-sm">
                Receiving Agent
                <select
                  className="h-9 rounded-md border bg-background px-3 text-sm"
                  onChange={(event) => setRecipient(event.target.value)}
                  value={recipient}
                >
                  <option value="">Select an Agent</option>
                  {availableAgents.map((candidate) => (
                    <option key={candidate.pubkey} value={candidate.pubkey}>
                      {candidate.name}
                      {candidate.deleted ? " [已删除]" : ""} ·{" "}
                      {candidate.ownerPubkey
                        ? (ownerProfiles?.[candidate.ownerPubkey.toLowerCase()]
                            ?.displayName ??
                          `用户 ${truncatePubkey(candidate.ownerPubkey)}`)
                        : "未知用户"}{" "}
                      · {truncatePubkey(candidate.pubkey)}
                    </option>
                  ))}
                </select>
              </label>
              <label className="grid gap-1.5 text-sm">
                Title
                <input
                  className="h-9 rounded-md border bg-background px-3 text-sm"
                  onChange={(event) => setTitle(event.target.value)}
                  placeholder="Continue file preview work"
                  value={title}
                />
              </label>
              <label className="grid gap-1.5 text-sm">
                Summary
                <input
                  className="h-9 rounded-md border bg-background px-3 text-sm"
                  onChange={(event) => setSummary(event.target.value)}
                  placeholder="What is complete and what remains"
                  value={summary}
                />
              </label>
              <label className="grid gap-1.5 text-sm">
                Handoff history (Markdown)
                <textarea
                  className="min-h-64 resize-y rounded-md border bg-background px-3 py-2 font-mono text-xs"
                  onChange={(event) => setBody(event.target.value)}
                  value={body}
                />
              </label>
              {send.error ? (
                <p className="text-sm text-destructive">
                  {formatHandoffError(send.error)}
                </p>
              ) : null}
              <div className="flex justify-end gap-2">
                <Button onClick={() => setOpen(false)} variant="ghost">
                  Cancel
                </Button>
                <Button
                  disabled={
                    !recipient ||
                    !title.trim() ||
                    !body.trim() ||
                    send.isPending
                  }
                  onClick={() => send.mutate()}
                >
                  <Send className="mr-1.5 h-4 w-4" />
                  {send.isPending ? "Sending…" : "Send handoff"}
                </Button>
              </div>
            </div>
          ) : (
            <div className="grid min-h-0 gap-4 overflow-y-auto px-6 pb-6 pt-4">
              {received.isPending ? (
                <p className="text-sm text-muted-foreground">
                  Loading handoffs…
                </p>
              ) : null}
              {received.error ? (
                <p className="text-sm text-destructive">
                  Unable to load handoffs.
                </p>
              ) : null}
              {!received.isPending &&
              !received.error &&
              (received.data?.length ?? 0) === 0 ? (
                <p className="text-sm text-muted-foreground">
                  No handoffs have been sent to this Agent.
                </p>
              ) : null}
              <div className="grid gap-2">
                {received.data?.map((item) => (
                  <button
                    className={cn(
                      "rounded-md border px-3 py-2 text-left transition-colors hover:bg-muted/60",
                      selectedEvent === item.eventId &&
                        "border-primary bg-muted/50",
                    )}
                    key={item.eventId}
                    onClick={() => setSelectedEvent(item.eventId)}
                    type="button"
                  >
                    <div className="flex items-center justify-between gap-2">
                      <span className="truncate text-sm font-medium">
                        {item.title}
                      </span>
                      <Badge variant="outline">
                        {new Date(item.createdAt * 1000).toLocaleDateString()}
                      </Badge>
                    </div>
                    {item.summary ? (
                      <p className="mt-1 line-clamp-2 text-xs text-muted-foreground">
                        {item.summary}
                      </p>
                    ) : null}
                  </button>
                ))}
              </div>
              {detail.data ? <HandoffDetail record={detail.data} /> : null}
            </div>
          )}
        </DialogContent>
      </Dialog>
    </>
  );
}

function HandoffDetail({ record }: { record: AgentHandoffRecord }) {
  return (
    <section className="grid gap-2 rounded-md border bg-muted/20 p-4">
      <div className="flex items-center justify-between gap-2">
        <h4 className="text-sm font-semibold">{record.title}</h4>
        <span className="text-xs text-muted-foreground">
          from {record.senderPubkey.slice(0, 12)}…
        </span>
      </div>
      {record.summary ? (
        <p className="text-sm text-muted-foreground">{record.summary}</p>
      ) : null}
      <pre className="max-h-96 overflow-auto whitespace-pre-wrap rounded-md bg-background p-3 font-mono text-xs">
        {record.history}
      </pre>
    </section>
  );
}

function formatHandoffError(error: unknown) {
  const message = error instanceof Error ? error.message : String(error);
  return message.includes("unknown event kind")
    ? "当前 relay 尚未支持 Agent handoff（kind 44201），需要先更新 relay 服务。"
    : message;
}
