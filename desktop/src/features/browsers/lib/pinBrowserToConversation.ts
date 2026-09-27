import { parseGrantConversationLink } from "@/features/browser-agent/lib/parseGrantConversationLink";
import {
  playgroundConversationFromPopout,
  playgroundPinScopeKey,
} from "@/features/playground/lib/conversation";
import {
  listConversationPinBindingsForSid,
  pinPlaygroundToConversation,
  type ConversationPinBinding,
  type ConversationPlaygroundPin,
} from "@/features/playground/lib/conversationPins";
import {
  PLAYGROUND_HULA,
  PLAYGROUND_VERSION,
  type PlaygroundCard,
} from "@/features/playground/lib/types";

import {
  normalizeBrowserSessionName,
  normalizeBrowserSessionUrl,
} from "./addBrowserSession";

export type BrowserRowPinTarget = {
  scopeKey: string;
  channelId: string;
  threadRoot: string | null;
};

/** Resolve channel/thread ids into a conversation pin scope. */
export function resolveBrowserRowPinTarget(input: {
  channelId: string;
  threadRoot?: string | null;
}): BrowserRowPinTarget | null {
  const conversation = playgroundConversationFromPopout({
    channelId: input.channelId,
    threadId: input.threadRoot,
  });
  if (!conversation) return null;
  const threadRoot = conversation.draftKey.startsWith("thread:")
    ? conversation.draftKey.slice("thread:".length) || null
    : null;
  return {
    scopeKey: playgroundPinScopeKey(conversation),
    channelId: conversation.channelId,
    threadRoot,
  };
}

/**
 * Build a playground card for pinning an existing browser row.
 * Keeps the row sid so BindingChips stay linked; URL/name may be edited.
 */
export function buildBrowserRowPinCard(input: {
  sid: string;
  title: string;
  url: string;
  name?: string;
  pin?: string;
  stack?: string;
  expires?: string | number;
}): PlaygroundCard | null {
  const sid = input.sid.trim();
  if (!sid) return null;
  const url = normalizeBrowserSessionUrl(input.url);
  if (!url) return null;
  return {
    hula: PLAYGROUND_HULA,
    v: PLAYGROUND_VERSION,
    name: normalizeBrowserSessionName(input.name ?? input.title, url),
    url,
    sid,
    ...(input.pin ? { pin: input.pin } : {}),
    ...(input.stack ? { stack: input.stack } : {}),
    ...(input.expires != null ? { expires: input.expires } : {}),
  };
}

/**
 * Parse a pasted Buzz channel/thread link into a pin target.
 * Returns null when the paste is empty or not a recognized link.
 */
export function parseBrowserRowPinLink(
  raw: string,
): BrowserRowPinTarget | null {
  const parsed = parseGrantConversationLink(raw);
  if (!parsed) return null;
  return resolveBrowserRowPinTarget({
    channelId: parsed.channelId,
    threadRoot: parsed.threadRoot,
  });
}

/**
 * Pin an existing browser session to a channel or thread.
 * Reuses conversationPins — same store as playground-card Pin.
 */
export function pinBrowserRowToConversation(input: {
  sid: string;
  title: string;
  url: string;
  name?: string;
  channelId: string;
  threadRoot?: string | null;
  pin?: string;
  stack?: string;
  expires?: string | number;
}): ConversationPlaygroundPin | null {
  const target = resolveBrowserRowPinTarget({
    channelId: input.channelId,
    threadRoot: input.threadRoot,
  });
  if (!target) return null;
  const card = buildBrowserRowPinCard(input);
  if (!card) return null;
  return pinPlaygroundToConversation(target.scopeKey, card, target.channelId);
}

/**
 * Collect pin bindings for every tab sid on a Browsers list row
 * (main + active + secondary), deduped by scopeKey.
 */
export function listBrowserRowPinBindings(row: {
  mainSurfaceId: string;
  surfaceId: string;
  secondaryTabs?: ReadonlyArray<{ surfaceId: string }>;
}): ConversationPinBinding[] {
  const sids = new Set<string>();
  const main = row.mainSurfaceId.trim();
  const active = row.surfaceId.trim();
  if (main) sids.add(main);
  if (active) sids.add(active);
  for (const tab of row.secondaryTabs ?? []) {
    const sid = tab.surfaceId.trim();
    if (sid) sids.add(sid);
  }
  const out: ConversationPinBinding[] = [];
  const seen = new Set<string>();
  for (const sid of sids) {
    for (const binding of listConversationPinBindingsForSid(sid)) {
      if (seen.has(binding.scopeKey)) continue;
      seen.add(binding.scopeKey);
      out.push(binding);
    }
  }
  return out;
}
