import { subscribeControlResults } from "@/features/agents/observerRelayStore";
import { sendAgentObserverControl } from "@/shared/api/observerRelay";
import type { RelayEvent } from "@/shared/api/types";
import {
  KIND_PROJECT_ANNOUNCEMENT,
  KIND_REPO_ANNOUNCEMENT,
} from "@/shared/constants/kinds";

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

/**
 * A remote-agent publish that failed after some announcements already landed.
 * `publishedEvents` holds the events the ACP side reported as live before the
 * failure, so callers can tell a clean failure (retry republishes everything)
 * from a partial one (retry must resume from where publication stopped).
 */
export class PartialAnnouncementPublishError extends Error {
  readonly publishedEvents: RelayEvent[];

  constructor(message: string, publishedEvents: RelayEvent[]) {
    super(message);
    this.name = "PartialAnnouncementPublishError";
    this.publishedEvents = publishedEvents;
  }
}

/**
 * True when a failed [project, repository] announcement publish stopped
 * exactly between its two events — the project head landed, the repository
 * event did not. That is the only partial state addRepo can resume from by
 * republishing just the repository event; anything else must surface.
 */
export function isDanglingProjectMemberPublish(
  error: unknown,
): error is PartialAnnouncementPublishError {
  return (
    error instanceof PartialAnnouncementPublishError &&
    error.publishedEvents.some(
      (event) => event.kind === KIND_PROJECT_ANNOUNCEMENT,
    ) &&
    !error.publishedEvents.some(
      (event) => event.kind === KIND_REPO_ANNOUNCEMENT,
    )
  );
}

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
        const message =
          projectFrame.error || "The agent could not update this project.";
        // The ACP side publishes announcements sequentially and reports the
        // ones that were already live when a later one failed. Preserve that
        // partial-success metadata instead of discarding it, so callers can
        // resume publication rather than treating the state as unrecoverable.
        const published = projectFrame.events ?? [];
        finish({
          error:
            published.length > 0
              ? new PartialAnnouncementPublishError(message, published)
              : new Error(message),
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
