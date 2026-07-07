import * as React from "react";

const STORAGE_PREFIX = "buzz:chat-work-binding:v2";
const STORAGE_EVENT = "buzz:chat-work-binding-changed";
const CONTEXT_SENT_PREFIX = "buzz:chat-work-context-sent:v2";

export type ChatWorkBinding = {
  projectName: string | null;
  projectPath: string | null;
  branch: string | null;
  prHref: string | null;
  /**
   * True when the user explicitly disconnected the chat from PR monitoring.
   * Automatic PR discovery/link parsing must not repopulate `prHref` while
   * this is set.
   */
  prDetached: boolean;
  updatedAt: number;
};

export type ChatWorkBindingPatch = Partial<
  Pick<
    ChatWorkBinding,
    "projectName" | "projectPath" | "branch" | "prHref" | "prDetached"
  >
>;

export type MergeChatWorkBindingOptions = {
  now?: number;
  replaceProject?: boolean;
  replaceBranch?: boolean;
  replacePr?: boolean;
};

const EMPTY_BINDING: ChatWorkBinding = {
  projectName: null,
  projectPath: null,
  branch: null,
  prHref: null,
  prDetached: false,
  updatedAt: 0,
};

function storageKey(chatId: string) {
  return `${STORAGE_PREFIX}:${chatId}`;
}

function contextSentKey(chatId: string) {
  return `${CONTEXT_SENT_PREFIX}:${chatId}`;
}

function hasOwn<T extends object>(value: T, key: keyof T) {
  return Object.keys(value).includes(String(key));
}

function cleanString(value: unknown) {
  return typeof value === "string" && value.trim() ? value.trim() : null;
}

function normalizeBinding(value: Partial<ChatWorkBinding>): ChatWorkBinding {
  return {
    projectName: cleanString(value.projectName),
    projectPath: cleanString(value.projectPath),
    branch: cleanString(value.branch),
    prHref: cleanString(value.prHref),
    prDetached: Boolean(value.prDetached),
    updatedAt: Number.isFinite(value.updatedAt) ? Number(value.updatedAt) : 0,
  };
}

export function isEmptyChatWorkBinding(binding: ChatWorkBinding | null) {
  return (
    binding === null ||
    (!binding.projectName &&
      !binding.projectPath &&
      !binding.branch &&
      !binding.prHref &&
      !binding.prDetached)
  );
}

function bindingsEqual(
  left: ChatWorkBinding | null,
  right: ChatWorkBinding | null,
) {
  if (left === right) {
    return true;
  }
  if (!left || !right) {
    return false;
  }
  return (
    left.projectName === right.projectName &&
    left.projectPath === right.projectPath &&
    left.branch === right.branch &&
    left.prHref === right.prHref &&
    left.prDetached === right.prDetached
  );
}

export function mergeChatWorkBinding(
  current: ChatWorkBinding | null,
  patch: ChatWorkBindingPatch,
  options: MergeChatWorkBindingOptions = {},
): ChatWorkBinding | null {
  const now = options.now ?? Date.now();
  const next: ChatWorkBinding = {
    ...(current ?? EMPTY_BINDING),
    updatedAt: current?.updatedAt ?? 0,
  };

  if (options.replaceProject || !current?.projectName) {
    if (hasOwn(patch, "projectName")) {
      next.projectName = cleanString(patch.projectName);
    }
  }
  if (options.replaceProject || !current?.projectPath) {
    if (hasOwn(patch, "projectPath")) {
      next.projectPath = cleanString(patch.projectPath);
    }
  }

  if (options.replaceBranch || !current?.branch) {
    if (hasOwn(patch, "branch")) {
      next.branch = cleanString(patch.branch);
    }
  }

  if (options.replacePr || (!current?.prHref && !current?.prDetached)) {
    if (hasOwn(patch, "prHref")) {
      next.prHref = cleanString(patch.prHref);
    }
    if (hasOwn(patch, "prDetached")) {
      next.prDetached = Boolean(patch.prDetached);
    } else if (next.prHref) {
      next.prDetached = false;
    }
  }

  const normalized = normalizeBinding({ ...next, updatedAt: now });
  return isEmptyChatWorkBinding(normalized) ? null : normalized;
}

export function readChatWorkBinding(chatId: string): ChatWorkBinding | null {
  if (typeof window === "undefined") {
    return null;
  }
  try {
    const raw = window.localStorage.getItem(storageKey(chatId));
    if (!raw) {
      return null;
    }
    const normalized = normalizeBinding(
      JSON.parse(raw) as Partial<ChatWorkBinding>,
    );
    return isEmptyChatWorkBinding(normalized) ? null : normalized;
  } catch {
    return null;
  }
}

export function writeChatWorkBinding(
  chatId: string,
  binding: ChatWorkBinding | null,
) {
  if (typeof window === "undefined") {
    return;
  }
  try {
    if (isEmptyChatWorkBinding(binding)) {
      window.localStorage.removeItem(storageKey(chatId));
    } else {
      window.localStorage.setItem(storageKey(chatId), JSON.stringify(binding));
    }
    window.dispatchEvent(new CustomEvent(STORAGE_EVENT));
  } catch {
    // Chat work binding is local convenience state; ignore storage failures.
  }
}

export function updateChatWorkBinding(
  chatId: string,
  patch: ChatWorkBindingPatch,
  options?: MergeChatWorkBindingOptions,
) {
  const current = readChatWorkBinding(chatId);
  const next = mergeChatWorkBinding(current, patch, options);
  if (!bindingsEqual(current, next)) {
    writeChatWorkBinding(chatId, next);
  }
  return next;
}

export function clearChatWorkBindingPr(chatId: string, href?: string | null) {
  const current = readChatWorkBinding(chatId);
  if (!current) {
    return null;
  }
  if (href && current.prHref !== href) {
    return current;
  }
  return updateChatWorkBinding(
    chatId,
    { prHref: null, prDetached: false },
    { replacePr: true },
  );
}

type ChatWorkBindingState = {
  chatId: string | null;
  binding: ChatWorkBinding | null;
};

export function useChatWorkBinding(chatId: string | null | undefined) {
  const normalizedChatId = chatId ?? null;
  const [state, setState] = React.useState<ChatWorkBindingState>(() => ({
    chatId: normalizedChatId,
    binding: normalizedChatId ? readChatWorkBinding(normalizedChatId) : null,
  }));

  const currentBinding =
    state.chatId === normalizedChatId
      ? state.binding
      : normalizedChatId
        ? readChatWorkBinding(normalizedChatId)
        : null;

  React.useEffect(() => {
    const refresh = () => {
      const next = normalizedChatId
        ? readChatWorkBinding(normalizedChatId)
        : null;
      setState((current) =>
        current.chatId === normalizedChatId &&
        bindingsEqual(current.binding, next)
          ? current
          : { chatId: normalizedChatId, binding: next },
      );
    };
    refresh();
    window.addEventListener(STORAGE_EVENT, refresh);
    window.addEventListener("storage", refresh);
    return () => {
      window.removeEventListener(STORAGE_EVENT, refresh);
      window.removeEventListener("storage", refresh);
    };
  }, [normalizedChatId]);

  return currentBinding;
}

export function buildChatWorkContext(binding: ChatWorkBinding | null) {
  if (!binding || (!binding.branch && !binding.prHref)) {
    return null;
  }

  const lines = [
    "Work context",
    "Scope: this chat is isolated to the work item below. Do not use branches, pull requests, or status from other chats unless the user explicitly asks to switch.",
  ];
  if (binding.projectName) {
    lines.push(`Project: ${binding.projectName}`);
  }
  if (binding.projectPath) {
    lines.push(`Folder: ${binding.projectPath}`);
  }
  if (binding.branch) {
    lines.push(`Branch: ${binding.branch}`);
  }
  if (binding.prHref) {
    lines.push(`Pull request: ${binding.prHref}`);
  }
  return lines.join("\n");
}

export function isChatWorkContextContent(content: string) {
  return content.startsWith("Work context");
}

export function shouldShowChatWorkContextContent(
  _content: string,
  _currentWorkContext: string | null,
) {
  // Work context rows are persisted chat history. A newer branch/PR binding
  // should add another row, not hide the older one and make the card look
  // pinned to the bottom.
  return true;
}

export function wasChatWorkContextSent(chatId: string, content: string) {
  if (typeof window === "undefined") {
    return false;
  }
  try {
    return window.localStorage.getItem(contextSentKey(chatId)) === content;
  } catch {
    return false;
  }
}

export function markChatWorkContextSent(chatId: string, content: string) {
  if (typeof window === "undefined") {
    return;
  }
  try {
    window.localStorage.setItem(contextSentKey(chatId), content);
  } catch {
    // Best effort.
  }
}
