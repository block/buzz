import type {
  AgentActivityDescriptor,
  TranscriptItem,
} from "./agentSessionTypes";
import type { UserProfileLookup } from "@/features/profile/lib/identity";
import { debugPlanUpdateItem } from "./debugAgentActivityPlanFixture";

const sessionId = "debug-session-render-classes";
const turnId = "debug-turn-render-classes";
const channelId = "debug-channel-render-classes";
const userPubkey =
  "1111111111111111111111111111111111111111111111111111111111111111";
const reviewerPubkey =
  "2222222222222222222222222222222222222222222222222222222222222222";
const baseTimestamp = Date.parse("2026-06-30T00:00:00.000Z");
const workspacePath = "/Users/tho/.buzz/REPOS/buzz-pr-3-activity-feed-rebuild";
export const DEBUG_AGENT_ACTIVITY_AGENT_NAME = "Fixture Agent";
export const DEBUG_AGENT_ACTIVITY_AGENT_AVATAR_URL =
  "https://picsum.photos/seed/activity-agent-placeholder/200";
export const DEBUG_AGENT_ACTIVITY_PROFILES: UserProfileLookup = {
  [userPubkey]: debugProfile("Taylor Ho", "seedhash", "tho"),
  [reviewerPubkey]: debugProfile("Avery", "activity-user-avery", "avery"),
};

function timestamp(seconds: number) {
  return new Date(baseTimestamp + seconds * 1000).toISOString();
}

function debugProfile(displayName: string, seed: string, handle: string) {
  return {
    displayName,
    avatarUrl: `https://picsum.photos/seed/${seed}/200`,
    nip05Handle: `${handle}@buzz.local`,
    isAgent: false,
    ownerPubkey: null,
  };
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

type FileEditScenario = {
  oldStr: string;
  newStr: string;
  diff: string;
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

function fileEditScenario(path: string): FileEditScenario {
  if (path.endsWith("VISION_ACTIVITY.md")) {
    return {
      oldStr:
        "## Principles\n\nThe activity feed should make agent work legible without turning every event into chat.\n",
      newStr:
        "## Principles\n\nThe activity feed should make agent work legible without turning every event into chat. The common path is agent text, stdout, user feedback, then another tool pass.\n",
      diff: " ## Principles\n \n-The activity feed should make agent work legible without turning every event into chat.\n+The activity feed should make agent work legible without turning every event into chat. The common path is agent text, stdout, user feedback, then another tool pass.\n",
    };
  }

  if (path.endsWith("activityRenderClasses/ActivityRow.tsx")) {
    return {
      oldStr:
        "  const match = label.match(\n    /^(Captured|Edited|Ran|Read|Updated|Viewed)\\s+(.+)$/,\n  );\n",
      newStr:
        "  const match = label.match(\n    /^(Captured|Edited|Ran|Read|Updated|Viewed|Wrote)\\s+(.+)$/,\n  );\n",
      diff: "   const match = label.match(\n-    /^(Captured|Edited|Ran|Read|Updated|Viewed)\\s+(.+)$/,\n+    /^(Captured|Edited|Ran|Read|Updated|Viewed|Wrote)\\s+(.+)$/,\n   );\n",
    };
  }

  if (path.endsWith("AgentSessionThreadPanel.tsx")) {
    return {
      oldStr:
        "          showRaw={showRawFeed}\n          transcriptOverride={\n            showDebugRenderClasses ? DEBUG_AGENT_ACTIVITY_FIXTURE : undefined\n          }\n",
      newStr:
        "          showRaw={showRawFeed}\n          transcriptOverride={debugTranscript}\n",
      diff: "           showRaw={showRawFeed}\n-          transcriptOverride={\n-            showDebugRenderClasses ? DEBUG_AGENT_ACTIVITY_FIXTURE : undefined\n-          }\n+          transcriptOverride={debugTranscript}\n",
    };
  }

  if (path.endsWith("ManagedAgentSessionPanel.tsx")) {
    return {
      oldStr:
        "  const displayTranscript = transcriptOverride ?? scopedTranscript;\n\n  const scopedEvents = React.useMemo(\n",
      newStr:
        "  const displayTranscript = transcriptOverride ?? scopedTranscript;\n  const displayEventCount = transcriptOverride?.length ?? scopedEvents.length;\n\n  const scopedEvents = React.useMemo(\n",
      diff: "   const displayTranscript = transcriptOverride ?? scopedTranscript;\n+  const displayEventCount = transcriptOverride?.length ?? scopedEvents.length;\n \n   const scopedEvents = React.useMemo(\n",
    };
  }

  if (path.endsWith("AgentSessionTranscriptList.tsx")) {
    return {
      oldStr:
        '        {displayBlocks.map((block) => (\n          <div\n            className="content-visibility-auto"\n',
      newStr:
        '        {displayBlocks.map((block, index) => (\n          <div\n            className="content-visibility-auto"\n            data-debug-index={index}\n',
      diff: '         {displayBlocks.map((block) => (\n+        {displayBlocks.map((block, index) => (\n           <div\n             className="content-visibility-auto"\n+            data-debug-index={index}\n',
    };
  }

  if (path.endsWith("agentSessionTranscriptGrouping.ts")) {
    return {
      oldStr:
        '    if (run.length >= 3) {\n      grouped.push({\n        kind: "summary",\n',
      newStr:
        '    const minimumRunLength = key === "shell:command" ? 3 : 4;\n    if (run.length >= minimumRunLength) {\n      grouped.push({\n        kind: "summary",\n',
      diff: '     if (run.length >= 3) {\n+    const minimumRunLength = key === "shell:command" ? 3 : 4;\n+    if (run.length >= minimumRunLength) {\n       grouped.push({\n',
    };
  }

  if (path.endsWith("AgentSessionToolItem.tsx")) {
    return {
      oldStr:
        '      {duration ? (\n        <span className={cn("shrink-0 text-xs", mutedTone)}>{duration}</span>\n      ) : null}\n',
      newStr:
        '      {duration ? (\n        <span className={cn("shrink-0 font-mono text-xs", mutedTone)}>\n          {duration}\n        </span>\n      ) : null}\n',
      diff: '       {duration ? (\n-        <span className={cn("shrink-0 text-xs", mutedTone)}>{duration}</span>\n+        <span className={cn("shrink-0 font-mono text-xs", mutedTone)}>\n+          {duration}\n+        </span>\n       ) : null}\n',
    };
  }

  if (path.endsWith("agentSessionToolSummary.ts")) {
    return {
      oldStr:
        "  return {\n    label: descriptor.label,\n    preview: descriptor.preview,\n",
      newStr:
        "  return {\n    label: descriptor.label,\n    preview: descriptor.preview?.trim() || null,\n",
      diff: "   return {\n     label: descriptor.label,\n-    preview: descriptor.preview,\n+    preview: descriptor.preview?.trim() || null,\n",
    };
  }

  return {
    oldStr:
      '  if (kind === "str_replace") {\n    const path = getToolString(input.args, ["path"]);\n',
    newStr:
      '  if (kind === "str_replace") {\n    const path = getToolString(input.args, ["path"]);\n    const replaceAll = Boolean(input.args.replace_all);\n',
    diff: '   if (kind === "str_replace") {\n     const path = getToolString(input.args, ["path"]);\n+    const replaceAll = Boolean(input.args.replace_all);\n',
  };
}

function fileEditItem(
  id: string,
  path: string,
  seconds: number,
  durationMs = 420,
): Extract<TranscriptItem, { type: "tool" }> {
  const change = fileEditScenario(path);
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
      old_str: change.oldStr,
      new_str: change.newStr,
      workdir: workspacePath,
    },
    result: `Replaced 1 occurrence in ${workspacePath}/${path}.\n\n--- a/${workspacePath}/${path}\n+++ b/${workspacePath}/${path}\n@@\n${change.diff}`,
    completedSeconds: seconds + durationMs / 1000,
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
  durationMs = 420,
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
    result: shellResultJson(result, { durationMs }),
    completedSeconds: seconds + durationMs / 1000,
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

function assistantMessage(
  id: string,
  text: string,
  seconds: number,
): Extract<TranscriptItem, { type: "message" }> {
  return {
    id,
    type: "message",
    renderClass: "message",
    role: "assistant",
    title: DEBUG_AGENT_ACTIVITY_AGENT_NAME,
    text,
    timestamp: timestamp(seconds),
    acpSource: "agent_message_chunk",
    turnId,
    sessionId,
    channelId,
  };
}

function userMessage(
  id: string,
  authorPubkey: string,
  title: string,
  text: string,
  seconds: number,
): Extract<TranscriptItem, { type: "message" }> {
  return {
    id,
    type: "message",
    renderClass: "message",
    role: "user",
    title,
    text,
    timestamp: timestamp(seconds),
    acpSource: "user_message_chunk",
    authorPubkey,
    turnId,
    sessionId,
    channelId,
  };
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
        title: "Agent Memory — core",
        body: [
          "Prefer auditing the taxonomy before changing the renderer.",
          "Keep reasoning, assistant/user messages, tool output, relay operations, and raw ACP payloads visually distinct.",
        ].join("\n"),
      },
      {
        title: "Context",
        body: [
          "Channel: buzz-agent-observability",
          "Thread: live agent activity rebuild review",
          "Goal: design the activity feed against realistic prompt context, boring chatter, and tool-heavy turns.",
        ].join("\n"),
      },
      {
        title: "Buzz event: @mention",
        body: [
          `From: Taylor Ho (hex: ${userPubkey})`,
          "Kind: channel message",
          "Content: @Agent audit the activity feed taxonomy and show me the risky spots before you edit.",
        ].join("\n"),
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
  assistantMessage(
    "debug:assistant-shape",
    "Let me think about the shape:\n\n1. Confirm how the current render classes group tool activity.\n2. Check the fixture against real ACP/MCP payloads.\n3. Keep the debug state noisy enough to resemble a normal agent turn.",
    4.4,
  ),
  shellCommandItem(
    "debug:shell-tool",
    "git status --short",
    4.8,
    "## tho/activity-feed-rebuild...origin/main [ahead 4]\n",
  ),
  userMessage(
    "debug:user-followup-location",
    userPubkey,
    "Taylor Ho",
    "Yep, and make sure the fixture includes the boring chatter too — short status messages are most of what I see in real agent turns.",
    5.4,
  ),
  debugPlanUpdateItem(
    "debug:plan-initial",
    "1. [ ] Read the transcript components.\n2. [ ] Classify the observed tools.\n3. [ ] Patch the Activity header.\n4. [ ] Run desktop gates and report the pushed SHA.",
    5,
  ),
  assistantMessage(
    "debug:assistant-ack-location",
    "Makes sense. I’ll bias this toward the ordinary path: small observations, quick shell checks, file edits, and a final report instead of only taxonomy edge cases.",
    5.8,
  ),
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
  assistantMessage(
    "debug:assistant-after-status",
    "The branch already has the activity-feed work stacked, so I’m going to keep this as a fixture-only pass and avoid touching the live transcript renderer.",
    8.8,
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
      "@Agent picked it up — testing the full taxonomy now.",
      "buzz-cli:messages.send",
      {
        operation: "messages.send",
        object: "@Agent picked it up — testing the full taxonomy now.",
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
        'buzz messages send --channel buzz-agent-observability --content "@Agent picked it up — testing the full taxonomy now."',
      workdir: workspacePath,
      timeout_ms: 120000,
    },
    result: shellResultJson(
      '{"accepted":true,"event_id":"debug-openable-message"}\n',
    ),
    isError: false,
    timestamp: timestamp(12),
    startedAt: timestamp(12),
    completedAt: timestamp(13),
    acpSource: "tool_call_update",
    turnId,
    sessionId,
    channelId,
  },
  userMessage(
    "debug:user-followup-raw",
    reviewerPubkey,
    "Avery",
    "Could you include one of the raw payload examples in the middle of the turn? That’s where I usually need to compare the compact row with the wire event.",
    13.4,
  ),
  debugPlanUpdateItem(
    "debug:plan-after-inspection",
    "1. [x] Read the transcript components.\n2. [x] Classify the observed tools.\n3. [ ] Patch the Activity header.\n4. [ ] Run desktop gates and report the pushed SHA.",
    13.6,
  ),
  assistantMessage(
    "debug:assistant-before-edits",
    "Good call. I’ll leave a raw ACP sample in the fixture and make the surrounding messages look like a real debugging exchange.",
    13.8,
  ),
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
    180,
  ),
  fileEditItem(
    "debug:file-edit-managed-panel-1",
    "desktop/src/features/agents/ui/ManagedAgentSessionPanel.tsx",
    15.6,
    530,
  ),
  fileEditItem(
    "debug:file-edit-transcript-list-1",
    "desktop/src/features/agents/ui/AgentSessionTranscriptList.tsx",
    16,
    260,
  ),
  fileEditItem(
    "debug:file-edit-grouping-1",
    "desktop/src/features/agents/ui/agentSessionTranscriptGrouping.ts",
    16.4,
    740,
  ),
  fileEditItem(
    "debug:file-edit-tool-item-1",
    "desktop/src/features/agents/ui/AgentSessionToolItem.tsx",
    16.8,
    310,
  ),
  fileEditItem(
    "debug:file-edit-tool-summary-1",
    "desktop/src/features/agents/ui/agentSessionToolSummary.ts",
    17.2,
    480,
  ),
  fileEditItem(
    "debug:file-edit-classifier-1",
    "desktop/src/features/agents/ui/agentSessionToolClassifier.ts",
    17.6,
    690,
  ),
  debugPlanUpdateItem(
    "debug:plan-after-edits",
    "1. [x] Read the transcript components.\n2. [x] Classify the observed tools.\n3. [x] Patch the Activity header.\n4. [ ] Run desktop gates and report the pushed SHA.",
    18,
  ),
  shellCommandItem(
    "debug:shell-burst-1",
    "git status --short",
    18.2,
    "M desktop/src/features/agents/ui/ManagedAgentSessionPanel.tsx\n",
    96,
  ),
  shellCommandItem(
    "debug:shell-burst-2",
    "pnpm --dir desktop lint",
    18.6,
    "Checked 812 files in 1.9s. No fixes applied.\n",
    1900,
  ),
  shellCommandItem(
    "debug:shell-burst-3",
    "node --import ./test-loader.mjs --experimental-strip-types --test src/features/agents/ui/debugAgentActivityFixture.test.mjs",
    19,
    "1 test passed\n",
    183,
  ),
  shellCommandItem(
    "debug:shell-burst-4",
    "pnpm --dir desktop typecheck",
    19.4,
    "Typecheck completed.\n",
    3600,
  ),
  shellCommandItem(
    "debug:shell-burst-5",
    "git diff --stat",
    19.8,
    "4 files changed, 211 insertions(+), 38 deletions(-)\n",
    122,
  ),
  debugPlanUpdateItem(
    "debug:plan-after-gates",
    "1. [x] Read the transcript components.\n2. [x] Classify the observed tools.\n3. [x] Patch the Activity header.\n4. [x] Run desktop gates.\n5. [ ] Report the pushed SHA.",
    19.9,
  ),
  assistantMessage(
    "debug:assistant-mid-check",
    "The grouped command burst is now doing the thing I wanted: it compresses repetitive shell checks, but each child still has distinct stdout and timing when expanded.",
    20,
  ),
  shellCommandItem(
    "debug:shell-burst-6",
    "git add desktop/src/features/agents/ui desktop/src/features/channels/ui",
    20.2,
    "",
    140,
  ),
  shellCommandItem(
    "debug:shell-burst-7",
    'git commit -m "feat(desktop): add activity render-class debug fixture"',
    20.6,
    "[tho/activity-feed-rebuild aa84200ad] feat(desktop): add activity render-class debug fixture\n",
    5530,
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
  userMessage(
    "debug:user-followup-finish",
    userPubkey,
    "Taylor Ho",
    "After the push check, include the exact branch and SHA in the channel message. That final report bubble is a common case.",
    21.7,
  ),
  fileEditItem("debug:single-edit-vision-doc", "VISION_ACTIVITY.md", 21.9, 210),
  todoUpdateItem(
    "debug:todo-after-first-check",
    "Inspect transcript panel settings and debug activity fixture",
    22,
  ),
  shellCommandItem(
    "debug:shell-push-burst-1",
    "git status --short",
    22.8,
    "",
    88,
  ),
  shellCommandItem(
    "debug:shell-push-burst-2",
    "git push -u origin HEAD",
    23.2,
    "branch 'tho/activity-feed-rebuild' set up to track 'origin/tho/activity-feed-rebuild'.\n",
    5400,
  ),
  shellCommandItem(
    "debug:shell-push-burst-3",
    "git rev-parse --short=40 HEAD",
    23.6,
    "aa84200ad266d16f81da2f9c347518a7525a3ef4\n",
    74,
  ),
  debugPlanUpdateItem(
    "debug:plan-after-push",
    "1. [x] Read the transcript components.\n2. [x] Classify the observed tools.\n3. [x] Patch the Activity header.\n4. [x] Run desktop gates.\n5. [x] Report the pushed SHA.",
    24,
  ),
  todoUpdateItem(
    "debug:todo-after-push",
    "Report branch, SHA, and commit to the requester",
    24.4,
  ),
  assistantMessage(
    "debug:assistant-reporting",
    "Everything is ready to report. I’m going to send a concise channel update with branch, SHA, and commit title, then stop.",
    24.8,
  ),
  fileEditItem(
    "debug:single-edit-activity-row",
    "desktop/src/features/agents/ui/activityRenderClasses/ActivityRow.tsx",
    25,
    160,
  ),
  {
    id: "debug:message-tool-pushed-report",
    type: "tool",
    renderClass: "message",
    descriptor: descriptor(
      "message",
      "Send Message",
      "@Agent Done and pushed.\n\nBranch: `tho/activity-feed-rebuild`\nSHA: `aa84200ad266d16f81da2f9c347518a7525a3ef4`\nCommit: `feat(desktop): add activity render-class debug fixture`",
      "buzz-cli:messages.send",
      {
        operation: "messages.send",
        object: "@Agent Done and pushed.",
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
        'buzz messages send --channel agents --content "@Agent Done and pushed."',
      workdir: workspacePath,
      timeout_ms: 120000,
    },
    result: shellResultJson(
      '{"accepted":true,"event_id":"debug-openable-pushed-report"}\n',
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
