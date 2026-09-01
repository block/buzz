import { Agentation, type Annotation } from "agentation";
import { AlertCircle, Check, Settings2 } from "lucide-react";
import * as React from "react";
import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import { useRelayAgentsQuery } from "@/features/agents/hooks";
import { useChannelsQuery } from "@/features/channels/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import { useSendMessageMutation } from "@/features/messages/hooks";
import { useIdentityQuery } from "@/shared/api/hooks";
import { createRelayMessageEvent } from "@/shared/api/relayMessageEvent";
import type { RelayEvent } from "@/shared/api/types";
import { Button } from "@/shared/ui/button";
import { AgentationDestinationFields } from "./AgentationDestinationFields";
import {
  formatAgentationMessage,
  resolveAgentationDestination,
} from "./agentationDestination";
import { useAgentationDestination } from "./agentationStore";
import {
  clearRetainedAgentationSubmission,
  readRetainedAgentationSubmission,
  retainAgentationSubmission,
} from "./agentationSubmissionStore";
import {
  agentationScope,
  emitAgentationPendingChange,
  readAgentationAnnotations,
} from "./agentationPendingStore";

type Status =
  | { type: "idle" }
  | { type: "error"; message: string }
  | {
      type: "sent";
      eventId: string;
      channelId: string;
      channelName: string;
    };

export function AgentationExperimentRoot() {
  const identity = useIdentityQuery();
  const { activeCommunity } = useCommunities();
  const storageScope = agentationScope(
    activeCommunity?.relayUrl,
    identity.data?.pubkey,
  );
  return (
    <ScopedAgentationExperimentRoot
      key={storageScope}
      storageScope={storageScope}
    />
  );
}

function ScopedAgentationExperimentRoot({
  storageScope,
}: {
  storageScope: string;
}) {
  const identity = useIdentityQuery();
  const { activeCommunity } = useCommunities();
  const channelsQuery = useChannelsQuery();
  const agentsQuery = useRelayAgentsQuery();
  const [destination, setDestination] = useAgentationDestination(
    activeCommunity?.relayUrl,
    identity.data?.pubkey,
  );
  const [annotations, setAnnotations] = React.useState<Annotation[]>(
    () => readAgentationAnnotations(storageScope) as Annotation[],
  );
  const [pickerOpen, setPickerOpen] = React.useState(false);
  const [status, setStatus] = React.useState<Status>({ type: "idle" });
  const { goChannel } = useAppNavigation();
  const send = useSendMessageMutation(null, identity.data);
  const inFlight = React.useRef<Promise<{
    ok: boolean;
    eventId?: string;
    acceptedAnnotations?: Annotation[];
  }> | null>(null);
  const batchSubmission = React.useRef<{
    fingerprint: string;
    submissionId: string;
    annotations: Annotation[];
    channelId: string;
    agentPubkey: string;
    event: RelayEvent;
  } | null>(readRetainedAgentationSubmission(storageScope));
  React.useEffect(() => {
    emitAgentationPendingChange(annotations.length);
  }, [annotations]);

  const retainedDestination = batchSubmission.current
    ? {
        channelId: batchSubmission.current.channelId,
        agentPubkey: batchSubmission.current.agentPubkey,
      }
    : null;
  const resolved = resolveAgentationDestination(
    retainedDestination ?? destination,
    channelsQuery.data ?? [],
    agentsQuery.data ?? [],
    identity.data?.pubkey,
  );

  const submit = React.useCallback(
    (output: string, batch: Annotation[]) => {
      if (inFlight.current) return inFlight.current;
      const retainedSubmission = batchSubmission.current;
      const submitDestination = retainedSubmission
        ? {
            channelId: retainedSubmission.channelId,
            agentPubkey: retainedSubmission.agentPubkey,
          }
        : destination;
      const current = resolveAgentationDestination(
        submitDestination,
        channelsQuery.data ?? [],
        agentsQuery.data ?? [],
        identity.data?.pubkey,
      );
      if (!current.valid || !current.channel || !current.agent) {
        setPickerOpen(true);
        setStatus({
          type: "error",
          message: "Choose a valid channel and agent before sending.",
        });
        return Promise.resolve({ ok: false });
      }
      const channel = current.channel;
      const agent = current.agent;
      const fingerprint = [
        channel.id,
        agent.pubkey,
        output,
        ...batch.map((annotation) => annotation.id).sort(),
      ].join(":");
      const retained = retainedSubmission;
      const submissionId = retained?.submissionId ?? crypto.randomUUID();
      const content =
        retained?.event.content ??
        formatAgentationMessage(agent.name, output, submissionId);
      const prepare = retained
        ? Promise.resolve(retained.event)
        : createRelayMessageEvent(channel.id, content, [agent.pubkey]).then(
            (event) => {
              const submission = {
                fingerprint,
                submissionId,
                annotations: batch,
                channelId: channel.id,
                agentPubkey: agent.pubkey,
                event,
              };
              batchSubmission.current = submission;
              retainAgentationSubmission(storageScope, submission);
              return event;
            },
          );
      const promise = prepare
        .then((signedEvent) =>
          send.mutateAsync({
            targetChannel: channel,
            content,
            mentionPubkeys: [agent.pubkey],
            signedEvent,
          }),
        )
        .then((event) => {
          setStatus({
            type: "sent",
            eventId: event.id,
            channelId: channel.id,
            channelName: channel.name,
          });
          batchSubmission.current = null;
          clearRetainedAgentationSubmission(storageScope);
          const acceptedAnnotations = retained?.annotations ?? batch;
          setAnnotations((existing) =>
            existing.filter(
              (annotation) =>
                !acceptedAnnotations.some(
                  (accepted) =>
                    accepted.id === annotation.id &&
                    JSON.stringify(accepted) === JSON.stringify(annotation),
                ),
            ),
          );
          return {
            ok: true,
            eventId: event.id,
            acceptedAnnotations,
          };
        })
        .catch((error: unknown) => {
          setStatus({
            type: "error",
            message:
              error instanceof Error
                ? error.message
                : "The annotations could not be sent.",
          });
          return { ok: false };
        })
        .finally(() => {
          inFlight.current = null;
        });
      inFlight.current = promise;
      return promise;
    },
    [
      agentsQuery.data,
      channelsQuery.data,
      destination,
      identity.data,
      send,
      storageScope,
    ],
  );

  return (
    <>
      <Agentation
        copyToClipboard={false}
        key={storageScope}
        storageScope={storageScope}
        onAnnotationAdd={(annotation) =>
          setAnnotations((current) => [...current, annotation])
        }
        onAnnotationDelete={(annotation) =>
          setAnnotations((current) =>
            current.filter((candidate) => candidate.id !== annotation.id),
          )
        }
        onAnnotationUpdate={(annotation) =>
          setAnnotations((current) =>
            current.map((candidate) =>
              candidate.id === annotation.id ? annotation : candidate,
            ),
          )
        }
        onAnnotationsClear={(cleared) => {
          setAnnotations((current) =>
            current.filter(
              (annotation) =>
                !cleared.some(
                  (accepted) =>
                    accepted.id === annotation.id &&
                    JSON.stringify(accepted) === JSON.stringify(annotation),
                ),
            ),
          );
        }}
        onSubmit={submit}
        submitEnabled={resolved.valid}
      />
      <aside
        className="fixed bottom-20 right-4 z-2147483646 w-[min(25rem,calc(100vw-2rem))] rounded-xl border border-border/70 bg-background/95 p-3 shadow-xl backdrop-blur"
        data-agentation-buzz-shell
      >
        <div className="flex items-center justify-between gap-3">
          <div className="min-w-0">
            <p className="truncate text-sm font-medium">
              {resolved.valid
                ? `#${resolved.channel?.name} → ${resolved.agent?.name}`
                : "Choose a channel and agent to send"}
            </p>
            <p className="text-xs text-muted-foreground">
              {annotations.length} pending annotation
              {annotations.length === 1 ? "" : "s"}
            </p>
          </div>
          <Button
            aria-expanded={pickerOpen}
            aria-label="Choose Agentation destination"
            onClick={() => setPickerOpen((open) => !open)}
            size="icon"
            variant="outline"
          >
            <Settings2 />
          </Button>
        </div>
        {pickerOpen ? (
          <div className="mt-3 border-t border-border/60 pt-3">
            <AgentationDestinationFields
              agents={agentsQuery.data ?? []}
              channels={channelsQuery.data ?? []}
              currentPubkey={identity.data?.pubkey}
              destination={destination}
              onChange={setDestination}
            />
          </div>
        ) : null}
        {status.type !== "idle" ? (
          <p
            aria-live="polite"
            className={`mt-2 flex items-start gap-1.5 text-xs ${status.type === "error" ? "text-destructive" : "text-emerald-600"}`}
          >
            {status.type === "error" ? <AlertCircle /> : <Check />}
            {status.type === "error" ? (
              status.message
            ) : (
              <>
                Sent to #{status.channelName}.{" "}
                <button
                  className="underline underline-offset-2"
                  onClick={() =>
                    void goChannel(status.channelId, {
                      messageId: status.eventId,
                    })
                  }
                  type="button"
                >
                  View message
                </button>
              </>
            )}
          </p>
        ) : null}
      </aside>
    </>
  );
}
