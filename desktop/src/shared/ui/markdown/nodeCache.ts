import type * as React from "react";
import ReactMarkdown, { type Components } from "react-markdown";
import remarkBreaks from "remark-breaks";
import remarkGfm from "remark-gfm";

import remarkMessageLinks from "@/features/messages/lib/remarkMessageLinks";
import rehypeImageGallery from "@/shared/lib/rehypeImageGallery";
import rehypeSearchHighlight from "@/shared/lib/rehypeSearchHighlight";
import remarkChannelLinks from "@/shared/lib/remarkChannelLinks";
import remarkCustomEmoji, {
  type CustomEmoji,
} from "@/shared/lib/remarkCustomEmoji";
import remarkMentions from "@/shared/lib/remarkMentions";
import remarkSpoilers from "@/shared/lib/remarkSpoilers";

import { messageLinkUrlTransform } from "./utils";

/**
 * Parsed-markdown element cache.
 *
 * The message timeline's scroll container is keyed by channel id (see
 * MessageTimeline — required so TanStack Router's scroll restoration never
 * writes a stale scrollTop into a reused scroll node), so every channel
 * switch remounts every row and `React.memo` cannot carry the react-markdown
 * parse across the remount. react-markdown's `Markdown` is a plain
 * synchronous hook-free function, so its element tree is a pure function of
 * the parse inputs below and can be reused across mounts. Everything
 * per-mount (channels, imeta lookup, navigation callbacks) flows through
 * `MarkdownRuntimeContext`, read at render time — a cached element never
 * captures per-mount state. The `components` map passed in must be
 * module-stable for the same reason (see `getMarkdownComponents`).
 *
 * Recency-ordered via Map insertion order; capacity comfortably covers two
 * window-ceiling channels' worth of rows.
 */
const MARKDOWN_NODE_CACHE_LIMIT = 1000;
const markdownNodeCache = new Map<string, React.ReactElement>();

/** Workspace switches swap relays; drop parses keyed against the old
 * workspace's mention/channel-name space (see `resetWorkspaceState`). */
export function clearMarkdownNodeCache() {
  markdownNodeCache.clear();
}

export type MarkdownParseInputs = {
  channelNames?: string[];
  components: Components;
  content: string;
  customEmoji?: CustomEmoji[];
  interactive: boolean;
  mediaInset: boolean;
  mentionNames?: string[];
  searchQuery?: string;
};

function buildMarkdownElement(input: MarkdownParseInputs): React.ReactElement {
  // biome-ignore lint/suspicious/noExplicitAny: PluggableList type not directly importable
  const rehypePlugins: any[] = [rehypeImageGallery];
  if (input.searchQuery && input.searchQuery.trim().length >= 2) {
    rehypePlugins.push([rehypeSearchHighlight, { query: input.searchQuery }]);
  }
  // Called as a plain function rather than rendered as <ReactMarkdown/>:
  // react-markdown's `Markdown` is synchronous and hook-free (the hook
  // variant is `MarkdownHooks`), so this returns the parsed element tree
  // directly, which is what lets it live in a module-level cache.
  return ReactMarkdown({
    children: input.content,
    components: input.components,
    remarkPlugins: [
      remarkGfm,
      remarkBreaks,
      remarkSpoilers,
      remarkMessageLinks,
      [remarkMentions, { mentionNames: input.mentionNames }],
      [remarkChannelLinks, { channelNames: input.channelNames }],
      [remarkCustomEmoji, { customEmoji: input.customEmoji }],
      // biome-ignore lint/suspicious/noExplicitAny: PluggableList type not directly importable
    ] as any[],
    rehypePlugins,
    urlTransform: messageLinkUrlTransform,
  });
}

export function renderCachedMarkdown(
  input: MarkdownParseInputs,
): React.ReactElement {
  // Search highlighting is transient and query-specific: parse fresh rather
  // than churn the cache with per-query variants.
  if (input.searchQuery && input.searchQuery.trim().length >= 2) {
    return buildMarkdownElement(input);
  }
  // Everything that changes the parse output must be in the key. Arrays are
  // keyed by value, not identity — callers rebuild them across mounts.
  // Control-char separators keep adjacent segments from colliding.
  const key = [
    input.interactive ? "i" : "",
    input.mediaInset ? "m" : "",
    input.mentionNames?.join("\u0001") ?? "",
    input.channelNames?.join("\u0001") ?? "",
    input.customEmoji
      ?.map((emoji) => `${emoji.shortcode}\u0002${emoji.url}`)
      .join("\u0001") ?? "",
    input.content,
  ].join("\u0000");

  const hit = markdownNodeCache.get(key);
  if (hit) {
    markdownNodeCache.delete(key);
    markdownNodeCache.set(key, hit);
    return hit;
  }
  const element = buildMarkdownElement(input);
  markdownNodeCache.set(key, element);
  if (markdownNodeCache.size > MARKDOWN_NODE_CACHE_LIMIT) {
    const oldest = markdownNodeCache.keys().next().value;
    if (oldest !== undefined) {
      markdownNodeCache.delete(oldest);
    }
  }
  return element;
}
