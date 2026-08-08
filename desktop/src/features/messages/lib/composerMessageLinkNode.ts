import { mergeAttributes, Node } from "@tiptap/core";
import type { Node as ProseMirrorNode } from "@tiptap/pm/model";
import { TextSelection } from "@tiptap/pm/state";
import type { EditorView } from "@tiptap/pm/view";

import { MENTION_CHIP_BASE_CLASSES } from "@/shared/ui/mentionChip";
import {
  getMessageLinkChannelLabel,
  getMessageLinkLabel,
  MESSAGE_LINK_PREFIX,
} from "./messageLinkLabel";
import { buildMessageLink, parseMessageLink } from "./messageLink";

export const COMPOSER_MESSAGE_LINK_NODE_NAME = "composerMessageLink";

export type ComposerMessageLinkNodeOptions = {
  resolveChannelName: (channelId: string) => string | undefined;
};

export type ComposerMessageLinkAttributes = {
  channelName: string;
  href: string;
};

const BARE_MESSAGE_LINK_AT_START = /^(?:buzz):\/\/message\?[^\s<>"')\]}*_]+/i;
const TRAILING_PUNCTUATION = /[.,;:!?]+$/;

function trimBareMessageLink(value: string): string {
  let trimmed = value.replace(TRAILING_PUNCTUATION, "");
  while (/[)\]]$/.test(trimmed)) {
    const closing = trimmed.at(-1) ?? "";
    const opening = closing === ")" ? "(" : "[";
    if (trimmed.split(closing).length <= trimmed.split(opening).length) break;
    trimmed = trimmed.slice(0, -1).replace(TRAILING_PUNCTUATION, "");
  }
  return trimmed;
}

export function resolveComposerMessageLinkAttributes(
  href: string,
  resolveChannelName: ComposerMessageLinkNodeOptions["resolveChannelName"],
): ComposerMessageLinkAttributes | null {
  const parsed = parseMessageLink(href);
  if (!parsed.ok) return null;
  return {
    channelName: resolveChannelName(parsed.value.channelId) ?? "",
    href: buildMessageLink({
      channelId: parsed.value.channelId,
      messageId: parsed.value.messageId,
      threadRootId: parsed.value.threadRootId,
    }),
  };
}

function unwrapExactMessageLink(text: string): string | null {
  const href =
    text.startsWith("<") && text.endsWith(">") ? text.slice(1, -1) : text;
  if (!href || /\s/.test(href)) return null;
  return parseMessageLink(href).ok ? href : null;
}

function unwrapExactHttpLink(text: string): string | null {
  const match = /^(?:<(https?:\/\/[^\s<>]+)>|(https?:\/\/\S+))$/i.exec(text);
  return match?.[1] ?? match?.[2] ?? null;
}

function replaceSelectionWithNode(view: EditorView, node: ProseMirrorNode) {
  const { from, to } = view.state.selection;
  let transaction = view.state.tr.replaceRangeWith(from, to, node);
  const end = transaction.mapping.map(to);
  transaction = transaction.insertText(" ", end);
  const linkMark = view.state.schema.marks.link;
  if (linkMark) transaction = transaction.removeMark(end, end + 1, linkMark);
  transaction = transaction.setSelection(
    TextSelection.create(transaction.doc, end + 1),
  );
  view.dispatch(transaction.setStoredMarks([]).scrollIntoView());
  view.focus();
}

export function createComposerLinkPasteHandler(
  resolveChannelName: ComposerMessageLinkNodeOptions["resolveChannelName"],
) {
  return (view: EditorView, event: ClipboardEvent): boolean => {
    const text = event.clipboardData?.getData("text/plain") ?? "";
    const messageHref = unwrapExactMessageLink(text);
    const messageLinkType =
      view.state.schema.nodes[COMPOSER_MESSAGE_LINK_NODE_NAME];
    if (messageHref && messageLinkType) {
      const attrs = resolveComposerMessageLinkAttributes(
        messageHref,
        resolveChannelName,
      );
      if (attrs) {
        replaceSelectionWithNode(view, messageLinkType.create(attrs));
        event.preventDefault();
        return true;
      }
    }

    const httpHref = unwrapExactHttpLink(text);
    const linkMark = view.state.schema.marks.link;
    if (!httpHref || !linkMark) return false;
    replaceSelectionWithNode(
      view,
      view.state.schema.text(httpHref, [linkMark.create({ href: httpHref })]),
    );
    event.preventDefault();
    return true;
  };
}

export function registerComposerMessageLinkMarkdownIt(
  // biome-ignore lint/suspicious/noExplicitAny: markdown-it is untyped here
  md: any,
  options: ComposerMessageLinkNodeOptions,
): void {
  const ruleName = "buzz_composer_message_link";
  const tokenType = "buzz_composer_message_link";
  if (md.renderer.rules[tokenType]) return;

  // biome-ignore lint/suspicious/noExplicitAny: markdown-it state/silent
  const rule = (state: any, silent: boolean): boolean => {
    const remaining = state.src.slice(state.pos);
    const fullMatch = BARE_MESSAGE_LINK_AT_START.exec(remaining);
    const suffixMatch = /^:\/\/message\?[^\s<>"')\]}*_]+/i.exec(remaining);
    const resumesTextToken =
      !fullMatch && suffixMatch && /buzz$/i.test(state.pending ?? "");
    const rawHref =
      fullMatch?.[0] ?? (resumesTextToken ? `buzz${suffixMatch[0]}` : null);
    if (!rawHref) return false;
    const href = trimBareMessageLink(rawHref);
    const attrs = resolveComposerMessageLinkAttributes(
      href,
      options.resolveChannelName,
    );
    if (!attrs) return false;
    if (!silent) {
      if (resumesTextToken) state.pending = state.pending.slice(0, -4);
      const token = state.push(tokenType, "span", 0);
      token.meta = attrs;
    }
    state.pos += href.length - (resumesTextToken ? 4 : 0);
    return true;
  };

  md.inline.ruler.before("text", ruleName, rule);
  // biome-ignore lint/suspicious/noExplicitAny: markdown-it token
  md.renderer.rules[tokenType] = (tokens: any[], index: number): string => {
    const attrs = tokens[index].meta as ComposerMessageLinkAttributes;
    const escapeHtml = md.utils.escapeHtml;
    return `<span data-composer-message-link="" data-channel-name="${escapeHtml(attrs.channelName)}" data-href="${escapeHtml(attrs.href)}"></span>`;
  };
}

export const ComposerMessageLinkNode =
  Node.create<ComposerMessageLinkNodeOptions>({
    name: COMPOSER_MESSAGE_LINK_NODE_NAME,
    group: "inline",
    inline: true,
    atom: true,
    selectable: true,

    addOptions() {
      return { resolveChannelName: () => undefined };
    },

    addAttributes() {
      return {
        channelName: {
          default: "",
          parseHTML: (element) =>
            (element as HTMLElement).getAttribute("data-channel-name") ?? "",
          renderHTML: () => ({}),
        },
        href: {
          default: "",
          parseHTML: (element) =>
            (element as HTMLElement).getAttribute("data-href") ?? "",
          renderHTML: () => ({}),
        },
      };
    },

    parseHTML() {
      return [{ tag: "span[data-composer-message-link]" }];
    },

    renderHTML({ node, HTMLAttributes }) {
      const href = String(node.attrs.href ?? "");
      const parsed = parseMessageLink(href);
      const channelName = parsed.ok
        ? (this.options.resolveChannelName(parsed.value.channelId) ??
          (String(node.attrs.channelName ?? "") || "channel"))
        : "channel";
      const label = getMessageLinkLabel({ channelName });
      const channelLinkLabel = getMessageLinkChannelLabel(channelName);
      return [
        "span",
        mergeAttributes(HTMLAttributes, {
          "aria-label": label,
          class:
            "inline-flex min-w-0 max-w-80 items-center gap-1.5 align-baseline",
          "data-channel-name": channelName,
          "data-composer-message-link": "",
          "data-href": href,
          "data-message-link": "",
          title: label,
        }),
        ["span", { class: "shrink-0" }, MESSAGE_LINK_PREFIX],
        [
          "span",
          {
            class: `${MENTION_CHIP_BASE_CLASSES} min-w-0 max-w-full truncate`,
            "data-channel-link": "",
          },
          channelLinkLabel,
        ],
      ];
    },

    renderText({ node }) {
      return String(node.attrs.href ?? "");
    },

    addStorage() {
      return {
        markdown: {
          // biome-ignore lint/suspicious/noExplicitAny: prosemirror-markdown is untyped here
          serialize(state: any, node: any) {
            state.write(String(node.attrs.href ?? ""));
          },
          parse: {
            // biome-ignore lint/suspicious/noExplicitAny: markdown-it is untyped here
            setup(this: { options: ComposerMessageLinkNodeOptions }, md: any) {
              registerComposerMessageLinkMarkdownIt(md, this.options);
            },
          },
        },
      };
    },
  });
