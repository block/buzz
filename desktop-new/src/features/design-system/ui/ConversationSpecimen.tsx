import { PreparedMessagePreview } from "./PreparedMessagePreview";
import { MessageComposer } from "@/features/composer/ui/MessageComposer";
import {
  IconCheck,
  IconCopy,
  IconMessageCircle,
  IconThumbUp,
} from "@tabler/icons-react";
import { useState } from "react";
import { AgentWorkArea } from "@/features/agent-activity/ui/AgentWorkArea";
import { WorkStep } from "@/features/agent-activity/ui/WorkStep";
import type { AgentWorkPresentation } from "@/features/agent-activity/ui/workPresentation";
import { ConversationMessage } from "@/features/conversation/ui/ConversationMessage";
import { ConversationState } from "@/features/conversation/ui/ConversationState";
import { SessionExchange } from "@/features/conversation/ui/SessionExchange";
import { Button } from "@/shared/ui/Button";
import { ConversationHeader } from "@/shared/ui/ConversationHeader";
import { IconButton } from "@/shared/ui/IconButton";
import { Select } from "@/shared/ui/Select";
import {
  RIVET_REPLY,
  RIVET_WORK,
  SESSION_PROMPT,
  VOGUE_REPLY,
  VOGUE_WORK,
  completedWork,
} from "./conversationFixtures";
import "./conversationSpecimens.css";

type Scenario =
  | "working"
  | "one-finished"
  | "finished"
  | "approval"
  | "failed"
  | "waiting"
  | "cancelled"
  | "unavailable";
const SCENARIOS = [
  {
    label: "Session states",
    options: [
      { value: "working", label: "Two agents working" },
      { value: "one-finished", label: "One agent finished" },
      { value: "finished", label: "Both finished" },
      { value: "approval", label: "Permission needed" },
      { value: "failed", label: "Work failed" },
      { value: "waiting", label: "Waiting to start" },
      { value: "cancelled", label: "Stopped" },
      { value: "unavailable", label: "Activity unavailable" },
    ],
  },
];

/** Fixtures alone own these transitions. They never invoke or imitate a runtime. */
function useWorkExample() {
  const [scenario, setScenario] = useState<Scenario>("working");
  const [decision, setDecision] = useState<string | null>(null);
  function select(value: string) {
    if (SCENARIOS[0].options.some((option) => option.value === value)) {
      setScenario(value as Scenario);
      setDecision(null);
    }
  }
  let vogue = VOGUE_WORK;
  let rivet: AgentWorkPresentation = RIVET_WORK;
  if (["one-finished", "finished"].includes(scenario))
    vogue = completedWork(VOGUE_WORK);
  if (scenario === "finished") rivet = completedWork(RIVET_WORK);
  if (scenario === "approval")
    rivet = {
      ...RIVET_WORK,
      status:
        decision === "Allowed in this preview"
          ? "running"
          : decision
            ? "cancelled"
            : "needs-you",
      statusLabel:
        decision === "Allowed in this preview"
          ? "Working"
          : decision
            ? "Stopped"
            : "Needs you",
      notice: {
        title: decision ?? "Allow Rivet to run the checks?",
        description:
          "Run the local test suite for this Session. This example does not execute commands.",
        actions: decision ? undefined : (
          <>
            <Button
              size="compact"
              onClick={() => setDecision("Allowed in this preview")}
            >
              Allow once
            </Button>
            <Button
              size="compact"
              variant="ghost"
              onClick={() => setDecision("Declined in this preview")}
            >
              Decline
            </Button>
          </>
        ),
      },
    };
  if (scenario === "failed")
    rivet = {
      ...RIVET_WORK,
      status: "failed",
      statusLabel: "Failed",
      notice: {
        title: "The checks couldn’t finish",
        description:
          "The working folder was unavailable. The conversation and earlier steps are still here.",
        actions: (
          <Button size="compact" onClick={() => select("working")}>
            Retry in preview
          </Button>
        ),
      },
    };
  if (scenario === "waiting")
    rivet = {
      ...RIVET_WORK,
      status: "waiting",
      statusLabel: "Waiting",
      summary: "Waiting for the agent to start. No activity yet.",
      earlierSteps: [],
      visibleSteps: [],
    };
  if (scenario === "cancelled")
    rivet = {
      ...RIVET_WORK,
      status: "cancelled",
      statusLabel: "Stopped",
      notice: {
        title: "You stopped this work",
        description:
          "Completed steps remain available. No final answer was posted.",
      },
    };
  if (scenario === "unavailable")
    rivet = {
      ...RIVET_WORK,
      status: "unavailable",
      statusLabel: "Unavailable",
      notice: {
        title: "Live activity is unavailable",
        description:
          "The agent may still be working. This is not a completion signal.",
        actions: (
          <Button size="compact" onClick={() => select("working")}>
            Reconnect in preview
          </Button>
        ),
      },
    };
  if (rivet.status !== "running" && rivet.status !== "completed") {
    rivet = {
      ...rivet,
      visibleSteps: rivet.visibleSteps.filter(
        (step) => step.status !== "running",
      ),
    };
  }
  const replies =
    scenario === "finished"
      ? [VOGUE_REPLY, RIVET_REPLY]
      : scenario === "one-finished"
        ? [VOGUE_REPLY]
        : [];
  return { scenario, select, work: [vogue, rivet], replies };
}

export function ConversationSpecimen() {
  const example = useWorkExample();
  const [draft, setDraft] = useState("");
  const [sent, setSent] = useState<{ id: string; content: string }[]>([]);
  return (
    <div className="component-specimen-stack">
      <div className="conversation-preview-controls">
        <Select
          label="Preview"
          value={example.scenario}
          groups={SCENARIOS}
          onValueChange={example.select}
        />
        <span className="text-body-sm text-tertiary">
          Illustrative content · no live agents
        </span>
      </div>
      <section className="conversation-preview" aria-label="Session playground">
        <ConversationHeader
          title="Make room for the work"
          icon={<IconMessageCircle size={18} />}
          context="Session in #design"
        />
        <div className="conversation-preview-transcript">
          <p className="conversation-date text-body-sm text-tertiary">
            Today · September 8
          </p>
          <SessionExchange
            prompt={SESSION_PROMPT}
            work={example.work}
            replies={example.replies}
          />
          {sent.map(({ content, id }) => (
            <ConversationMessage
              key={id}
              message={{
                ...SESSION_PROMPT,
                id,
                content: <PreparedMessagePreview content={content} />,
                delivery: { label: "Sent in this preview only" },
              }}
            />
          ))}
        </div>
        <div className="conversation-preview-footer">
          <MessageComposer
            autoFocus={false}
            draft={draft}
            onDraftChange={setDraft}
            onSend={async (content) => {
              setSent((current) => [
                ...current,
                { id: crypto.randomUUID(), content },
              ]);
            }}
            placeholder="Reply in this session"
          />
          <p className="text-body-sm text-tertiary">
            Local preview only. Messages are not saved or sent to agents.
          </p>
        </div>
      </section>
      <section className="component-specimen-group">
        <h2 className="text-heading text-primary">Built from product pieces</h2>
        <p className="text-body text-secondary">
          Messages carry the shared conversation. Agent work keeps a stable
          place under the prompt. Replies arrive below it without collapsing.
          Each piece has its own page in Product UI.
        </p>
      </section>
    </div>
  );
}

export function AgentWorkSpecimen() {
  const example = useWorkExample();
  return (
    <div className="component-specimen-stack">
      <Select
        label="Preview"
        value={example.scenario}
        groups={SCENARIOS}
        onValueChange={example.select}
      />
      <div className="work-area-specimen">
        <AgentWorkArea work={example.work[1]} />
      </div>
      <p className="text-body-sm text-secondary">
        Earlier steps and tool output open independently. Notices and artifact
        access remain outside the completed-work disclosure.
      </p>
    </div>
  );
}

export function WorkStepSpecimen() {
  return (
    <div className="component-specimen-stack">
      <ol className="work-steps">
        {[
          ...RIVET_WORK.visibleSteps,
          {
            id: "failed-step",
            title: "Couldn’t read the working folder",
            kind: "tool" as const,
            status: "failed" as const,
            detail: "Working folder unavailable.\nNo files were changed.",
          },
        ].map((step) => (
          <WorkStep key={step.id} step={step} />
        ))}
      </ol>
      <p className="text-body-sm text-secondary">
        Progress, thought summary, tool output, running, and failed. Content is
        prepared by Agent Activity; these rows do not parse tool events.
      </p>
    </div>
  );
}

export function ConversationMessageSpecimen() {
  const [liked, setLiked] = useState(false);
  const [copied, setCopied] = useState(false);
  const [copyError, setCopyError] = useState(false);
  const [failed, setFailed] = useState(true);
  const [reply, setReply] = useState(false);
  return (
    <div className="component-specimen-stack">
      <ConversationMessage
        message={{
          ...VOGUE_REPLY,
          actions: (
            <>
              <Button
                variant={liked ? "quiet" : "ghost"}
                size="compact"
                aria-pressed={liked}
                aria-label={`Thumbs up, ${liked ? 2 : 1} reactions`}
                onClick={() => setLiked(!liked)}
              >
                <IconThumbUp size={16} aria-hidden="true" />
                {liked ? "2" : "1"}
              </Button>
              <IconButton
                size="compact"
                aria-label="Reply to Vogue"
                icon={<IconMessageCircle size={16} />}
                onClick={() => setReply(!reply)}
              />
              <IconButton
                size="compact"
                aria-label={copied ? "Message copied" : "Copy message"}
                icon={copied ? <IconCheck size={16} /> : <IconCopy size={16} />}
                onClick={() => {
                  setCopyError(false);
                  void navigator.clipboard
                    .writeText(
                      "Let the conversation lead. Give each of us a small, steady place to work, then let that work fold away when we’re done.",
                    )
                    .then(
                      () => setCopied(true),
                      () => setCopyError(true),
                    );
                }}
              />
            </>
          ),
        }}
      />
      {copyError ? (
        <p role="alert" className="text-body-sm text-red-12">
          Couldn’t copy. You can select the message text instead.
        </p>
      ) : null}
      {reply ? (
        <p role="status" className="text-body-sm text-secondary">
          Reply target selected: Vogue. Session reply behavior is still being
          designed.
        </p>
      ) : null}
      <ConversationMessage
        message={{
          ...SESSION_PROMPT,
          id: "delivery-example",
          content: <p>Let’s try this with two agents first.</p>,
          delivery: {
            label: failed
              ? "Not sent. Your message is still here."
              : "Sent in this preview",
            failed,
            onRetry: failed ? () => setFailed(false) : undefined,
          },
        }}
      />
      <p className="text-body-sm text-secondary">
        Shared agent answer, message actions, selected reaction, and failed-send
        recovery. Reactions and retry are local examples.
      </p>
    </div>
  );
}

export function ConversationStateSpecimen() {
  const [state, setState] = useState<
    "loading" | "empty" | "failed" | "read-only"
  >("failed");
  return (
    <div className="component-specimen-stack">
      <Select
        label="Preview"
        value={state}
        onValueChange={(value) => setState(value as typeof state)}
        groups={[
          {
            label: "Conversation states",
            options: ["loading", "empty", "failed", "read-only"].map(
              (value) => ({ value, label: value }),
            ),
          },
        ]}
      />
      <ConversationState state={state} onRetry={() => setState("empty")} />
      <p className="text-body-sm text-secondary">
        Retry switches to an empty fixture, not a live request.
      </p>
    </div>
  );
}
