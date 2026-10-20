import * as React from "react";
import { SquareCheck } from "lucide-react";

import { Alert, AlertDescription } from "@/shared/ui/alert";
import { Button } from "@/shared/ui/button";
import { useOpenEntityLink } from "@/shared/ui/markdown/entityLinks";
import { Skeleton } from "@/shared/ui/skeleton";
import {
  CATCH_UP_SECTION_LABELS,
  describeCatchUpCoverage,
  describeCatchUpRange,
  groupCatchUpPoints,
  isCatchUpSourceChangedError,
} from "./threadCatchUp";
import {
  ThreadCatchUpSources,
  useThreadMessagesById,
} from "./ThreadCatchUpSources";
import type { CatchUpPhase, ThreadCatchUpController } from "./useThreadCatchUp";
import { useThreadCatchUpAssignments } from "./useThreadCatchUpAssignments";

type ReadyPhase = Extract<CatchUpPhase, { kind: "ready" }>;
type Controller = NonNullable<ThreadCatchUpController>;

/** A private, generated recap. Model text renders as plain text only. */
export function ThreadCatchUpRecapView({
  controller,
  currentPubkey,
  phase,
}: {
  controller: Controller;
  currentPubkey: string;
  phase: ReadyPhase;
}) {
  const { recap, choice, citedBodies } = phase;
  const messagesById = useThreadMessagesById(controller.threadMessages);
  const groups = groupCatchUpPoints(recap.points);
  const sourceProps = {
    bodies: controller.bodies,
    channelId: controller.scope.channelId,
    citedBodies,
    messagesById,
    rootEventId: controller.scope.rootEventId,
  };

  return (
    <div className="space-y-3" data-testid="thread-catch-up-recap">
      <div className="space-y-0.5 text-2xs text-muted-foreground">
        <p>{describeCatchUpRange(choice, recap)}</p>
        <p data-testid="thread-catch-up-coverage">
          {describeCatchUpCoverage(recap.coverage)}
        </p>
        <p>
          Generated on this computer. Check the sources before relying on it.
        </p>
      </div>
      {recap.coverage.warnings.length > 0 ? (
        <Alert data-testid="thread-catch-up-warnings">
          <AlertDescription>
            <ul className="list-disc space-y-0.5 pl-4">
              {recap.coverage.warnings.map((warning, index) => (
                // biome-ignore lint/suspicious/noArrayIndexKey: warnings are an immutable list
                <li key={index}>{warning}</li>
              ))}
            </ul>
          </AlertDescription>
        </Alert>
      ) : null}

      {recap.points.length === 0 ? (
        <p
          className="text-sm text-muted-foreground"
          data-testid="thread-catch-up-empty"
        >
          No decisions or open questions in this period.
        </p>
      ) : (
        (["decision", "question"] as const).map((category) =>
          groups[category].length > 0 ? (
            <section
              aria-labelledby={`thread-catch-up-${category}`}
              data-testid={`thread-catch-up-section-${category}`}
              key={category}
            >
              <h4
                className="mb-1 text-xs font-medium text-muted-foreground"
                id={`thread-catch-up-${category}`}
              >
                {CATCH_UP_SECTION_LABELS[category]}
              </h4>
              <ul className="space-y-2">
                {groups[category].map((point) => (
                  <li data-point-id={point.id} key={point.id}>
                    <p
                      className="whitespace-pre-wrap break-words text-sm leading-5"
                      data-testid="thread-catch-up-point"
                    >
                      {point.text}
                    </p>
                    <ThreadCatchUpSources
                      {...sourceProps}
                      sourceIds={point.sourceIds}
                    />
                  </li>
                ))}
              </ul>
            </section>
          ) : null,
        )
      )}

      <AssignedToYou
        bodies={controller.bodies}
        currentPubkey={currentPubkey}
        recap={recap}
        sourceProps={sourceProps}
      />

      <ShareRecap controller={controller} phase={phase} />
    </div>
  );
}

function AssignedToYou({
  bodies,
  currentPubkey,
  recap,
  sourceProps,
}: {
  bodies: ReadonlyMap<string, string>;
  currentPubkey: string;
  recap: ReadyPhase["recap"];
  sourceProps: Omit<
    React.ComponentProps<typeof ThreadCatchUpSources>,
    "sourceIds"
  >;
}) {
  const assignments = useThreadCatchUpAssignments({
    bodies,
    currentPubkey,
    recap,
  });
  const openEntityLink = useOpenEntityLink();
  if (assignments.status === "none") return null;
  return (
    <section
      aria-labelledby="thread-catch-up-assigned"
      data-testid="thread-catch-up-assigned"
    >
      <h4
        className="mb-1 text-xs font-medium text-muted-foreground"
        id="thread-catch-up-assigned"
      >
        Assigned to you
      </h4>
      {assignments.status === "loading" ? (
        <Skeleton
          aria-label="Checking linked tasks"
          className="h-4 w-1/2"
          role="status"
        />
      ) : assignments.status === "error" ? (
        <p className="text-xs text-muted-foreground">
          Linked tasks couldn't be checked right now.
        </p>
      ) : (
        <>
          {assignments.tasks.length === 0 ? (
            <p className="text-xs text-muted-foreground">
              None of the tasks linked in this thread are assigned to you.
            </p>
          ) : (
            <ul className="space-y-2">
              {assignments.tasks.map(({ issue, reference }) => (
                <li key={reference.issueId}>
                  <button
                    className="inline-flex max-w-full items-center gap-1.5 rounded-md text-left text-sm leading-5 hover:underline focus-visible:outline-hidden focus-visible:ring-1 focus-visible:ring-ring"
                    data-testid="thread-catch-up-task"
                    onClick={() =>
                      openEntityLink({
                        type: "issue",
                        id: reference.issueId,
                        owner: reference.owner,
                        dtag: reference.dtag,
                      })
                    }
                    type="button"
                  >
                    <SquareCheck
                      aria-hidden
                      className="size-4 shrink-0 text-muted-foreground"
                    />
                    <span className="min-w-0 truncate">{issue.title}</span>
                    <span className="shrink-0 text-2xs text-muted-foreground">
                      {issue.status}
                    </span>
                  </button>
                  <ThreadCatchUpSources
                    {...sourceProps}
                    sourceIds={reference.sourceIds}
                  />
                </li>
              ))}
            </ul>
          )}
          {assignments.unverifiedCount > 0 ? (
            <p className="mt-1 text-2xs text-muted-foreground">
              {assignments.unverifiedCount} linked{" "}
              {assignments.unverifiedCount === 1 ? "task" : "tasks"} couldn't be
              verified and {assignments.unverifiedCount === 1 ? "is" : "are"}{" "}
              not shown.
            </p>
          ) : null}
        </>
      )}
    </section>
  );
}

function ShareRecap({
  controller,
  phase,
}: {
  controller: Controller;
  phase: ReadyPhase;
}) {
  const [confirming, setConfirming] = React.useState(false);
  const { share } = controller;
  const forThisRecap = share.variables === phase.recap;
  const shared = forThisRecap && share.isSuccess;
  const canShare = phase.recap.points.length > 0 && !controller.isArchived;

  return (
    <div className="space-y-2 border-t border-border/40 pt-3">
      {shared ? (
        <p
          aria-live="polite"
          className="text-xs text-muted-foreground"
          data-testid="thread-catch-up-shared-notice"
        >
          Shared with this thread.
        </p>
      ) : confirming ? (
        <div className="space-y-2" data-testid="thread-catch-up-share-confirm">
          <p className="text-xs leading-5">
            Thread members will see this recap, its sources, and who shared it,
            and can add corrections. It is not posted as a message.
          </p>
          <div className="flex flex-wrap gap-2">
            <Button
              data-testid="thread-catch-up-share-confirm-button"
              disabled={share.isPending}
              onClick={() =>
                share.mutate(phase.recap, {
                  onSettled: () => setConfirming(false),
                })
              }
              size="sm"
              type="button"
            >
              {share.isPending ? "Sharing…" : "Share"}
            </Button>
            <Button
              onClick={() => setConfirming(false)}
              size="sm"
              type="button"
              variant="ghost"
            >
              Cancel
            </Button>
          </div>
        </div>
      ) : (
        <div className="flex flex-wrap items-center gap-2">
          <Button
            data-testid="thread-catch-up-share"
            disabled={!canShare}
            onClick={() => setConfirming(true)}
            size="sm"
            type="button"
            variant="outline"
          >
            Share with this thread
          </Button>
          <Button
            data-testid="thread-catch-up-new"
            onClick={controller.cancel}
            size="sm"
            type="button"
            variant="ghost"
          >
            New recap
          </Button>
          {controller.isArchived ? (
            <span className="text-2xs text-muted-foreground">
              Archived channels can't receive shared recaps.
            </span>
          ) : null}
        </div>
      )}
      {forThisRecap && share.isError ? (
        isCatchUpSourceChangedError(share.error) ? (
          <Alert data-testid="thread-catch-up-share-source-changed">
            <AlertDescription className="space-y-2">
              <p>
                Not shared. A cited message changed after this recap was made,
                so it may no longer match the thread. Your recap is still here.
              </p>
              <div className="flex flex-wrap gap-2">
                <Button
                  data-testid="thread-catch-up-regenerate"
                  onClick={() => controller.generate(phase.choice)}
                  size="sm"
                  type="button"
                >
                  Regenerate
                </Button>
                <Button
                  data-testid="thread-catch-up-share-retry"
                  disabled={share.isPending}
                  onClick={() => share.mutate(phase.recap)}
                  size="sm"
                  type="button"
                  variant="outline"
                >
                  Try sharing again
                </Button>
              </div>
            </AlertDescription>
          </Alert>
        ) : (
          <Alert variant="destructive">
            <AlertDescription>
              Not shared. {share.error.message}
            </AlertDescription>
          </Alert>
        )
      ) : null}
    </div>
  );
}
