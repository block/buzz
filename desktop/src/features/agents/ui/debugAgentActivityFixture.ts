import type {
  AgentActivityDescriptor,
  TranscriptItem,
} from "./agentSessionTypes";

const sessionId = "debug-session-render-classes";
const turnId = "debug-turn-render-classes";
const channelId = "debug-channel-render-classes";
const userPubkey = "debug-user-render-classes";
const baseTimestamp = Date.parse("2026-06-30T00:00:00.000Z");
const workspacePath = "/Users/tho/.buzz/REPOS/buzz-pr-3-activity-feed-rebuild";

function timestamp(seconds: number) {
  return new Date(baseTimestamp + seconds * 1000).toISOString();
}

function descriptor(
  renderClass: AgentActivityDescriptor["renderClass"],
  label: string,
  preview: string | null,
  groupKey: string,
  options: Partial<AgentActivityDescriptor> = {},
): AgentActivityDescriptor {
  return {
    renderClass,
    label,
    preview,
    source: "fallback",
    groupKey,
    ...options,
  };
}

type ToolFixtureOptions = {
  id: string;
  renderClass: AgentActivityDescriptor["renderClass"];
  label: string;
  preview: string | null;
  groupKey: string;
  seconds: number;
  title?: string;
  toolName: string;
  buzzToolName?: string | null;
  args?: Record<string, unknown>;
  result?: string;
  status?: Extract<TranscriptItem, { type: "tool" }>["status"];
  isError?: boolean;
  completedSeconds?: number | null;
  descriptorOptions?: Partial<AgentActivityDescriptor>;
};

function toolItem({
  id,
  renderClass,
  label,
  preview,
  groupKey,
  seconds,
  title = label,
  toolName,
  buzzToolName = null,
  args = {},
  result = "",
  status = "completed",
  isError = false,
  completedSeconds = seconds + 0.4,
  descriptorOptions = {},
}: ToolFixtureOptions): Extract<TranscriptItem, { type: "tool" }> {
  return {
    id,
    type: "tool",
    renderClass,
    descriptor: descriptor(renderClass, label, preview, groupKey, {
      source: "harness",
      ...descriptorOptions,
    }),
    title,
    toolName,
    buzzToolName,
    status,
    args,
    result,
    isError,
    timestamp: timestamp(seconds),
    startedAt: timestamp(seconds),
    completedAt: completedSeconds == null ? null : timestamp(completedSeconds),
    acpSource: "tool_call_update",
    turnId,
    sessionId,
    channelId,
  };
}

function fileEditItem(
  id: string,
  path: string,
  seconds: number,
): Extract<TranscriptItem, { type: "tool" }> {
  return toolItem({
    id,
    renderClass: "file-edit",
    label: "Edited file",
    preview: path,
    groupKey: "file-edit:str_replace",
    seconds,
    title: "str_replace",
    toolName: "dev__str_replace",
    args: {
      path,
      old_str:
        "          <ManagedAgentSessionPanel\n            agent={agent}\n            channelId={channel?.id ?? null}\n",
      new_str:
        "          <ManagedAgentSessionPanel\n            agent={agent}\n            channelId={channel?.id ?? null}\n            transcriptOverride={debugTranscript}\n",
      workdir: workspacePath,
    },
    result: `Replaced 1 occurrence in ${workspacePath}/${path}.\n\n--- a/${workspacePath}/${path}\n+++ b/${workspacePath}/${path}\n@@\n           <ManagedAgentSessionPanel\n             agent={agent}\n             channelId={channel?.id ?? null}\n+            transcriptOverride={debugTranscript}\n`,
    descriptorOptions: {
      operation: "str_replace",
      object: path,
      tone: "write",
    },
  });
}

function shellResultJson(
  stdout = "",
  {
    exitCode = 0,
    stderr = "",
    durationMs = 420,
  }: {
    exitCode?: number;
    stderr?: string;
    durationMs?: number;
  } = {},
) {
  return JSON.stringify(
    {
      exit_code: exitCode,
      stdout,
      stderr,
      timed_out: false,
      duration_ms: durationMs,
      stdout_truncated: false,
      stderr_truncated: false,
      stdout_artifact: null,
      stderr_artifact: null,
      notes: [],
    },
    null,
    2,
  );
}

function shellCommandItem(
  id: string,
  command: string,
  seconds: number,
  result = "",
): Extract<TranscriptItem, { type: "tool" }> {
  return toolItem({
    id,
    renderClass: "shell",
    label: "Ran command",
    preview: command,
    groupKey: "shell:command",
    seconds,
    title: "Shell",
    toolName: "dev__shell",
    args: { command, workdir: workspacePath, timeout_ms: 120000 },
    result: shellResultJson(result),
    descriptorOptions: {
      operation: "shell",
    },
  });
}

function todoUpdateItem(
  id: string,
  preview: string,
  seconds: number,
): Extract<TranscriptItem, { type: "tool" }> {
  return toolItem({
    id,
    renderClass: "plan",
    label: "Updated todos",
    preview,
    groupKey: "plan:todo",
    seconds,
    title: "todo",
    toolName: "dev__todo",
    args: {
      todos: [
        {
          text: preview,
          done: true,
        },
      ],
    },
    result: `- [x] ${preview}`,
    descriptorOptions: {
      operation: "todo",
      tone: "write",
    },
  });
}

/**
 * Temporary design-debug fixture: one coherent turn that exercises every
 * AgentActivityRenderClass and every TranscriptItem variant. Keep this isolated
 * so it can be removed surgically before the PR merges.
 */
export const DEBUG_AGENT_ACTIVITY_FIXTURE: TranscriptItem[] = [
  {
    id: "debug:turn-started",
    type: "lifecycle",
    renderClass: "status",
    title: "Turn started",
    text: "Triggered by 1 event.",
    timestamp: timestamp(0),
    acpSource: "turn_started",
    turnId,
    sessionId,
    channelId,
  },
  {
    id: "debug:session-resolved",
    type: "lifecycle",
    renderClass: "status",
    title: "Session ready",
    text: "Observer attached to the local agent session.",
    timestamp: timestamp(1),
    acpSource: "session_resolved",
    turnId,
    sessionId,
    channelId,
  },
  {
    id: "debug:user-prompt",
    type: "message",
    renderClass: "message",
    role: "user",
    title: "Taylor Ho",
    text: "@Agent audit the activity feed taxonomy and show me the risky spots before you edit.",
    timestamp: timestamp(2),
    acpSource: "session/prompt:user",
    authorPubkey: userPubkey,
    turnId,
    sessionId,
    channelId,
  },
  {
    id: "debug:prompt-context",
    type: "metadata",
    renderClass: "raw-rail",
    title: "Prompt context",
    sections: [
      {
        title: "Channel",
        body: "buzz-agent-observability · live agent activity thread",
      },
      {
        title: "Recent directive",
        body: "Keep the UI honest: reasoning, messages, shell output, relay operations, and raw ACP payloads stay distinct.",
      },
    ],
    timestamp: timestamp(2.2),
    acpSource: "session/prompt:context",
    turnId,
    sessionId,
    channelId,
  },
  {
    id: "debug:thought",
    type: "thought",
    renderClass: "thought",
    title: "Thinking",
    text: "I need to inspect the existing taxonomy, verify the render classes, then make the smallest safe change. Shell stdout is evidence, not reasoning.",
    timestamp: timestamp(4),
    acpSource: "agent_thought_chunk",
    turnId,
    sessionId,
    channelId,
  },
  {
    id: "debug:plan",
    type: "plan",
    renderClass: "plan",
    title: "Plan updated",
    text: "1. Read the transcript components.\n2. Classify the observed tools.\n3. Patch the Activity header.\n4. Run desktop gates and report the pushed SHA.",
    timestamp: timestamp(5),
    acpSource: "plan",
    turnId,
    sessionId,
    channelId,
  },
  {
    id: "debug:status",
    type: "lifecycle",
    renderClass: "status",
    title: "Observer connected",
    text: "Streaming normalized ACP activity.",
    timestamp: timestamp(6),
    acpSource: "observer_connected",
    turnId,
    sessionId,
    channelId,
  },
  shellCommandItem(
    "debug:shell-tool",
    "git status --short",
    8,
    "## tho/activity-feed-rebuild...origin/main [ahead 4]\n",
  ),
  {
    id: "debug:relay-op-tool",
    type: "tool",
    renderClass: "relay-op",
    descriptor: descriptor(
      "relay-op",
      "Channels Get",
      "buzz-agent-observability",
      "buzz-cli:channels.get",
      {
        operation: "channels.get",
        object: "buzz-agent-observability",
        source: "shell",
        tone: "read",
      },
    ),
    title: "buzz channels get",
    toolName: "dev__shell",
    buzzToolName: null,
    status: "completed",
    args: {
      command: "buzz channels get --channel buzz-agent-observability",
      workdir: workspacePath,
      timeout_ms: 120000,
    },
    result: shellResultJson(
      '{"name":"buzz-agent-observability","members":7}\n',
    ),
    isError: false,
    timestamp: timestamp(10),
    startedAt: timestamp(10),
    completedAt: timestamp(11),
    acpSource: "tool_call_update",
    turnId,
    sessionId,
    channelId,
  },
  {
    id: "debug:message-tool",
    type: "tool",
    renderClass: "message",
    descriptor: descriptor(
      "message",
      "Send Message",
      "@Ned picked it up — testing the full taxonomy now.",
      "buzz-cli:messages.send",
      {
        operation: "messages.send",
        object: "@Ned picked it up — testing the full taxonomy now.",
        source: "shell",
        tone: "write",
      },
    ),
    title: "buzz messages send",
    toolName: "dev__shell",
    buzzToolName: null,
    status: "completed",
    args: {
      command:
        'buzz messages send --channel buzz-agent-observability --content "@Ned picked it up — testing the full taxonomy now."',
      workdir: workspacePath,
      timeout_ms: 120000,
    },
    result: shellResultJson('{"accepted":true,"event_id":"debug-event"}\n'),
    isError: false,
    timestamp: timestamp(12),
    startedAt: timestamp(12),
    completedAt: timestamp(13),
    acpSource: "tool_call_update",
    turnId,
    sessionId,
    channelId,
  },
  {
    id: "debug:file-edit-tool",
    type: "tool",
    renderClass: "file-edit",
    descriptor: descriptor(
      "file-edit",
      "Edited file",
      "desktop/src/features/channels/ui/AgentSessionThreadPanel.tsx",
      "file-edit:str_replace",
      {
        operation: "str_replace",
        object: "desktop/src/features/channels/ui/AgentSessionThreadPanel.tsx",
        source: "harness",
        tone: "write",
      },
    ),
    title: "str_replace",
    toolName: "dev__str_replace",
    buzzToolName: null,
    status: "completed",
    args: {
      path: "desktop/src/features/channels/ui/AgentSessionThreadPanel.tsx",
      old_str: "<Switch />",
      new_str: "<DropdownMenuCheckboxItem />",
      workdir: workspacePath,
    },
    result: `Replaced 1 occurrence in ${workspacePath}/desktop/src/features/channels/ui/AgentSessionThreadPanel.tsx.\n\n--- a/${workspacePath}/desktop/src/features/channels/ui/AgentSessionThreadPanel.tsx\n+++ b/${workspacePath}/desktop/src/features/channels/ui/AgentSessionThreadPanel.tsx\n@@\n-<Switch />\n+<DropdownMenuCheckboxItem />\n`,
    isError: false,
    timestamp: timestamp(14),
    startedAt: timestamp(14),
    completedAt: timestamp(15),
    acpSource: "tool_call_update",
    turnId,
    sessionId,
    channelId,
  },
  fileEditItem(
    "debug:file-edit-thread-panel-2",
    "desktop/src/features/channels/ui/AgentSessionThreadPanel.tsx",
    15.2,
  ),
  fileEditItem(
    "debug:file-edit-managed-panel-1",
    "desktop/src/features/agents/ui/ManagedAgentSessionPanel.tsx",
    15.6,
  ),
  fileEditItem(
    "debug:file-edit-transcript-list-1",
    "desktop/src/features/agents/ui/AgentSessionTranscriptList.tsx",
    16,
  ),
  fileEditItem(
    "debug:file-edit-grouping-1",
    "desktop/src/features/agents/ui/agentSessionTranscriptGrouping.ts",
    16.4,
  ),
  fileEditItem(
    "debug:file-edit-tool-item-1",
    "desktop/src/features/agents/ui/AgentSessionToolItem.tsx",
    16.8,
  ),
  fileEditItem(
    "debug:file-edit-tool-summary-1",
    "desktop/src/features/agents/ui/agentSessionToolSummary.ts",
    17.2,
  ),
  fileEditItem(
    "debug:file-edit-classifier-1",
    "desktop/src/features/agents/ui/agentSessionToolClassifier.ts",
    17.6,
  ),
  shellCommandItem(
    "debug:shell-burst-1",
    "git status --short",
    18.2,
    "M desktop/src/features/agents/ui/ManagedAgentSessionPanel.tsx\n",
  ),
  shellCommandItem(
    "debug:shell-burst-2",
    "pnpm --dir desktop lint",
    18.6,
    "Checked 812 files in 1.9s. No fixes applied.\n",
  ),
  shellCommandItem(
    "debug:shell-burst-3",
    "node --import ./test-loader.mjs --experimental-strip-types --test src/features/agents/ui/debugAgentActivityFixture.test.mjs",
    19,
    "1 test passed\n",
  ),
  shellCommandItem(
    "debug:shell-burst-4",
    "pnpm --dir desktop typecheck",
    19.4,
    "Typecheck completed.\n",
  ),
  shellCommandItem(
    "debug:shell-burst-5",
    "git diff --stat",
    19.8,
    "4 files changed, 211 insertions(+), 38 deletions(-)\n",
  ),
  shellCommandItem(
    "debug:shell-burst-6",
    "git add desktop/src/features/agents/ui desktop/src/features/channels/ui",
    20.2,
  ),
  shellCommandItem(
    "debug:shell-burst-7",
    'git commit -m "feat(desktop): add activity render-class debug fixture"',
    20.6,
    "[tho/activity-feed-rebuild aa84200ad] feat(desktop): add activity render-class debug fixture\n",
  ),
  toolItem({
    id: "debug:block-safe-github",
    renderClass: "generic",
    label: "Ran tool",
    preview: "block-safe-github",
    groupKey: "generic:block-safe-github",
    seconds: 21.4,
    title: "block-safe-github",
    toolName: "block-safe-github",
    result: "Remote origin is in the block GitHub org.",
    descriptorOptions: {
      operation: "block-safe-github",
    },
  }),
  todoUpdateItem(
    "debug:todo-after-first-check",
    "Inspect transcript panel settings and debug activity fixture",
    22,
  ),
  shellCommandItem("debug:shell-push-burst-1", "git status --short", 22.8, ""),
  shellCommandItem(
    "debug:shell-push-burst-2",
    "git push -u origin HEAD",
    23.2,
    "branch 'tho/activity-feed-rebuild' set up to track 'origin/tho/activity-feed-rebuild'.\n",
  ),
  shellCommandItem(
    "debug:shell-push-burst-3",
    "git rev-parse --short=40 HEAD",
    23.6,
    "aa84200ad266d16f81da2f9c347518a7525a3ef4\n",
  ),
  todoUpdateItem(
    "debug:todo-after-push",
    "Report branch, SHA, and commit to Ned",
    24.4,
  ),
  {
    id: "debug:message-tool-pushed-report",
    type: "tool",
    renderClass: "message",
    descriptor: descriptor(
      "message",
      "Send Message",
      "@Ned Done and pushed.\n\nBranch: `tho/activity-feed-rebuild`\nSHA: `aa84200ad266d16f81da2f9c347518a7525a3ef4`\nCommit: `feat(desktop): add activity render-class debug fixture`",
      "buzz-cli:messages.send",
      {
        operation: "messages.send",
        object: "@Ned Done and pushed.",
        source: "shell",
        tone: "write",
      },
    ),
    title: "buzz messages send",
    toolName: "dev__shell",
    buzzToolName: null,
    status: "completed",
    args: {
      command:
        'buzz messages send --channel agents --content "@Ned Done and pushed."',
      workdir: workspacePath,
      timeout_ms: 120000,
    },
    result: shellResultJson(
      '{"accepted":true,"event_id":"debug-pushed-report"}\n',
    ),
    isError: false,
    timestamp: timestamp(25.2),
    startedAt: timestamp(25.2),
    completedAt: timestamp(25.8),
    acpSource: "tool_call_update",
    turnId,
    sessionId,
    channelId,
  },
  {
    id: "debug:permission",
    type: "lifecycle",
    renderClass: "permission",
    title: "Permission requested",
    text: "Confirm force-with-lease push to block/buzz.",
    timestamp: timestamp(16),
    descriptor: descriptor(
      "permission",
      "Permission requested",
      "force-with-lease push",
      "permission:git-push",
      {
        source: "acp",
        tone: "admin",
      },
    ),
    acpSource: "permission_request",
    turnId,
    sessionId,
    channelId,
  },
  {
    id: "debug:generic-tool",
    type: "tool",
    renderClass: "generic",
    descriptor: descriptor(
      "generic",
      "Ran tool",
      "resolve-worktree-context",
      "generic:resolve_worktree_context",
      {
        operation: "resolve_worktree_context",
        source: "fallback",
      },
    ),
    title: "resolve-worktree-context",
    toolName: "resolve_worktree_context",
    buzzToolName: null,
    status: "completed",
    args: { worktree: "~/.buzz/REPOS/buzz-pr-3-activity-feed-rebuild" },
    result: "branch=tho/activity-feed-rebuild clean=true",
    isError: false,
    timestamp: timestamp(17),
    startedAt: timestamp(17),
    completedAt: timestamp(18),
    acpSource: "tool_call_update",
    turnId,
    sessionId,
    channelId,
  },
  {
    id: "debug:error-tool",
    type: "tool",
    renderClass: "error",
    descriptor: descriptor(
      "error",
      "Command failed",
      "pnpm --dir desktop test -- --runInBand",
      "error:shell",
      {
        operation: "shell",
        source: "harness",
      },
    ),
    title: "Shell",
    toolName: "dev__shell",
    buzzToolName: null,
    status: "failed",
    args: {
      command: "pnpm --dir desktop test -- --runInBand",
      workdir: workspacePath,
      timeout_ms: 120000,
    },
    result: shellResultJson("", {
      exitCode: 1,
      stderr: "Unknown option: --runInBand\n",
      durationMs: 913,
    }),
    isError: true,
    timestamp: timestamp(19),
    startedAt: timestamp(19),
    completedAt: timestamp(20),
    acpSource: "tool_call_update",
    turnId,
    sessionId,
    channelId,
  },
  {
    id: "debug:suppressed-tool",
    type: "tool",
    renderClass: "suppressed",
    descriptor: descriptor(
      "suppressed",
      "Checked todos",
      null,
      "suppressed:stop-hook",
      {
        operation: "stop_hook",
        source: "harness",
      },
    ),
    title: "_Stop",
    toolName: "dev___Stop",
    buzzToolName: null,
    status: "completed",
    args: {},
    result: "",
    isError: false,
    timestamp: timestamp(21),
    startedAt: timestamp(21),
    completedAt: timestamp(21.4),
    acpSource: "tool_call_update",
    turnId,
    sessionId,
    channelId,
  },
  {
    id: "debug:raw-rail",
    type: "metadata",
    renderClass: "raw-rail",
    title: "Raw ACP payload",
    sections: [
      {
        title: "tool_call_update",
        body: JSON.stringify(
          {
            jsonrpc: "2.0",
            method: "session/update",
            params: {
              sessionId,
              update: {
                sessionUpdate: "tool_call_update",
                toolCallId: "debug:file-edit-tool",
                status: "completed",
                content: [
                  {
                    type: "content",
                    content: {
                      type: "text",
                      text: `Replaced 1 occurrence in ${workspacePath}/desktop/src/features/channels/ui/AgentSessionThreadPanel.tsx.\n\n--- a/${workspacePath}/desktop/src/features/channels/ui/AgentSessionThreadPanel.tsx\n+++ b/${workspacePath}/desktop/src/features/channels/ui/AgentSessionThreadPanel.tsx\n@@\n-<Switch />\n+<DropdownMenuCheckboxItem />\n`,
                    },
                  },
                ],
                rawOutput: {
                  isError: false,
                },
              },
            },
          },
          null,
          2,
        ),
      },
    ],
    timestamp: timestamp(22),
    acpSource: "raw_json_rpc",
    turnId,
    sessionId,
    channelId,
  },
  {
    id: "debug:error-lifecycle",
    type: "lifecycle",
    renderClass: "error",
    title: "Turn error recovered",
    text: "Retried with the supported desktop test command.",
    timestamp: timestamp(23),
    descriptor: descriptor(
      "error",
      "Turn error recovered",
      "unsupported test flag",
      "error:turn",
      {
        source: "acp",
      },
    ),
    acpSource: "turn_error",
    turnId,
    sessionId,
    channelId,
  },
  {
    id: "debug:assistant-message",
    type: "message",
    renderClass: "message",
    role: "assistant",
    title: "Agent",
    text: "Done — the Activity header now keeps Raw inside the settings cog, and the desktop gates are green.",
    timestamp: timestamp(25),
    acpSource: "agent_message_chunk",
    turnId,
    sessionId,
    channelId,
  },
];
