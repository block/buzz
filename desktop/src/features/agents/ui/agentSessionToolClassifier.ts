import type {
  AgentActivityAction,
  AgentActivityDescriptor,
  AgentActivityRenderClass,
  AgentActivityTone,
  TranscriptItem,
} from "./agentSessionTypes";
import {
  formatToolTitle,
  getBuzzToolInfo,
  normalizeToolNameText,
} from "./agentSessionToolCatalog";
import {
  asRecord,
  getToolString,
  getToolStringList,
} from "./agentSessionUtils";

type ToolItem = Extract<TranscriptItem, { type: "tool" }>;

export type ToolClassificationInput = {
  title: string;
  toolName: string;
  buzzToolName: string | null;
  args: Record<string, unknown>;
  result: string;
  isError: boolean;
};

type ToolClassifierProvider = (
  input: ToolClassificationInput,
) => AgentActivityDescriptor | null;

const DEVELOPER_TOOL_BASES = new Set([
  "shell",
  "read_file",
  "view_image",
  "str_replace",
  "todo",
  "stop",
  "postcompact",
]);

const BUZZ_CLI_GROUPS = new Set([
  "messages",
  "channels",
  "dms",
  "reactions",
  "canvas",
  "feed",
  "users",
  "workflows",
  "social",
  "repos",
  "upload",
  "mem",
  "notes",
  "patches",
  "pr",
  "issues",
  "emoji",
  "pack",
]);

const BUZZ_CLI_ADMIN_VERBS = new Set([
  "archive",
  "unarchive",
  "create",
  "delete",
  "remove",
  "add-channel-member",
  "remove-channel-member",
  "set-channel-add-policy",
]);

const BUZZ_CLI_READ_VERBS = new Set([
  "get",
  "list",
  "thread",
  "search",
  "members",
  "runs",
  "notes",
]);

const TOOL_CLASS_LABELS: Record<AgentActivityRenderClass, string> = {
  message: "Message",
  "relay-op": "Buzz relay op",
  "file-edit": "File edit",
  "file-read": "File read",
  "skill-read": "Skill read",
  image: "Image",
  shell: "Shell command",
  status: "Status",
  thought: "Thought",
  plan: "Plan",
  permission: "Permission",
  error: "Error",
  generic: "Tool",
  "raw-rail": "Raw event",
  suppressed: "Suppressed",
};

const providers: ToolClassifierProvider[] = [
  classifyLoadSkillTool,
  classifyDeveloperHarnessTool,
  classifyBuzzTool,
];

export function classifyTool(
  input: ToolClassificationInput,
): AgentActivityDescriptor {
  for (const provider of providers) {
    const descriptor = provider(input);
    if (descriptor) {
      return input.isError || descriptor.renderClass === "error"
        ? {
            ...descriptor,
            renderClass: "error",
            label: descriptor.label.endsWith("failed")
              ? descriptor.label
              : `${descriptor.label} failed`,
          }
        : descriptor;
    }
  }

  return genericDescriptor(input);
}

export function classifyToolItem(item: ToolItem): AgentActivityDescriptor {
  return classifyTool({
    title: item.title,
    toolName: item.toolName,
    buzzToolName: item.buzzToolName,
    args: item.args,
    result: item.result,
    isError: item.isError,
  });
}

export function renderClassLabel(renderClass: AgentActivityRenderClass) {
  return TOOL_CLASS_LABELS[renderClass];
}

function classifyLoadSkillTool(
  input: ToolClassificationInput,
): AgentActivityDescriptor | null {
  const isLoadSkill = [input.toolName, input.title, input.buzzToolName].some(
    (value) => value && normalizeToolNameText(value) === "load_skill",
  );
  if (!isLoadSkill) return null;

  const skillRef = getToolString(input.args, ["name"]);
  const object = skillRef ?? "skill";
  const isSupportingFile = skillRef?.includes("/") ?? false;

  return {
    renderClass: "skill-read",
    label: isSupportingFile ? "Read skill file" : "Read skill",
    preview: skillRef,
    action: { verb: "Read", object },
    source: "harness",
    groupKey: isSupportingFile ? "skill:load-file" : "skill:load",
  };
}

function classifyDeveloperHarnessTool(
  input: ToolClassificationInput,
): AgentActivityDescriptor | null {
  const kind = resolveDeveloperToolKind(input);
  if (!kind) return null;

  if (kind === "shell") {
    const command = getToolString(input.args, ["command"]);
    const buzzCli = command ? parseBuzzCliCommand(command) : null;
    if (buzzCli) {
      return buzzCli;
    }
    return {
      renderClass: "shell",
      label: "Ran command",
      preview: command,
      action: { verb: "Ran", object: command ?? "command" },
      source: "harness",
      groupKey: "shell:command",
    };
  }

  if (kind === "read_file") {
    const path = getToolString(input.args, ["path"]);
    return {
      renderClass: "file-read",
      label: "Read file",
      preview: path,
      action: { verb: "Read", object: path ?? "file" },
      source: "harness",
      groupKey: "read_file",
    };
  }

  if (kind === "view_image") {
    const source = getToolString(input.args, ["source"]);
    return {
      renderClass: "image",
      label: "Viewed image",
      preview: source ? basenameOrUrl(source) : null,
      action: {
        verb: "Viewed",
        object: source ? basenameOrUrl(source) : "image",
      },
      source: "harness",
      groupKey: "view_image",
    };
  }

  if (kind === "str_replace") {
    const path = getToolString(input.args, ["path"]);
    return {
      renderClass: "file-edit",
      label: "Edited file",
      preview: path,
      action: { verb: "Edited", object: path ?? "file" },
      source: "harness",
      groupKey: "file-edit:str_replace",
    };
  }

  if (kind === "todo") {
    const preview = getTodoPreview(input.args);
    return {
      renderClass: "plan",
      label: "Updated todos",
      preview,
      action: { verb: "Updated", object: preview },
      source: "harness",
      groupKey: "plan:todo",
    };
  }

  if (kind === "stop_hook") {
    return {
      renderClass: "suppressed",
      label: "Checked todos",
      preview: null,
      action: { verb: "Checked", object: "todos" },
      source: "harness",
      groupKey: "suppressed:stop-hook",
    };
  }

  if (kind === "post_compact_hook") {
    return {
      renderClass: "status",
      label: "Context compacted",
      preview: null,
      action: { verb: "Compacted", object: "context" },
      source: "harness",
      groupKey: "status:post-compact",
    };
  }

  const preview = genericPreview(input);
  return {
    renderClass: "generic",
    label: "Ran tool",
    preview,
    action: { verb: "Ran", object: preview ?? "tool" },
    source: "harness",
    groupKey: "generic:dev-mcp",
  };
}

function classifyBuzzTool(
  input: ToolClassificationInput,
): AgentActivityDescriptor | null {
  const name = [input.buzzToolName, input.toolName, input.title].find(
    (value) => value && getBuzzToolInfo(value),
  );
  if (!name) return null;

  const info = getBuzzToolInfo(name);
  if (!info) return null;

  const operation = normalizeToolNameText(name);
  const label = formatToolTitle(name, input.title);
  const preview = extractBuzzToolPreview(input.args);
  return {
    renderClass: isBuzzMessageSend(operation) ? "message" : "relay-op",
    label,
    preview,
    action: actionForBuzzOperation(operation, preview, info.tone),
    tone: info.tone,
    operation,
    object: preview,
    source: "mcp",
    groupKey: `buzz:${operation}`,
  };
}

function genericDescriptor(
  input: ToolClassificationInput,
): AgentActivityDescriptor {
  const preview = genericPreview(input);
  return {
    renderClass: "generic",
    label: "Ran tool",
    preview,
    action: { verb: "Ran", object: preview ?? "tool" },
    source: "fallback",
    groupKey: `generic:${normalizeToolNameText(input.toolName || input.title)}`,
  };
}

function resolveDeveloperToolKind(
  input: ToolClassificationInput,
):
  | "shell"
  | "read_file"
  | "view_image"
  | "str_replace"
  | "todo"
  | "stop_hook"
  | "post_compact_hook"
  | "dev_mcp"
  | null {
  for (const value of [input.toolName, input.title, input.buzzToolName]) {
    const kind = classifyDeveloperToolName(value);
    if (kind) return kind;
  }
  return null;
}

function classifyDeveloperToolName(value: string | null | undefined) {
  if (!value) return null;

  const normalized = normalizeToolNameText(value);
  const base = normalized.replace(/^buzz_dev_mcp_/, "");

  if (base === "shell" || normalized.endsWith("_shell")) return "shell";
  if (base === "read_file" || normalized.endsWith("_read_file"))
    return "read_file";
  if (base === "view_image" || normalized.endsWith("_view_image"))
    return "view_image";
  if (base === "str_replace" || normalized.endsWith("_str_replace"))
    return "str_replace";
  if (base === "todo") return "todo";
  if (base === "stop") return "stop_hook";
  if (base === "postcompact") return "post_compact_hook";
  if (DEVELOPER_TOOL_BASES.has(base) || normalized.includes("buzz_dev_mcp")) {
    return "dev_mcp";
  }
  return null;
}

export function parseBuzzCliCommand(
  command: string,
): AgentActivityDescriptor | null {
  const tokens = tokenizeShellCommand(command);
  const range = findBuzzCommand(tokens);
  if (!range) return null;

  const group = tokens[range.groupIndex];
  const verb = tokens[range.verbIndex] ?? "run";
  const operation = `${group}.${verb}`;
  const content =
    group === "messages" && verb === "send"
      ? extractBuzzCliSendMessageContent(tokens, range)
      : null;
  const preview = content ?? extractBuzzCliObjectPreview(tokens, range);
  const tone = buzzCliTone(group, verb);
  return {
    renderClass:
      group === "messages" && verb === "send" ? "message" : "relay-op",
    label: titleForBuzzCli(group, verb),
    preview,
    action: actionForBuzzOperation(operation, preview, tone),
    tone,
    operation,
    object: preview,
    source: "shell",
    groupKey: `buzz-cli:${operation}`,
  };
}

function titleForBuzzCli(group: string, verb: string) {
  if (group === "messages" && verb === "send") return "Send Message";
  return [group, verb]
    .map((part) =>
      part
        .split(/[-_]+/)
        .filter(Boolean)
        .map((word) => word.charAt(0).toUpperCase() + word.slice(1))
        .join(" "),
    )
    .filter(Boolean)
    .join(" ");
}

function actionForBuzzOperation(
  operation: string,
  object: string | null,
  tone: AgentActivityTone,
): AgentActivityAction {
  const verb = buzzOperationVerbToken(operation);
  return {
    verb: buzzOperationVerb(verb, tone),
    object: object ?? buzzOperationObject(operation),
  };
}

function buzzOperationVerbToken(operation: string) {
  if (operation.includes(".")) {
    return operation.split(".")[1] ?? "run";
  }
  return operation.split("_")[0] ?? "run";
}

function buzzOperationVerb(verb: string, tone: AgentActivityTone) {
  if (verb === "add") return "Added";
  if (verb === "archive") return "Archived";
  if (verb === "create") return "Created";
  if (verb === "delete") return "Deleted";
  if (verb === "get" || verb === "list" || verb === "members") return "Read";
  if (verb === "remove") return "Removed";
  if (verb === "runs") return "Read";
  if (verb === "search") return "Searched";
  if (verb === "send") return "Sent";
  if (verb === "thread") return "Read";
  if (verb === "unarchive") return "Unarchived";
  if (tone === "read") return "Read";
  return "Updated";
}

function buzzOperationObject(operation: string) {
  if (isBuzzMessageSend(operation)) return "message";
  if (operation.includes(".")) {
    const [group] = operation.split(".");
    return group ? group.replace(/[-_]+/g, " ") : "Buzz";
  }
  const object = operation.replace(
    /^(add|approve|archive|create|delete|edit|get|hide|join|leave|list|open|publish|remove|search|send|set|trigger|unarchive|update|vote)_/,
    "",
  );
  return object ? object.replace(/[-_]+/g, " ") : "Buzz";
}

function buzzCliTone(group: string, verb: string): AgentActivityTone {
  if (BUZZ_CLI_ADMIN_VERBS.has(verb)) return "admin";
  if (BUZZ_CLI_READ_VERBS.has(verb)) return "read";
  if (group === "feed" && verb === "get") return "read";
  return "write";
}

function extractBuzzCliSendMessageContent(
  tokens: string[],
  range: BuzzCommandRange,
): string | null {
  const content = getFlagValue(tokens, range.verbIndex + 1, "--content");
  if (!content) return null;
  if (content !== "-") return content;
  return extractSimpleEchoPipeContent(tokens, range.buzzIndex) ?? null;
}

function extractBuzzCliObjectPreview(
  tokens: string[],
  range: BuzzCommandRange,
): string | null {
  const flagPreview =
    getFlagValue(tokens, range.verbIndex + 1, "--channel") ??
    getFlagValue(tokens, range.verbIndex + 1, "--event") ??
    getFlagValue(tokens, range.verbIndex + 1, "--query") ??
    getFlagValue(tokens, range.verbIndex + 1, "--name") ??
    getFlagValue(tokens, range.verbIndex + 1, "--file");
  if (flagPreview) return flagPreview;

  const next = tokens[range.verbIndex + 1];
  return next && !isCommandSeparator(next) && !next.startsWith("-")
    ? next
    : null;
}

type BuzzCommandRange = {
  buzzIndex: number;
  groupIndex: number;
  verbIndex: number;
};

function findBuzzCommand(tokens: string[]): BuzzCommandRange | null {
  for (let i = 0; i < tokens.length; i++) {
    if (!isBuzzExecutable(tokens[i])) continue;

    for (let j = i + 1; j < tokens.length; j++) {
      if (isCommandSeparator(tokens[j])) break;
      if (tokens[j].startsWith("-")) {
        if (
          !tokens[j].includes("=") &&
          tokens[j + 1]?.startsWith("-") === false
        ) {
          j += 1;
        }
        continue;
      }
      if (!BUZZ_CLI_GROUPS.has(tokens[j])) continue;
      const verbIndex = j + 1;
      if (!tokens[verbIndex] || isCommandSeparator(tokens[verbIndex])) {
        return null;
      }
      return { buzzIndex: i, groupIndex: j, verbIndex };
    }
  }
  return null;
}

export function tokenizeShellCommand(command: string): string[] {
  const tokens: string[] = [];
  let current = "";
  let quote: "'" | '"' | null = null;
  let escaping = false;

  const pushCurrent = () => {
    if (current.length > 0) {
      tokens.push(current);
      current = "";
    }
  };

  for (const char of command) {
    if (escaping) {
      current += char;
      escaping = false;
      continue;
    }
    if (char === "\\" && quote !== "'") {
      escaping = true;
      continue;
    }
    if (quote) {
      if (char === quote) quote = null;
      else current += char;
      continue;
    }
    if (char === "'" || char === '"') {
      quote = char;
      continue;
    }
    if (/\s/.test(char)) {
      pushCurrent();
      continue;
    }
    if (char === "|" || char === ";" || char === "&") {
      pushCurrent();
      tokens.push(char);
      continue;
    }
    current += char;
  }

  if (escaping) current += "\\";
  pushCurrent();
  return tokens;
}

function isBuzzExecutable(token: string) {
  return token === "buzz" || token.split(/[\\/]/).pop() === "buzz";
}

function isCommandSeparator(token: string) {
  return token === "|" || token === ";" || token === "&";
}

function getFlagValue(tokens: string[], start: number, flag: string) {
  for (let i = start; i < tokens.length; i++) {
    const token = tokens[i];
    if (isCommandSeparator(token)) return null;
    if (token === flag) {
      return tokens[i + 1] && !isCommandSeparator(tokens[i + 1])
        ? tokens[i + 1]
        : null;
    }
    if (token.startsWith(`${flag}=`)) return token.slice(flag.length + 1);
  }
  return null;
}

export function extractSimpleEchoPipeContent(
  tokens: string[],
  buzzIndex: number,
): string | null {
  const pipeIndex = tokens.lastIndexOf("|", buzzIndex);
  if (pipeIndex <= 0) return null;
  const echoStart = findSegmentStart(tokens, pipeIndex - 1);
  const leftSegment = tokens.slice(echoStart, pipeIndex);
  if (leftSegment[0] !== "echo") return null;
  const contentTokens = leftSegment
    .slice(1)
    .filter((token) => !token.startsWith("-"));
  return contentTokens.length > 0 ? contentTokens.join(" ") : null;
}

function findSegmentStart(tokens: string[], beforeIndex: number) {
  for (let i = beforeIndex; i >= 0; i--) {
    if (isCommandSeparator(tokens[i])) return i + 1;
  }
  return 0;
}

function extractBuzzToolPreview(args: Record<string, unknown>): string | null {
  const content = getToolString(args, ["content", "message", "text", "body"]);
  if (content) return content;
  const query = getToolString(args, ["query", "search"]);
  if (query) return query;
  const channelId = getToolString(args, ["channel_id", "channelId"]);
  if (channelId) return channelId;
  const workflowId = getToolString(args, ["workflow_id", "workflowId"]);
  if (workflowId) return workflowId;
  const pubkeys = getToolStringList(args, ["pubkeys", "pubkey"]);
  if (pubkeys.length === 1) return pubkeys[0];
  if (pubkeys.length > 1) return `${pubkeys.length} users`;
  return getToolString(args, ["event_id", "eventId", "name"]);
}

function genericPreview(input: ToolClassificationInput): string | null {
  return (
    getToolString(input.args, [
      "command",
      "path",
      "source",
      "query",
      "name",
      "content",
      "message",
    ]) ?? (input.title ? input.title : null)
  );
}

function isBuzzMessageSend(operation: string) {
  return operation === "send_message" || operation === "messages_send";
}

function basenameOrUrl(source: string): string {
  const trimmed = source.trim();
  if (
    trimmed.startsWith("data:image/") ||
    trimmed.startsWith("http://") ||
    trimmed.startsWith("https://")
  ) {
    return trimmed;
  }
  return trimmed.split(/[/\\]/).pop() ?? trimmed;
}

function getTodoPreview(args: Record<string, unknown>): string | null {
  const todos = args.todos;
  if (!Array.isArray(todos)) return "todo list";
  if (todos.length === 0) return "empty list";
  const first = todos[0];
  const firstText =
    first && typeof first === "object"
      ? getToolString(asRecord(first), ["text"])
      : null;
  if (firstText)
    return todos.length > 1 ? `${firstText} (+${todos.length - 1})` : firstText;
  return `${todos.length} item${todos.length === 1 ? "" : "s"}`;
}
