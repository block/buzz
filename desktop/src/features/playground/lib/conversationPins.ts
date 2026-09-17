import type { PlaygroundCard } from "./types";

export type ConversationPlaygroundPin = {
  sid: string;
  name: string;
  url: string;
  pin?: string;
  stack?: string;
  expires?: string;
};

type PinsByScope = Map<string, Map<string, ConversationPlaygroundPin>>;

const pinsByScope: PinsByScope = new Map();
const listeners = new Set<() => void>();

type MenuOpenRequest = { scopeKey: string; nonce: number };
let menuOpenRequest: MenuOpenRequest | null = null;
let menuOpenNonce = 0;
const menuOpenListeners = new Set<() => void>();

let cachedPinsSnapshot: ConversationPlaygroundPin[] = [];
let cachedPinsScope: string | null = null;

function emitPins() {
  cachedPinsScope = null;
  for (const listener of listeners) listener();
}

function emitMenuOpen() {
  for (const listener of menuOpenListeners) listener();
}

function cardToPin(card: PlaygroundCard): ConversationPlaygroundPin {
  return {
    sid: card.sid,
    name: card.name,
    url: card.url,
    ...(card.pin ? { pin: card.pin } : {}),
    ...(card.stack ? { stack: card.stack } : {}),
    ...(card.expires != null ? { expires: String(card.expires) } : {}),
  };
}

/** Native pin-webview id kept alive for a conversation playground pin. */
export function conversationPlaygroundPinWebviewId(sid: string): string {
  return `playground-pin:${sid}`;
}

export function subscribeConversationPlaygroundPins(
  listener: () => void,
): () => void {
  listeners.add(listener);
  return () => {
    listeners.delete(listener);
  };
}

export function listConversationPlaygroundPins(
  scopeKey: string,
): ConversationPlaygroundPin[] {
  if (cachedPinsScope === scopeKey) return cachedPinsSnapshot;
  const scope = pinsByScope.get(scopeKey);
  cachedPinsSnapshot = scope ? [...scope.values()] : [];
  cachedPinsScope = scopeKey;
  return cachedPinsSnapshot;
}

export function hasConversationPlaygroundPin(
  scopeKey: string,
  sid: string,
): boolean {
  return pinsByScope.get(scopeKey)?.has(sid) ?? false;
}

/**
 * Add (or refresh) a named playground pin under a channel/thread scope.
 * Client-lifetime only — never persisted. Does not open a slide-out or overlay.
 */
export function pinPlaygroundToConversation(
  scopeKey: string,
  card: PlaygroundCard,
): ConversationPlaygroundPin {
  const trimmed = scopeKey.trim();
  if (!trimmed) {
    throw new Error("Missing playground pin scope.");
  }
  let scope = pinsByScope.get(trimmed);
  if (!scope) {
    scope = new Map();
    pinsByScope.set(trimmed, scope);
  }
  const pin = cardToPin(card);
  scope.set(pin.sid, pin);
  emitPins();
  return pin;
}

export function unpinPlaygroundFromConversation(
  scopeKey: string,
  sid: string,
): ConversationPlaygroundPin | null {
  const scope = pinsByScope.get(scopeKey);
  if (!scope) return null;
  const existing = scope.get(sid) ?? null;
  if (!existing) return null;
  scope.delete(sid);
  if (scope.size === 0) pinsByScope.delete(scopeKey);
  emitPins();
  return existing;
}

export function requestOpenConversationPlaygroundPinsMenu(
  scopeKey: string,
): void {
  menuOpenNonce += 1;
  menuOpenRequest = { scopeKey, nonce: menuOpenNonce };
  emitMenuOpen();
}

export function subscribeConversationPlaygroundPinsMenu(
  listener: () => void,
): () => void {
  menuOpenListeners.add(listener);
  return () => {
    menuOpenListeners.delete(listener);
  };
}

export function getConversationPlaygroundPinsMenuOpenRequest(): MenuOpenRequest | null {
  return menuOpenRequest;
}

/** Test helper — wipe in-memory pins and menu-open requests. */
export function resetConversationPlaygroundPins(): void {
  pinsByScope.clear();
  cachedPinsSnapshot = [];
  cachedPinsScope = null;
  menuOpenRequest = null;
  emitPins();
  emitMenuOpen();
}
