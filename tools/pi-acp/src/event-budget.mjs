function usageTokens(usage) {
  if (!usage) return 0;
  return (
    (usage.input || 0) +
    (usage.output || 0) +
    (usage.cacheRead || 0) +
    (usage.cacheWrite || 0)
  );
}

export class EventBudget {
  constructor(limits) {
    this.limits = limits;
    this.reset();
  }

  reset() {
    this.turns = 0;
    this.tools = 0;
    this.tokens = 0;
    this.thresholdReached = false;
    this.checkpointTurnStarted = false;
    this.forcedAbort = false;
  }

  onToolCall() {
    if (this.thresholdReached || this.tools >= this.limits.tools) {
      this.thresholdReached = true;
      return {
        block: true,
        reason: `Buzz event tool budget exhausted (${this.tools}/${this.limits.tools}); publish a concise budget checkpoint and stop.`,
      };
    }
    this.tools += 1;
    return undefined;
  }

  onTurnStart() {
    if (!this.thresholdReached) return "continue";
    if (!this.checkpointTurnStarted) {
      this.checkpointTurnStarted = true;
      return "checkpoint";
    }
    if (!this.forcedAbort) {
      this.forcedAbort = true;
      return "abort";
    }
    return "abort";
  }

  onTurnEnd(event) {
    this.turns += 1;
    this.tokens += usageTokens(event.message?.usage);
    for (const result of event.toolResults || []) {
      this.tokens += usageTokens(result?.usage);
    }
    if (
      !this.thresholdReached &&
      (this.turns >= this.limits.turns || this.tokens >= this.limits.tokens)
    ) {
      this.thresholdReached = true;
      return { checkpoint: true, message: this.checkpointMessage() };
    }
    return { checkpoint: false };
  }

  checkpointMessage() {
    return `[AUTOMATED BUDGET GUARD] Event budget reached: turns ${this.turns}/${this.limits.turns}, tools ${this.tools}/${this.limits.tools}, processed tokens ${this.tokens}/${this.limits.tokens}. Do not call more tools. Produce one concise [BUDGET CHECKPOINT] with completed work, exact blocker, and next action, then stop.`;
  }

  snapshot() {
    return {
      turns: this.turns,
      tools: this.tools,
      tokens: this.tokens,
      thresholdReached: this.thresholdReached,
      checkpointTurnStarted: this.checkpointTurnStarted,
      forcedAbort: this.forcedAbort,
      limits: this.limits,
    };
  }
}

export const testOnly = { usageTokens };
