import { useQueryClient } from "@tanstack/react-query";
import { useRef, useState } from "react";
import { useManagedAgentsQuery } from "@/features/agents/hooks";
import {
  assignmentCanvas,
  taskAssignee,
  type TaskAssignee,
} from "../lib/taskAssignment";
import { invokeTauri } from "@/shared/api/tauri";

export function AssignTask({
  channelId,
  content,
}: {
  channelId: string;
  content: string;
}) {
  const [choosing, setChoosing] = useState(false);
  const [pending, setPending] = useState(false);
  const [error, setError] = useState("");
  const busy = useRef(false);
  const client = useQueryClient();
  const agents = useManagedAgentsQuery();
  let assignee: TaskAssignee | null;
  try {
    assignee = taskAssignee(content);
  } catch (error) {
    return (
      <span role="status" className="text-xs text-destructive">
        {String(error)}
      </span>
    );
  }

  async function assign(pubkey: string) {
    if (busy.current) return;
    busy.current = true;
    setPending(true);
    setError("");
    try {
      const operationId =
        assignee?.pubkey === pubkey &&
        assignee.notification?.status === "pending"
          ? assignee.notification.id
          : crypto.randomUUID();
      const result = await invokeTauri<{
        assigned: boolean;
        notified: boolean;
        error?: string;
      }>("assign_task_channel", {
        channelId,
        assigneePubkey: pubkey,
        expectedCanvas: content,
        canvasContent: assignmentCanvas(
          content,
          pubkey,
          operationId,
          "pending",
        ),
        completedCanvas: assignmentCanvas(content, pubkey, operationId, "sent"),
        operationId,
      });
      if (!result.notified)
        setError(result.error ?? "Assignment saved; notification failed");
      setChoosing(false);
    } catch (error) {
      setError(String(error));
    } finally {
      await client.invalidateQueries({
        queryKey: ["channel-canvas", channelId],
      });
      busy.current = false;
      setPending(false);
    }
  }

  return (
    <div className="ml-auto flex items-center gap-2 text-xs">
      {assignee && !choosing ? (
        <>
          <button
            type="button"
            title="Change assignee"
            aria-label="Change assignee"
            disabled={pending}
            onClick={() => setChoosing(true)}
            className="rounded px-2 py-1 hover:bg-accent disabled:opacity-50"
          >
            Assigned to{" "}
            {agents.data?.find((agent) => agent.pubkey === assignee.pubkey)
              ?.name ?? "agent"}
          </button>
          {assignee.notification?.status === "pending" && (
            <button
              type="button"
              disabled={pending}
              className="hover:underline disabled:opacity-50"
              onClick={() => void assign(assignee.pubkey)}
            >
              {pending ? "Notifying…" : "Retry notification"}
            </button>
          )}
        </>
      ) : choosing ? (
        <>
          {assignee && (
            <span className="text-muted-foreground">
              Reassigning does not stop the previous agent.
            </span>
          )}
          <select
            aria-label="Assign task to agent"
            disabled={pending}
            value=""
            onChange={(event) => void assign(event.target.value)}
            className="max-w-48 rounded border bg-background p-1"
          >
            <option value="">
              {agents.isLoading ? "Loading agents…" : "Choose agent…"}
            </option>
            {agents.data?.map((agent) => (
              <option
                key={agent.pubkey}
                value={agent.pubkey}
                disabled={agent.pubkey === assignee?.pubkey}
              >
                {agent.name}
              </option>
            ))}
          </select>
          <button
            type="button"
            disabled={pending}
            onClick={() => setChoosing(false)}
          >
            Cancel
          </button>
          {agents.error && <span role="status">Could not load agents</span>}
        </>
      ) : (
        <button
          type="button"
          className="rounded border px-2 py-1 hover:bg-accent"
          onClick={() => setChoosing(true)}
        >
          Assign
        </button>
      )}
      {error && (
        <span role="alert" className="max-w-80 text-destructive">
          {error}
        </span>
      )}
    </div>
  );
}
