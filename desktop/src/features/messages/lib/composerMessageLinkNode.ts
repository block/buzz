import { mergeAttributes, Node } from "@tiptap/core";
import type { Node as ProseMirrorNode } from "@tiptap/pm/model";
import { TextSelection } from "@tiptap/pm/state";
import type { EditorView } from "@tiptap/pm/view";

import {
  buildIssueLink,
  buildPullRequestLink,
  buildRepoLink,
  parseEntityLink,
} from "@/shared/lib/entityLink";
import {
  inlineChipIconClasses,
  type InlineChipIconKind,
  MENTION_CHIP_BASE_CLASSES,
} from "@/shared/ui/mentionChip";
import { buildChannelLink, parseChannelLink } from "./channelLink";
import { getMessageLinkLabel } from "./messageLinkLabel";
import { buildMessageLink, parseMessageLink } from "./messageLink";

export const COMPOSER_MESSAGE_LINK_NODE_NAME = "composerMessageLink";

export type ComposerMessageLinkNodeOptions = {
  resolveChannelName: (channelId: string) => string | undefined;
};

export type ComposerMessageLinkAttributes = {
  channelName: string;
  href: string;
};

const BARE_BUZZ_LINK_AT_START =
  /^buzz:\/\/(?:message\?|channel\/|(?:pr|issue|repo)\?)[^\s<>"')\]}*]+/i;
const BUZZ_LINK_SUFFIX_AT_START =
  /^:\/\/(?:message\?|channel\/|(?:pr|issue|repo)\?)[^\s<>"')\]}*]+/i;
const TRAILING_PUNCTUATION = /[.,;:!?]+$/;

function trimBareBuzzLink(value: string): string {
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
  const message = parseMessageLink(href);
  if (message.ok) {
    return {
      channelName: resolveChannelName(message.value.channelId) ?? "",
      href: buildMessageLink({
        channelId: message.value.channelId,
        messageId: message.value.messageId,
        threadRootId: message.value.threadRootId,
      }),
    };
  }

  const channel = parseChannelLink(href);
  if (channel.ok) {
    return {
      channelName: resolveChannelName(channel.value.channelId) ?? "",
      href: buildChannelLink(channel.value.channelId),
    };
  }

  const entity = parseEntityLink(href);
  if (!entity.ok) return null;
  switch (entity.value.type) {
    case "repo":
      return {
        channelName: "",
        href: buildRepoLink(entity.value),
      };
    case "pr":
      return {
        channelName: "",
        href: buildPullRequestLink(entity.value),
      };
    case "issue":
      return {
        channelName: "",
        href: buildIssueLink(entity.value),
      };
  }
}

function unwrapExactBuzzLink(text: string): string | null {
  const href =
    text.startsWith("<") && text.endsWith(">") ? text.slice(1, -1) : text;
  if (!href || /\s/.test(href)) return null;
  return parseMessageLink(href).ok ||
    parseChannelLink(href).ok ||
    parseEntityLink(href).ok
    ? href
    : null;
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
    const buzzHref = unwrapExactBuzzLink(text);
    const buzzLinkType =
      view.state.schema.nodes[COMPOSER_MESSAGE_LINK_NODE_NAME];
    if (buzzHref && buzzLinkType) {
      const attrs = resolveComposerMessageLinkAttributes(
        buzzHref,
        resolveChannelName,
      );
      if (attrs) {
        replaceSelectionWithNode(view, buzzLinkType.create(attrs));
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
    const fullMatch = BARE_BUZZ_LINK_AT_START.exec(remaining);
    const suffixMatch = BUZZ_LINK_SUFFIX_AT_START.exec(remaining);
    const resumesTextToken =
      !fullMatch && suffixMatch && /buzz$/i.test(state.pending ?? "");
    const rawHref =
      fullMatch?.[0] ?? (resumesTextToken ? `buzz${suffixMatch[0]}` : null);
    if (!rawHref) return false;
    const href = trimBareBuzzLink(rawHref);
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
    return `<span data-composer-buzz-link="" data-channel-name="${escapeHtml(attrs.channelName)}" data-href="${escapeHtml(attrs.href)}"></span>`;
  };
}

type ComposerLinkPresentation = {
  ariaLabel: string;
  channelName: string;
  dataAttributes: Record<string, string>;
  icon: InlineChipIconKind;
  label: string;
};

function composerLinkPresentation(
  href: string,
  channelName: string,
  resolveChannelName: ComposerMessageLinkNodeOptions["resolveChannelName"],
): ComposerLinkPresentation {
  const message = parseMessageLink(href);
  if (message.ok) {
    const resolvedChannelName =
      resolveChannelName(message.value.channelId) || channelName || "channel";
    return {
      ariaLabel: getMessageLinkLabel({ channelName: resolvedChannelName }),
      channelName: resolvedChannelName,
      dataAttributes: {
        "data-composer-message-link": "",
        "data-message-link": "",
      },
      icon: "message",
      label: `${resolvedChannelName} · ${message.value.messageId.slice(0, 8)}`,
    };
  }

  const channel = parseChannelLink(href);
  if (channel.ok) {
    const resolvedChannelName =
      resolveChannelName(channel.value.channelId) ||
      channelName ||
      channel.value.channelId.slice(0, 8);
    return {
      ariaLabel: `Open channel ${resolvedChannelName}`,
      channelName: resolvedChannelName,
      dataAttributes: { "data-channel-deep-link": "" },
      icon: "channel",
      label: resolvedChannelName,
    };
  }

  const entity = parseEntityLink(href);
  if (!entity.ok) {
    return {
      ariaLabel: "Buzz link",
      channelName: "",
      dataAttributes: {},
      icon: "message",
      label: "Buzz link",
    };
  }

  const shortId =
    entity.value.type === "repo" ? "" : entity.value.id.slice(0, 8);
  return {
    ariaLabel:
      entity.value.type === "repo"
        ? `Open repository ${entity.value.dtag}`
        : `Open ${entity.value.type === "pr" ? "pull request" : "issue"} ${shortId} in repository ${entity.value.dtag}`,
    channelName: "",
    dataAttributes: { "data-buzz-link-kind": entity.value.type },
    icon: entity.value.type,
    label:
      entity.value.type === "repo"
        ? entity.value.dtag
        : `${entity.value.dtag} · ${shortId}`,
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
      return [
        { tag: "span[data-composer-buzz-link]" },
        { tag: "span[data-composer-message-link]" },
      ];
    },

    renderHTML({ node, HTMLAttributes }) {
      const href = String(node.attrs.href ?? "");
      const presentation = composerLinkPresentation(
        href,
        String(node.attrs.channelName ?? ""),
        this.options.resolveChannelName,
      );
      return [
        "span",
        mergeAttributes(HTMLAttributes, {
          "aria-label": presentation.ariaLabel,
          class: `${MENTION_CHIP_BASE_CLASSES} ${inlineChipIconClasses(presentation.icon)} cursor-text`,
          "data-buzz-link": "",
          "data-channel-name": presentation.channelName,
          "data-composer-buzz-link": "",
          "data-href": href,
          ...presentation.dataAttributes,
          title: presentation.ariaLabel,
        }),
        presentation.label,
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
