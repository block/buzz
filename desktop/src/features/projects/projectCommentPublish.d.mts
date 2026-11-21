export function projectCommentKindAndChannelTags(
  project: {
    owner: string;
    projectChannelId: string | null;
    repoAddress: string;
  },
  mentionPubkeys: string[],
): { kind: number; channelTags: string[][] };
