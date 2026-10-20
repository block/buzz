import type { Editor } from "@tiptap/react";
import * as React from "react";

import {
  COMPOSER_MESSAGE_LINK_NODE_NAME,
  ComposerMessageLinkNode,
} from "./composerMessageLinkNode";
import { parseMessageLink } from "./messageLink";

export type ComposerMessageLinkChannel = { id: string; name: string };

export function useComposerMessageLinks(
  channels: readonly ComposerMessageLinkChannel[] | undefined,
) {
  const channelsRef = React.useRef(channels ?? []);
  channelsRef.current = channels ?? [];

  const resolveChannelName = React.useCallback(
    (channelId: string) =>
      channelsRef.current.find((channel) => channel.id === channelId)?.name,
    [],
  );

  const extension = React.useMemo(
    () => ComposerMessageLinkNode.configure({ resolveChannelName }),
    [resolveChannelName],
  );

  const syncChannelNames = React.useCallback(
    (editor: Editor) => {
      const channelNamesById = new Map(
        (channels ?? []).map((channel) => [channel.id, channel.name]),
      );
      let transaction = editor.state.tr;
      let changed = false;
      editor.state.doc.descendants((node, position) => {
        if (node.type.name !== COMPOSER_MESSAGE_LINK_NODE_NAME) return;
        const parsed = parseMessageLink(String(node.attrs.href ?? ""));
        const nextName = parsed.ok
          ? (channelNamesById.get(parsed.value.channelId) ?? "")
          : "";
        if (nextName !== node.attrs.channelName) {
          transaction = transaction.setNodeAttribute(
            position,
            "channelName",
            nextName,
          );
          changed = true;
        }
      });
      if (changed) {
        editor.view.dispatch(transaction.setMeta("addToHistory", false));
      }
    },
    [channels],
  );

  return { extension, resolveChannelName, syncChannelNames };
}
