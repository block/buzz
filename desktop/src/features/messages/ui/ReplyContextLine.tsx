import { useAppNavigation } from "@/app/navigation/useAppNavigation";
import {
  getReplyContextId,
  getThreadReference,
} from "@/features/messages/lib/threading";

/** Retain a response's target without creating another thread level. */
export function ReplyContextLine({
  channelId,
  tags,
}: {
  channelId?: string | null;
  tags?: string[][];
}) {
  const { goChannel } = useAppNavigation();
  const contextId = getReplyContextId(tags ?? []);
  if (!channelId || !contextId) return null;
  const rootId = getThreadReference(tags ?? []).rootId;
  return (
    <button
      type="button"
      className="mb-1 text-sm text-muted-foreground hover:underline focus-visible:underline"
      data-testid="reply-context"
      onClick={() => {
        void goChannel(channelId, {
          messageId: contextId,
          threadRootId: rootId,
        });
      }}
    >
      Replying to message
    </button>
  );
}
