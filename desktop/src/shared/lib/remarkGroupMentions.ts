import { createRemarkPrefixPlugin } from "./createRemarkPrefixPlugin";
import { buildMentionPattern } from "./mentionPattern";

type RemarkGroupMentionsOptions = {
  groupHandles?: string[];
};

export default function remarkGroupMentions(
  options?: RemarkGroupMentionsOptions,
) {
  const pattern = buildMentionPattern(options?.groupHandles ?? []);
  return createRemarkPrefixPlugin(pattern, (matchText) => ({
    type: "groupMention",
    value: matchText,
    data: {
      hName: "group-mention",
      hChildren: [{ type: "text", value: matchText }],
    },
  }));
}
