#!/usr/bin/env node

import {
  createAgentSession,
  DefaultResourceLoader,
  getAgentDir,
  SessionManager,
} from "@earendil-works/pi-coding-agent";
import { createBuzzTools } from "./buzz-tools.mjs";
import { attachJsonlReader, writeJsonl } from "./jsonl.mjs";

function option(name) {
  const index = process.argv.indexOf(name);
  return index >= 0 ? process.argv[index + 1] : undefined;
}

function positiveInteger(name, fallback) {
  const value = Number.parseInt(process.env[name] || "", 10);
  return Number.isSafeInteger(value) && value > 0 ? value : fallback;
}

const limits = {
  turns: positiveInteger("PI_ACP_MAX_TURNS", 3),
  tools: positiveInteger("PI_ACP_MAX_TOOLS", 3),
  tokens: positiveInteger("PI_ACP_MAX_PROCESSED_TOKENS", 75_000),
};
const budget = {
  turns: 0,
  tools: 0,
  tokens: 0,
  thresholdReached: false,
  checkpointTurnStarted: false,
  forcedAbort: false,
};
function resetEventBudget() {
  Object.assign(budget, {
    turns: 0,
    tools: 0,
    tokens: 0,
    thresholdReached: false,
    checkpointTurnStarted: false,
    forcedAbort: false,
  });
}
let session;
let streaming = false;
let disposed = false;
let buzzContext = null;

const budgetExtension = {
  name: "buzz-event-budget",
  factory(pi) {
    pi.on("tool_call", async () => {
      if (budget.thresholdReached || budget.tools >= limits.tools) {
        budget.thresholdReached = true;
        return {
          block: true,
          reason: `Buzz event tool budget exhausted (${budget.tools}/${limits.tools}); publish a concise budget checkpoint and stop.`,
        };
      }
      budget.tools += 1;
      return undefined;
    });
  },
};

const builtInTools = (option("--tools") || process.env.PI_ACP_TOOLS || "read")
  .split(",")
  .map((value) => value.trim())
  .filter(Boolean);
const customTools = createBuzzTools({ getContext: () => buzzContext });
const tools = [...builtInTools, ...customTools.map((tool) => tool.name)];
const resourceLoader = new DefaultResourceLoader({
  cwd: process.cwd(),
  agentDir: getAgentDir(),
  noExtensions: true,
  noSkills: true,
  noPromptTemplates: true,
  noThemes: true,
  noContextFiles: true,
  systemPrompt: option("--system-prompt"),
  extensionFactories: [budgetExtension],
});

const ready = (async () => {
  await resourceLoader.reload();
  const created = await createAgentSession({
    cwd: process.cwd(),
    tools,
    customTools,
    resourceLoader,
    sessionManager: SessionManager.inMemory(process.cwd()),
  });
  session = created.session;
  session.subscribe((event) => {
    writeJsonl(process.stdout, event);
    if (event.type === "agent_start") streaming = true;
    if (event.type === "agent_settled") {
      streaming = false;
      buzzContext = null;
    }
    if (event.type === "turn_start" && budget.thresholdReached) {
      if (!budget.checkpointTurnStarted) budget.checkpointTurnStarted = true;
      else if (!budget.forcedAbort) {
        budget.forcedAbort = true;
        void session.abort();
      }
    }
    if (event.type === "turn_end") {
      budget.turns += 1;
      const usage = event.message?.usage;
      if (usage) {
        budget.tokens +=
          (usage.input || 0) +
          (usage.output || 0) +
          (usage.cacheRead || 0) +
          (usage.cacheWrite || 0);
      }
      for (const result of event.toolResults || []) {
        const nested = result?.usage;
        if (nested) {
          budget.tokens +=
            (nested.input || 0) +
            (nested.output || 0) +
            (nested.cacheRead || 0) +
            (nested.cacheWrite || 0);
        }
      }
      if (
        !budget.thresholdReached &&
        (budget.turns >= limits.turns || budget.tokens >= limits.tokens)
      ) {
        budget.thresholdReached = true;
        void session.steer(
          `[AUTOMATED BUDGET GUARD] Event budget reached: turns ${budget.turns}/${limits.turns}, tools ${budget.tools}/${limits.tools}, processed tokens ${budget.tokens}/${limits.tokens}. Do not call more tools. Produce one concise [BUDGET CHECKPOINT] with completed work, exact blocker, and next action, then stop.`,
        );
      }
    }
  });
})();

function response(command, success, data, error) {
  writeJsonl(process.stdout, {
    id: command.id,
    type: "response",
    command: command.type,
    success,
    ...(data === undefined ? {} : { data }),
    ...(error === undefined ? {} : { error }),
  });
}

async function handle(command) {
  try {
    await ready;
    switch (command.type) {
      case "get_state":
        response(command, true, {
          model: session.model,
          thinkingLevel: session.thinkingLevel,
          isStreaming: streaming,
          sessionId: session.sessionId,
          autoCompactionEnabled: session.autoCompactionEnabled,
          budget: { ...budget, limits },
        });
        break;
      case "prompt":
        if (streaming) {
          response(command, false, undefined, "agent is already streaming");
          break;
        }
        resetEventBudget();
        buzzContext = command.buzzContext;
        response(command, true);
        void session.prompt(command.message).catch((error) => {
          process.stderr.write(
            `[pi-acp-sdk] prompt failed: ${error.message}\n`,
          );
          writeJsonl(process.stdout, { type: "agent_settled" });
        });
        break;
      case "steer":
        await session.steer(command.message);
        response(command, true);
        break;
      case "abort":
        await session.abort();
        response(command, true);
        break;
      default:
        response(
          command,
          false,
          undefined,
          `unsupported SDK bridge command: ${command.type}`,
        );
    }
  } catch (error) {
    response(command, false, undefined, error.message);
  }
}

attachJsonlReader(
  process.stdin,
  (command) => void handle(command),
  (error) => process.stderr.write(`[pi-acp-sdk] ${error.message}\n`),
);

async function shutdown() {
  if (disposed) return;
  disposed = true;
  try {
    await ready;
    await session.abort();
    session.dispose();
  } catch {
    // Startup or shutdown already failed.
  }
}

process.stdin.on("end", () => void shutdown());
for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, async () => {
    await shutdown();
    process.exit(0);
  });
}
