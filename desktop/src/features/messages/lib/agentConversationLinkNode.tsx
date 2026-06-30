import { mergeAttributes, Node } from "@tiptap/core";
import {
  NodeViewWrapper,
  ReactNodeViewRenderer,
  type NodeViewProps,
} from "@tiptap/react";
import { ClipboardPlus } from "lucide-react";

import {
  AGENT_CONVERSATION_LINK_URL_PATTERN,
  isAgentConversationLink,
  trimAgentConversationLinkMatch,
} from "@/features/agents/agentConversationLink";
import { AGENT_CONVERSATION_LINK_NODE_NAME } from "./agentConversationLinkNodeName";

export type AgentConversationLinkNodeOptions = {
  titleForHref?: (href: string) => string | undefined;
};

function resolveTaskLinkFromElement(element: HTMLElement) {
  const dataHref = element.getAttribute("data-href");
  if (isAgentConversationLink(dataHref)) {
    return dataHref;
  }

  const href = element.getAttribute("href");
  if (isAgentConversationLink(href)) {
    return href;
  }

  const title = element.getAttribute("title");
  if (isAgentConversationLink(title)) {
    return title;
  }

  return null;
}

function getDisplayTitle(
  href: string,
  explicitTitle: string | null | undefined,
  options: AgentConversationLinkNodeOptions,
) {
  return (
    explicitTitle?.trim() || options.titleForHref?.(href)?.trim() || "Task"
  );
}

function ComposerAgentConversationLinkView({ extension, node }: NodeViewProps) {
  const href = String(node.attrs.href ?? "");
  const title = getDisplayTitle(
    href,
    String(node.attrs.title ?? ""),
    extension.options as AgentConversationLinkNodeOptions,
  );

  return (
    <NodeViewWrapper
      as="span"
      className="my-1 flex min-w-52 max-w-full cursor-default overflow-hidden rounded-lg border border-border/70 bg-muted/35 align-top text-left sm:max-w-xl"
      data-agent-conversation-link=""
      data-href={href}
      title={href}
    >
      <span className="flex min-w-0 flex-1 items-center gap-3 px-3 py-2">
        <span
          aria-hidden
          className="flex size-10 shrink-0 items-center justify-center rounded-lg bg-background p-2.5 text-muted-foreground shadow-xs ring-1 ring-border/60"
        >
          <ClipboardPlus className="size-5" />
        </span>
        <span className="min-w-0 flex-1">
          <span className="block truncate text-sm font-medium text-foreground">
            Task
          </span>
          <span className="mt-0.5 block truncate text-xs text-muted-foreground">
            {title}
          </span>
        </span>
        <span
          aria-hidden
          className="inline-flex h-8 shrink-0 items-center justify-center rounded-lg border border-input bg-background px-3 text-xs font-medium text-foreground shadow-xs"
        >
          Open
        </span>
      </span>
    </NodeViewWrapper>
  );
}

function registerAgentConversationLinkMarkdownIt(
  // biome-ignore lint/suspicious/noExplicitAny: markdown-it is untyped here
  md: any,
  options: AgentConversationLinkNodeOptions,
): void {
  const RULE_NAME = "buzz_agent_conversation_link";
  const TOKEN_TYPE = "buzz_agent_conversation_link";

  if (md.renderer.rules[TOKEN_TYPE]) return;

  // biome-ignore lint/suspicious/noExplicitAny: markdown-it state/silent
  const rule = (state: any, silent: boolean): boolean => {
    AGENT_CONVERSATION_LINK_URL_PATTERN.lastIndex = state.pos;
    const match = AGENT_CONVERSATION_LINK_URL_PATTERN.exec(state.src);
    if (!match || match.index !== state.pos) {
      return false;
    }

    const { value } = trimAgentConversationLinkMatch(match[0]);
    if (!isAgentConversationLink(value)) {
      return false;
    }

    if (!silent) {
      const token = state.push(TOKEN_TYPE, "span", 0);
      token.meta = {
        href: value,
        title: options.titleForHref?.(value) ?? "",
      };
    }
    state.pos += value.length;
    return true;
  };

  md.inline.ruler.before("text", RULE_NAME, rule);

  // biome-ignore lint/suspicious/noExplicitAny: markdown-it token
  md.renderer.rules[TOKEN_TYPE] = (tokens: any[], idx: number): string => {
    const { href, title } = tokens[idx].meta as {
      href: string;
      title: string;
    };
    const esc = md.utils.escapeHtml;
    return `<span data-agent-conversation-link data-href="${esc(href)}" data-title="${esc(title)}"></span>`;
  };
}

export const AgentConversationLinkNode =
  Node.create<AgentConversationLinkNodeOptions>({
    name: AGENT_CONVERSATION_LINK_NODE_NAME,

    group: "inline",
    inline: true,
    atom: true,
    selectable: true,

    addOptions() {
      return {
        titleForHref: undefined,
      };
    },

    addAttributes() {
      return {
        href: {
          default: "",
          parseHTML: (element) =>
            resolveTaskLinkFromElement(element as HTMLElement) ?? "",
          renderHTML: (attrs) => ({ "data-href": attrs.href }),
        },
        title: {
          default: "",
          parseHTML: (element) =>
            (element as HTMLElement).getAttribute("data-title") ?? "",
          renderHTML: (attrs) => ({ "data-title": attrs.title }),
        },
      };
    },

    parseHTML() {
      return [{ tag: "[data-agent-conversation-link]" }];
    },

    renderHTML({ HTMLAttributes, node }) {
      const href = String(node.attrs.href ?? "");
      const title = getDisplayTitle(
        href,
        String(node.attrs.title ?? ""),
        this.options,
      );

      return [
        "span",
        mergeAttributes(HTMLAttributes, {
          "data-agent-conversation-link": "",
          "data-href": href,
          "data-title": title,
          title: href,
        }),
        [
          "span",
          {
            class:
              "my-1 flex min-w-52 max-w-full overflow-hidden rounded-lg border border-border/70 bg-muted/35 align-top sm:max-w-xl",
          },
          [
            "span",
            {
              class: "flex min-w-0 flex-1 items-center gap-3 px-3 py-2",
            },
            ["span", {}, "Task"],
            ["span", {}, title],
          ],
        ],
      ];
    },

    renderText({ node }) {
      return String(node.attrs.href ?? "");
    },

    addNodeView() {
      return ReactNodeViewRenderer(ComposerAgentConversationLinkView);
    },

    addStorage() {
      return {
        markdown: {
          serialize(
            // biome-ignore lint/suspicious/noExplicitAny: prosemirror-markdown state is untyped
            state: any,
            // biome-ignore lint/suspicious/noExplicitAny: PM node
            node: any,
          ) {
            state.write(String(node.attrs.href ?? ""));
          },
          parse: {
            setup(
              this: { options: AgentConversationLinkNodeOptions },
              // biome-ignore lint/suspicious/noExplicitAny: markdown-it is untyped here
              md: any,
            ) {
              registerAgentConversationLinkMarkdownIt(md, this.options);
            },
          },
        },
      };
    },
  });
