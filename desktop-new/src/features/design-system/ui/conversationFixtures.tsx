import { useEffect } from "react";
import { chipFaces } from "@/shared/chips/faceResolver";
import { InlineChip } from "@/shared/ui/InlineChip";
import type {
  AgentWorkPresentation,
  WorkStepPresentation,
} from "@/features/agent-activity/ui/workPresentation";
import type { ConversationMessagePresentation } from "@/features/conversation/ui/ConversationMessage";

function ExampleMentions() {
  useEffect(() => {
    for (const name of ["Vogue", "Rivet"])
      chipFaces.put(
        { kind: "agent", id: `specimen-conversation-${name}` },
        { label: name, loading: false, resolved: true },
      );
  }, []);
  return (
    <>
      <InlineChip
        address={{ kind: "agent", id: "specimen-conversation-Vogue" }}
      />{" "}
      <InlineChip
        address={{ kind: "agent", id: "specimen-conversation-Rivet" }}
      />
    </>
  );
}

export const SESSION_PROMPT: ConversationMessagePresentation = {
  id: "prompt-1",
  author: { name: "Morgan", fallback: "M", kind: "human" },
  time: "6:21 PM",
  dateTime: "2026-09-08T18:21:00-07:00",
  content: (
    <p>
      <ExampleMentions /> Help me make the Session feel like a conversation. I
      want to see you working together, without losing the thread.
    </p>
  ),
};

const step = (
  id: string,
  title: string,
  kind: WorkStepPresentation["kind"] = "tool",
  detail?: string,
): WorkStepPresentation => ({ id, title, kind, status: "completed", detail });

export const VOGUE_WORK: AgentWorkPresentation = {
  id: "prompt-1:vogue:turn-1",
  agent: { name: "Vogue", fallback: "V" },
  status: "running",
  statusLabel: "Working",
  summary: "Reviewed the conversation",
  earlierSteps: [
    step("v-1", "Read the conversation brief"),
    step("v-2", "Compared the current client"),
  ],
  visibleSteps: [
    step("v-3", "The conversation should stay in the foreground.", "progress"),
    step(
      "v-4",
      "Checked the message hierarchy",
      "tool",
      "Reviewed 6 message states.\nHuman and agent messages share the same reading rhythm.\nPrivate work stays secondary.",
    ),
    { ...step("v-5", "Refining how the work settles"), status: "running" },
  ],
};
export const RIVET_WORK: AgentWorkPresentation = {
  id: "prompt-1:rivet:turn-1",
  agent: { name: "Rivet", fallback: "R" },
  status: "running",
  statusLabel: "Working",
  summary: "Checked the handoff",
  earlierSteps: [step("r-1", "Read the activity contract")],
  visibleSteps: [
    step("r-2", "Keep the two agents independent.", "thought"),
    step(
      "r-3",
      "Checked the activity boundaries",
      "tool",
      "conversation → shared messages\nagent activity → private work\n\nNo new subscription or runtime state is needed for this specimen.",
    ),
    { ...step("r-4", "Checking the completion states"), status: "running" },
  ],
};

export const VOGUE_REPLY: ConversationMessagePresentation = {
  id: "vogue-answer",
  author: { name: "Vogue", fallback: "V", kind: "agent" },
  time: "6:22 PM",
  dateTime: "2026-09-08T18:22:00-07:00",
  content: (
    <>
      <p>
        Let the conversation lead. Give each of us a small, steady place to
        work, then let that work fold away when we’re done.
      </p>
      <ul>
        <li>Your question stays above the work.</li>
        <li>Each agent keeps its own place.</li>
        <li>Our replies stay in the conversation.</li>
      </ul>
    </>
  ),
};
export const RIVET_REPLY: ConversationMessagePresentation = {
  id: "rivet-answer",
  author: { name: "Rivet", fallback: "R", kind: "agent" },
  time: "6:23 PM",
  dateTime: "2026-09-08T18:23:00-07:00",
  content: (
    <p>
      That separation works. The shared answer can arrive while another agent is
      still working. Neither agent needs to move or disappear.
    </p>
  ),
};

export function completedWork(
  work: AgentWorkPresentation,
): AgentWorkPresentation {
  return {
    ...work,
    status: "completed",
    statusLabel: "Done",
    visibleSteps: work.visibleSteps.map((item) => ({
      ...item,
      status: "completed",
    })),
  };
}
