import { subscribeControlResults } from "@/features/agents/observerRelayStore";
import { sendAgentObserverControl } from "@/shared/api/observerRelay";
import type { RelayEvent } from "@/shared/api/types";

const OWNER_CONTROL_TIMEOUT_MS = 20_000;

export type ProjectOwnerAnnouncementTemplate = {
  kind: number;
  content: string;
  createdAt?: number;
  tags: string[][];
};

type ProjectOwnerControlResult = {
  type: "publish_project_owner_announcements";
  status: string;
  requestId: string;
  events?: RelayEvent[];
  error?: string | null;
};

/** Ask a remotely managed agent to publish project events under its own key. */
export function publishOwnedAgentProjectAnnouncements(
  agentPubkey: string,
  announcements: ProjectOwnerAnnouncementTemplate[],
): Promise<RelayEvent[]> {
  const requestId = crypto.randomUUID();

  return new Promise((resolve, reject) => {
    let settled = false;
    const finish = (
      result: { events: RelayEvent[] } | { error: Error },
    ): void => {
      if (settled) return;
      settled = true;
      window.clearTimeout(timeout);
      unsubscribe();
      if ("error" in result) reject(result.error);
      else resolve(result.events);
    };
    const unsubscribe = subscribeControlResults(agentPubkey, (frame) => {
      const projectFrame = frame as unknown as ProjectOwnerControlResult;
      if (
        projectFrame.type !== "publish_project_owner_announcements" ||
        projectFrame.requestId !== requestId
      ) {
        return;
      }
      if (projectFrame.status === "ok" && projectFrame.events) {
        finish({ events: projectFrame.events });
      } else {
        finish({
          error: new Error(
            projectFrame.error || "The agent could not update this project.",
          ),
        });
      }
    });
    const timeout = window.setTimeout(() => {
      finish({
        error: new Error(
          "The project owner agent did not respond. Make sure it is running and try again.",
        ),
      });
    }, OWNER_CONTROL_TIMEOUT_MS);

    void sendAgentObserverControl(agentPubkey, {
      type: "publish_project_owner_announcements",
      requestId,
      announcements,
    }).catch((error: unknown) => {
      finish({
        error:
          error instanceof Error
            ? error
            : new Error("Failed to contact the project owner agent."),
      });
    });
  });
}
