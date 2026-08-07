import * as React from "react";

import {
  extractMessageEmbedLinks,
  type MessageEmbedLink,
} from "@/features/messages/lib/messageEmbed";
import type { ParsedMessageLink } from "@/features/messages/lib/messageLink";
import type { Channel } from "@/shared/api/types";
import { AttachmentGroup } from "@/shared/ui/attachment";

import { MessageEmbed } from "./MessageEmbed";

export function MessageEmbedList({
  channels,
  content,
  interactive,
  onOpenMessageLink,
}: {
  channels: Channel[];
  content: string;
  interactive: boolean;
  onOpenMessageLink: (link: ParsedMessageLink) => void;
}) {
  const links = React.useMemo(
    () => (interactive ? extractMessageEmbedLinks(content) : []),
    [content, interactive],
  );
  if (links.length === 0) return null;

  return (
    <AttachmentGroup
      className="w-full max-w-2xl flex-col items-stretch overflow-visible pb-0"
      data-message-embed-list=""
    >
      {links.map((link: MessageEmbedLink) => (
        <MessageEmbed
          channel={channels.find((channel) => channel.id === link.channelId)}
          key={link.href}
          link={link}
          onOpen={() => onOpenMessageLink(link)}
        />
      ))}
    </AttachmentGroup>
  );
}
