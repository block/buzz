import type { BriefDecision } from "./briefDecisions";
import { buildCommandDirectionMessage } from "./decisionDispatch";
import {
  createDecisionExecution,
  type DecisionDirectionSource,
  type DecisionExecution,
  updateDecisionExecution,
} from "./decisionExecutionStore";

type ChiefConversation = Readonly<{ pubkey: string; channelId: string }>;

export type CommandDecisionDispatchDependencies = Readonly<{
  openChief: () => Promise<ChiefConversation>;
  send: (message: {
    channelId: string;
    content: string;
    mentionPubkeys: readonly string[];
  }) => Promise<unknown>;
  onUpdate: (execution: DecisionExecution) => void;
}>;

export async function dispatchCommandDecision(
  input: Readonly<{
    decision: BriefDecision;
    direction: string;
    directionSource: DecisionDirectionSource;
    now?: () => number;
  }>,
  dependencies: CommandDecisionDispatchDependencies,
): Promise<DecisionExecution> {
  const now = input.now ?? Date.now;
  let execution = createDecisionExecution({
    key: input.decision.key,
    runId: input.decision.runId,
    actionId: input.decision.actionId,
    direction: input.direction,
    directionSource: input.directionSource,
    now: now(),
  });
  dependencies.onUpdate(execution);

  try {
    const chief = await dependencies.openChief();
    execution = updateDecisionExecution(execution, {
      agentPubkey: chief.pubkey,
      channelId: chief.channelId,
      statusText: "Sending to Chief of Staff.",
      now: now(),
    });
    dependencies.onUpdate(execution);
    await dependencies.send({
      channelId: chief.channelId,
      content: buildCommandDirectionMessage({
        directionId: input.decision.key,
        decision: input.decision.coaA,
        direction: input.direction,
      }),
      mentionPubkeys: [chief.pubkey],
    });
    execution = updateDecisionExecution(execution, {
      statusText: "Sent to Chief of Staff.",
      now: now(),
    });
  } catch {
    execution = updateDecisionExecution(execution, {
      status: "failed",
      statusText: "Could not send the direction to the Chief of Staff.",
      now: now(),
    });
  }
  dependencies.onUpdate(execution);
  return execution;
}
