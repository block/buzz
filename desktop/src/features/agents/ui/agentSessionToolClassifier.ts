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
  const tokens = tokenizeShellCommandTokens(command);
  const range = findBuzzCommand(tokens);
  if (!range) return null;

  const group = tokens[range.groupIndex].value;
  const verb = tokens[range.verbIndex]?.value ?? "run";
  const operation = `${group}.${verb}`;
  const isSend = group === "messages" && verb === "send";
  const preview = isSend
    ? extractBuzzCliInlineContent(tokens, range)
    : extractBuzzCliObjectPreview(tokens, range);
  const tone = buzzCliTone(group, verb);
  return {
    renderClass: isSend ? "message" : "relay-op",
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

function extractBuzzCliInlineContent(
  tokens: ShellToken[],
  range: BuzzCommandRange,
): string | null {
  const content = getFlagValue(tokens, range.verbIndex + 1, "--content");
  if (!content || content.value === "-") return null;
  // Quoting decides this, not the characters: a single-quoted or escaped `$` /
  // backtick is literal text the shell never touched, so it is safe to show.
  // Anything the shell could have expanded is dropped, as #2201 intended.
  if (content.hasSubstitution) return null;
  return content.value;
}

function extractBuzzCliObjectPreview(
  tokens: ShellToken[],
  range: BuzzCommandRange,
): string | null {
  const flagPreview =
    getFlagValue(tokens, range.verbIndex + 1, "--channel") ??
    getFlagValue(tokens, range.verbIndex + 1, "--event") ??
    getFlagValue(tokens, range.verbIndex + 1, "--query") ??
    getFlagValue(tokens, range.verbIndex + 1, "--name") ??
    getFlagValue(tokens, range.verbIndex + 1, "--file");
  if (flagPreview) return flagPreview.value;

  const next = tokens[range.verbIndex + 1];
  return next && !isCommandSeparator(next.value) && !next.value.startsWith("-")
    ? next.value
    : null;
}

type BuzzCommandRange = {
  buzzIndex: number;
  groupIndex: number;
  verbIndex: number;
};

function findBuzzCommand(tokens: ShellToken[]): BuzzCommandRange | null {
  for (let i = 0; i < tokens.length; i++) {
    if (!isBuzzExecutable(tokens[i].value)) continue;

    for (let j = i + 1; j < tokens.length; j++) {
      const token = tokens[j].value;
      if (isCommandSeparator(token)) break;
      if (token.startsWith("-")) {
        if (
          !token.includes("=") &&
          tokens[j + 1]?.value.startsWith("-") === false
        ) {
          j += 1;
        }
        continue;
      }
      if (!BUZZ_CLI_GROUPS.has(token)) continue;
      const verbIndex = j + 1;
      if (!tokens[verbIndex] || isCommandSeparator(tokens[verbIndex].value)) {
        return null;
      }
      return { buzzIndex: i, groupIndex: j, verbIndex };
    }
  }
  return null;
}

/**
 * A token plus whether the shell could have substituted anything into it.
 *
 * `hasSubstitution` is what quote context buys us: `$` and a backtick mean
 * expansion when they are unquoted or inside double quotes, and mean nothing at
 * all inside single quotes. Without this distinction a markdown code span in
 * `--content 'use `pnpm test`'` is indistinguishable from a real
 * `--content "$(cat file)"`, and both have to be discarded.
 *
 * Only single quoting is trusted, because only single quoting is literal in
 * every shell `BUZZ_SHELL` supports. A backslash escape is not: PowerShell
 * keeps the backslash and still expands `"literal \$MESSAGE"`. So escaped
 * substitution characters stay flagged even though bash would treat them as
 * literal text.
 *
 * The `$` rule is deliberately conservative in the same direction — any `$`
 * outside single quotes flags the token, even where bash would leave it literal
 * (a trailing `$`, or `$` before a space). Displaying an unexpanded shell
 * expression as if it were the sent text is the failure this guard exists to
 * prevent, so it errs towards dropping the preview.
 */
export type ShellToken = {
  value: string;
  hasSubstitution: boolean;
};

export function tokenizeShellCommand(command: string): string[] {
  return tokenizeShellCommandTokens(command).map((token) => token.value);
}

export function tokenizeShellCommandTokens(command: string): ShellToken[] {
  const tokens: ShellToken[] = [];
  let current = "";
  let hasSubstitution = false;
  let quote: "'" | '"' | null = null;
  let escaping = false;

  const pushCurrent = () => {
    if (current.length > 0) {
      tokens.push({ value: current, hasSubstitution });
      current = "";
    }
    hasSubstitution = false;
  };

  for (const char of command) {
    if (escaping) {
      // The value is built as bash would build it, but a backslash escape does
      // not earn substitution credit: backslash is not an escape character in
      // PowerShell (`"literal \$MESSAGE"` keeps the backslash AND expands the
      // variable) or cmd, and `BUZZ_SHELL` explicitly supports both. Only
      // single quoting is literal across all of them, so escape forms stay
      // conservative and keep #2201's behaviour.
      if (isSubstitutionChar(char)) hasSubstitution = true;
      current += char;
      escaping = false;
      continue;
    }
    if (char === "\\" && quote !== "'") {
      escaping = true;
      continue;
    }
    if (quote) {
      if (char === quote) {
        quote = null;
      } else {
        if (quote === '"' && isSubstitutionChar(char)) hasSubstitution = true;
        current += char;
      }
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
      tokens.push({ value: char, hasSubstitution: false });
      continue;
    }
    if (isSubstitutionChar(char)) hasSubstitution = true;
    current += char;
  }

  if (escaping) current += "\\";
  pushCurrent();
  return tokens;
}

function isSubstitutionChar(char: string) {
  return char === "$" || char === "`";
}

function isBuzzExecutable(token: string) {
  return token === "buzz" || token.split(/[\\/]/).pop() === "buzz";
}

function isCommandSeparator(token: string) {
  return token === "|" || token === ";" || token === "&";
}

function getFlagValue(
  tokens: ShellToken[],
  start: number,
  flag: string,
): ShellToken | null {
  for (let i = start; i < tokens.length; i++) {
    const token = tokens[i];
    if (isCommandSeparator(token.value)) return null;
    if (token.value === flag) {
      return tokens[i + 1] && !isCommandSeparator(tokens[i + 1].value)
        ? tokens[i + 1]
        : null;
    }
    if (token.value.startsWith(`${flag}=`)) {
      // The substitution flag is per token, so `--content=$X` carries it here
      // too; slicing the value off must not lose it.
      return {
        value: token.value.slice(flag.length + 1),
        hasSubstitution: token.hasSubstitution,
      };
    }
  }
  return null;
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
